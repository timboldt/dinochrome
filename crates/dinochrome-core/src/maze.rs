//! Procedural maze generation.
//!
//! Generation runs in two passes. First a **recursive backtracker** carves a
//! perfect maze — exactly one route between any two points — on the odd cells of
//! the grid, which is what makes the border ring and the wall between every pair
//! of corridors fall out for free. Then a **thinning** pass removes walls at
//! random until the requested [`MazeParams::density`] is met.
//!
//! The thinning pass is not cosmetic. A perfect maze is a bad arena: dead ends
//! are frustrating to drive out of, and a pathfinding enemy in a loop-free maze
//! is trivially avoidable because there is only ever one direction it can come
//! from. Loops are what make the maze a place to fight in.
//!
//! # The connectivity invariant
//!
//! Every open cell must be reachable from every other, or factories can be
//! placed where the player cannot get to them. The carve establishes that, and
//! thinning preserves it by construction rather than by checking afterwards: a
//! wall is only opened if it already has an open orthogonal neighbour, so the
//! cell it becomes necessarily joins the one existing component. `is_connected`
//! is asserted in tests over many seeds, but no generated maze depends on the
//! check passing.

use glam::IVec2;
use rand::seq::SliceRandom;
use rand::{Rng, RngExt, SeedableRng, rngs::Xoshiro256PlusPlus};

use crate::grid::{Cell, Grid, ORTHOGONAL};

/// Smallest maze the carve can produce: a border ring around a single cell.
pub const MIN_SPAN: i32 = 3;

/// What to generate. Separate from the seed, because level progression varies
/// these while the seed is chosen per run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MazeParams {
    /// Width in cells. Rounded up to odd; see [`MazeParams::normalized`].
    pub cols: i32,
    /// Height in cells. Rounded up to odd.
    pub rows: i32,
    /// Target fraction of the maze's *interior* cells that are wall, in `0..=1`.
    ///
    /// The border ring is excluded from the count because it is always solid;
    /// counting it would make the same number mean different things at different
    /// maze sizes. A perfect maze is around 0.46 at the sizes the game uses and
    /// nothing denser can be built without cutting the maze into pieces, so a
    /// higher request is silently met with the densest maze available — see
    /// [`Maze::wall_density`] for what a maze actually ended up at.
    pub density: f32,
}

impl MazeParams {
    /// The first level. The plan's suggested 32×24, snapped up to odd.
    pub const LEVEL_ONE: Self = Self {
        cols: 33,
        rows: 25,
        density: 0.34,
    };

    /// Clamps the dimensions to what the carve can actually work with.
    ///
    /// Both spans must be odd: the carve treats odd coordinates as corridor
    /// cells and even ones as the walls between them, so an even span would put
    /// a corridor against the outer edge and leave the maze open to the void.
    /// Rounding *up* means a caller's size is a floor rather than a hint.
    pub fn normalized(self) -> Self {
        Self {
            cols: self.cols.max(MIN_SPAN) | 1,
            rows: self.rows.max(MIN_SPAN) | 1,
            density: self.density.clamp(0.0, 1.0),
        }
    }
}

impl Default for MazeParams {
    fn default() -> Self {
        Self::LEVEL_ONE
    }
}

/// A generated maze, and everything about it a level needs to start.
#[derive(Clone, Debug)]
pub struct Maze {
    /// The cells.
    pub grid: Grid,
    /// Cell the player's tank starts in. Always open.
    pub spawn: IVec2,
    /// Seed this maze came from.
    ///
    /// Logged when a level starts, so that a maze someone got stuck in can be
    /// regenerated exactly from a bug report.
    pub seed: u64,
    /// The parameters actually used, after [`MazeParams::normalized`].
    pub params: MazeParams,
}

impl Maze {
    /// Fraction of the interior cells that are wall.
    ///
    /// Compare against [`MazeParams::density`]; the two agree unless the request
    /// was denser than a perfect maze of this size can be.
    pub fn wall_density(&self) -> f32 {
        let interior = self.interior_count();
        if interior == 0 {
            return 0.0;
        }
        self.interior_walls() as f32 / interior as f32
    }

    /// Number of cells not on the always-solid border ring.
    fn interior_count(&self) -> i32 {
        (self.grid.width() - 2).max(0) * (self.grid.height() - 2).max(0)
    }

    fn interior_walls(&self) -> i32 {
        interior_cells(&self.grid)
            .filter(|cell| self.grid.is_wall(*cell))
            .count() as i32
    }
}

/// Generates a maze.
///
/// The same `params` and `seed` always produce the same maze: the generator uses
/// xoshiro256++, which `rand` documents as reproducible across releases, rather
/// than `StdRng`, whose algorithm is explicitly allowed to change between
/// versions and across platforms.
pub fn generate(params: MazeParams, seed: u64) -> Maze {
    let params = params.normalized();
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);

    let mut grid = Grid::solid(params.cols, params.rows);
    carve(&mut grid, &mut rng);
    thin(&mut grid, params.density, &mut rng);
    let spawn = pick_spawn(&grid, &mut rng);

    Maze {
        grid,
        spawn,
        seed,
        params,
    }
}

/// Iterates the cells that are not on the border ring.
fn interior_cells(grid: &Grid) -> impl Iterator<Item = IVec2> + '_ {
    (1..grid.height() - 1).flat_map(move |y| (1..grid.width() - 1).map(move |x| IVec2::new(x, y)))
}

/// Carves a perfect maze with a recursive backtracker.
///
/// The recursion is an explicit stack: a 64×48 maze is 1500 corridor cells deep
/// in the worst case, and this has to run in a wasm build whose stack is a
/// fraction of a native thread's.
///
/// Corridors live on odd coordinates. Lattice node `(i, j)` is grid cell
/// `(2i + 1, 2j + 1)`, and stepping between adjacent nodes opens the single
/// even-coordinate cell between them.
fn carve(grid: &mut Grid, rng: &mut impl Rng) {
    let nodes = IVec2::new((grid.width() - 1) / 2, (grid.height() - 1) / 2);
    let cell_of = |node: IVec2| node * 2 + IVec2::ONE;
    let index_of = |node: IVec2| (node.y * nodes.x + node.x) as usize;

    let start = IVec2::new(rng.random_range(0..nodes.x), rng.random_range(0..nodes.y));
    let mut visited = vec![false; (nodes.x * nodes.y) as usize];
    visited[index_of(start)] = true;
    grid.set(cell_of(start), Cell::Open);

    let mut stack = vec![start];
    while let Some(&node) = stack.last() {
        // Collect the unvisited neighbours, then pick one. Sampling directions
        // until one lands would waste RNG draws and, at the last node of a long
        // corridor, would not terminate reliably.
        let mut options = [IVec2::ZERO; 4];
        let mut count = 0;
        for step in ORTHOGONAL {
            let next = node + step;
            let inside = next.cmpge(IVec2::ZERO).all() && next.cmplt(nodes).all();
            if inside && !visited[index_of(next)] {
                options[count] = step;
                count += 1;
            }
        }

        if count == 0 {
            stack.pop();
            continue;
        }

        let step = options[rng.random_range(0..count)];
        let next = node + step;
        visited[index_of(next)] = true;
        grid.set(cell_of(node) + step, Cell::Open);
        grid.set(cell_of(next), Cell::Open);
        stack.push(next);
    }
}

/// Opens interior walls at random until the wall fraction drops to `density`.
///
/// Only walls with an open orthogonal neighbour are eligible, which is what
/// keeps the maze in one piece. Opening one wall can make a previously
/// ineligible neighbour eligible, so the shuffled candidate list is swept
/// repeatedly until either the target is met or a whole sweep opens nothing.
fn thin(grid: &mut Grid, density: f32, rng: &mut impl Rng) {
    let interior = (grid.width() - 2) * (grid.height() - 2);
    let target = (density * interior as f32).round() as i32;

    let mut candidates: Vec<IVec2> = interior_cells(grid)
        .filter(|cell| grid.is_wall(*cell))
        .collect();
    let mut walls = candidates.len() as i32;
    if walls <= target {
        return;
    }
    candidates.shuffle(rng);

    loop {
        let before = walls;
        for &cell in &candidates {
            if walls <= target {
                return;
            }
            if grid.is_open(cell) || !grid.has_open_neighbour(cell) {
                continue;
            }
            grid.set(cell, Cell::Open);
            walls -= 1;
        }
        if walls == before {
            return;
        }
    }
}

/// Picks the cell the player starts in.
///
/// Any open cell will do for now; M2 places the factories relative to it, at
/// which point the choice starts to matter.
///
/// # Panics
///
/// If the grid has no open cells, which [`carve`] makes impossible.
fn pick_spawn(grid: &Grid, rng: &mut impl Rng) -> IVec2 {
    let open: Vec<IVec2> = grid.open().collect();
    assert!(!open.is_empty(), "a carved maze always has open cells");
    open[rng.random_range(0..open.len())]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many seeds to sweep, when a property has to hold for every maze
    /// rather than for one lucky one.
    const SEEDS: u64 = 24;

    /// Sizes the game will realistically ask for, smallest to largest.
    const SIZES: [(i32, i32); 4] = [(5, 5), (21, 15), (33, 25), (65, 49)];

    fn maze(cols: i32, rows: i32, density: f32, seed: u64) -> Maze {
        generate(
            MazeParams {
                cols,
                rows,
                density,
            },
            seed,
        )
    }

    #[test]
    fn dimensions_are_rounded_up_to_odd() {
        let m = maze(32, 24, 0.3, 1);
        assert_eq!((m.grid.width(), m.grid.height()), (33, 25));
        assert_eq!((m.params.cols, m.params.rows), (33, 25));
    }

    #[test]
    fn dimensions_below_the_minimum_are_raised_to_it() {
        let m = maze(0, 1, 0.3, 1);
        assert_eq!((m.grid.width(), m.grid.height()), (MIN_SPAN, MIN_SPAN));
    }

    #[test]
    fn odd_dimensions_are_left_alone() {
        let m = maze(33, 25, 0.3, 1);
        assert_eq!((m.grid.width(), m.grid.height()), (33, 25));
    }

    #[test]
    fn every_maze_is_fully_connected() {
        for seed in 0..SEEDS {
            for (cols, rows) in SIZES {
                for density in [0.0, 0.1, 0.25, 0.34, 0.45, 1.0] {
                    let m = maze(cols, rows, density, seed);
                    assert!(
                        m.grid.is_connected(),
                        "{cols}×{rows} density {density} seed {seed} is split into pieces"
                    );
                }
            }
        }
    }

    #[test]
    fn the_border_is_always_solid() {
        for seed in 0..SEEDS {
            let m = maze(33, 25, 0.0, seed);
            let (w, h) = (m.grid.width(), m.grid.height());
            for x in 0..w {
                assert!(m.grid.is_wall(IVec2::new(x, 0)), "seed {seed} bottom edge");
                assert!(m.grid.is_wall(IVec2::new(x, h - 1)), "seed {seed} top edge");
            }
            for y in 0..h {
                assert!(m.grid.is_wall(IVec2::new(0, y)), "seed {seed} left edge");
                assert!(
                    m.grid.is_wall(IVec2::new(w - 1, y)),
                    "seed {seed} right edge"
                );
            }
        }
    }

    #[test]
    fn the_carve_reaches_every_corridor_cell() {
        // A perfect maze on the odd lattice has one open cell per node plus one
        // per edge of a spanning tree over them: `n` plus `n - 1`.
        for seed in 0..SEEDS {
            let m = maze(33, 25, 1.0, seed);
            let nodes = 16 * 12;
            assert_eq!(
                m.grid.open_count(),
                nodes + nodes - 1,
                "seed {seed} did not carve a spanning tree"
            );
        }
    }

    #[test]
    fn density_is_honoured_within_five_percent() {
        for seed in 0..8 {
            for (cols, rows) in [(21, 15), (33, 25), (65, 49)] {
                for density in [0.05, 0.15, 0.25, 0.35, 0.4] {
                    let m = maze(cols, rows, density, seed);
                    let achieved = m.wall_density();
                    assert!(
                        (achieved - density).abs() <= 0.05,
                        "{cols}×{rows} seed {seed}: asked for {density}, got {achieved}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_density_no_maze_can_reach_yields_the_densest_maze_instead() {
        // Nothing denser than the untouched carve can be built without splitting
        // the maze, so an impossible request has to come back as that.
        for seed in 0..8 {
            let perfect = maze(33, 25, 1.0, seed);
            let requested = maze(33, 25, 0.9, seed);
            assert_eq!(
                requested.grid, perfect.grid,
                "seed {seed}: an unreachable density should thin nothing"
            );
            // And that ceiling is what the doc comment on `density` claims.
            let ceiling = perfect.wall_density();
            assert!(
                (0.44..0.48).contains(&ceiling),
                "seed {seed}: perfect-maze density {ceiling} is outside the documented range"
            );
        }
    }

    #[test]
    fn zero_density_opens_the_whole_interior() {
        let m = maze(33, 25, 0.0, 7);
        assert_eq!(m.wall_density(), 0.0);
        // Only the border ring is left.
        let border = 33 * 25 - 31 * 23;
        assert_eq!(m.grid.walls().count(), border as usize);
    }

    #[test]
    fn thinning_only_ever_opens_cells() {
        // Density is a one-way knob: a lower target may not put walls back.
        for seed in 0..8 {
            let dense = maze(33, 25, 0.4, seed);
            let sparse = maze(33, 25, 0.15, seed);
            for (cell, _) in dense.grid.iter() {
                if dense.grid.is_open(cell) {
                    assert!(
                        sparse.grid.is_open(cell),
                        "seed {seed}: {cell:?} closed when the maze got sparser"
                    );
                }
            }
        }
    }

    #[test]
    fn the_same_seed_reproduces_the_maze_exactly() {
        for seed in 0..SEEDS {
            let params = MazeParams::LEVEL_ONE;
            let first = generate(params, seed);
            let second = generate(params, seed);
            assert_eq!(first.grid, second.grid, "seed {seed} grid");
            assert_eq!(first.spawn, second.spawn, "seed {seed} spawn");
        }
    }

    #[test]
    fn different_seeds_produce_different_mazes() {
        // Not a guarantee the generator can make in general, but at level-one
        // size the odds of a collision are vanishing, so a failure here means
        // the seed is being ignored.
        let a = generate(MazeParams::LEVEL_ONE, 1);
        let b = generate(MazeParams::LEVEL_ONE, 2);
        assert_ne!(a.grid, b.grid);
    }

    #[test]
    fn the_spawn_cell_is_always_open() {
        for seed in 0..SEEDS {
            for (cols, rows) in SIZES {
                let m = maze(cols, rows, 0.34, seed);
                assert!(
                    m.grid.is_open(m.spawn),
                    "{cols}×{rows} seed {seed} spawned in a wall at {:?}",
                    m.spawn
                );
            }
        }
    }

    #[test]
    fn the_smallest_maze_is_a_single_open_cell() {
        let m = maze(MIN_SPAN, MIN_SPAN, 1.0, 3);
        assert_eq!(m.grid.open_count(), 1);
        assert_eq!(m.spawn, IVec2::new(1, 1));
    }
}
