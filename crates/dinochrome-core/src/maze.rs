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
//!
//! Factory placement is the one part that does need the check. A factory is a
//! building wide enough to plug a one-cell corridor, so putting one on a cell the
//! rest of the maze depends on would wall a region off — see [`pick_factories`]
//! for how that is ruled out.

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
    /// How many drone factories to place.
    ///
    /// A ceiling rather than a promise: a maze too cramped to hold this many
    /// without walling itself off gets as many as fit. [`Maze::factories`] is what
    /// a level actually got.
    pub factories: i32,
}

impl MazeParams {
    /// The first level. The plan's suggested 32×24, snapped up to odd.
    pub const LEVEL_ONE: Self = Self {
        cols: 33,
        rows: 25,
        density: 0.34,
        factories: 4,
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
            factories: self.factories.max(0),
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
    /// Cells the drone factories stand in. All open, and none of them
    /// [`spawn`](Self::spawn).
    ///
    /// May be shorter than [`MazeParams::factories`] asked for; see
    /// [`pick_factories`].
    pub factories: Vec<IVec2>,
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
    let factories = pick_factories(&grid, spawn, params.factories, &mut rng);

    Maze {
        grid,
        spawn,
        factories,
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
/// Any open cell will do; the factories are then placed at a distance from
/// whichever one it turned out to be.
///
/// # Panics
///
/// If the grid has no open cells, which [`carve`] makes impossible.
fn pick_spawn(grid: &Grid, rng: &mut impl Rng) -> IVec2 {
    let open: Vec<IVec2> = grid.open().collect();
    assert!(!open.is_empty(), "a carved maze always has open cells");
    open[rng.random_range(0..open.len())]
}

/// How far, in cells, a factory must be from where the player starts.
///
/// Far enough that the level does not open with a factory in the windscreen, and
/// that finding the first one is navigation rather than luck.
const SPAWN_CLEARANCE: f32 = 8.0;

/// How far, in cells, two factories must be from each other.
///
/// Wide enough that clearing one is a separate trip from clearing the next, and —
/// once drones exist in M3 — that two of them cannot pool their output onto the
/// same corridor.
const FACTORY_SPACING: f32 = 6.0;

/// Picks the cells the factories stand in.
///
/// Three things have to hold, and the last is the interesting one:
///
/// - clear of the player's spawn by [`SPAWN_CLEARANCE`],
/// - clear of each other by [`FACTORY_SPACING`],
/// - and **not load-bearing**. A factory is a building wide enough to plug a
///   one-cell corridor, so a factory on a cell the maze routes through would seal
///   off everything behind it. Whether a cell is load-bearing is not something the
///   distance rules can express, so it is checked directly: seal the candidate in a
///   scratch copy of the grid and see whether what is left is still one piece.
///   The copy carries the factories already chosen, because three cells that are
///   each individually spare can still be a region's only three ways out.
///
/// The distances are wants rather than needs. If they cannot all be met the whole
/// sweep is retried with them scaled down, so a maze too cramped for the spacing
/// gets crowded factories rather than none — but the connectivity check is never
/// relaxed, so a maze can still come back with fewer factories than were asked
/// for. Callers have to cope with that; see [`Maze::factories`].
fn pick_factories(grid: &Grid, spawn: IVec2, wanted: i32, rng: &mut impl Rng) -> Vec<IVec2> {
    let wanted = wanted.max(0) as usize;
    let mut chosen = Vec::with_capacity(wanted);
    if wanted == 0 {
        return chosen;
    }

    let mut candidates: Vec<IVec2> = grid.open().filter(|cell| *cell != spawn).collect();
    candidates.shuffle(rng);
    // Sealed carries the chosen cells, so the connectivity check always sees the
    // maze as it will actually be played rather than as it was generated.
    let mut sealed = grid.clone();

    let mut relax = 1.0;
    while chosen.len() < wanted && relax > 0.1 {
        let clearance = SPAWN_CLEARANCE * relax;
        let spacing = FACTORY_SPACING * relax;
        for &cell in &candidates {
            if chosen.len() == wanted {
                break;
            }
            // A chosen cell reads as wall in `sealed`, which is also how an
            // earlier pass's picks are skipped on a later one.
            if sealed.is_wall(cell)
                || distance(cell, spawn) < clearance
                || chosen.iter().any(|other| distance(cell, *other) < spacing)
            {
                continue;
            }
            sealed.set(cell, Cell::Wall);
            if sealed.is_connected() {
                chosen.push(cell);
            } else {
                sealed.set(cell, Cell::Open);
            }
        }
        relax *= 0.6;
    }
    chosen
}

/// Straight-line distance between two cells, in cells.
fn distance(a: IVec2, b: IVec2) -> f32 {
    (a - b).as_vec2().length()
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
                ..MazeParams::LEVEL_ONE
            },
            seed,
        )
    }

    /// A maze of the given size with a given number of factories in it.
    fn with_factories(cols: i32, rows: i32, factories: i32, seed: u64) -> Maze {
        generate(
            MazeParams {
                cols,
                rows,
                factories,
                ..MazeParams::LEVEL_ONE
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
            assert_eq!(first.factories, second.factories, "seed {seed} factories");
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

    #[test]
    fn a_level_one_maze_gets_every_factory_it_asked_for() {
        for seed in 0..SEEDS {
            let m = generate(MazeParams::LEVEL_ONE, seed);
            assert_eq!(
                m.factories.len(),
                MazeParams::LEVEL_ONE.factories as usize,
                "seed {seed} only placed {:?}",
                m.factories
            );
        }
    }

    #[test]
    fn factories_stand_on_open_cells_and_never_on_the_spawn() {
        for seed in 0..SEEDS {
            for (cols, rows) in SIZES {
                let m = with_factories(cols, rows, 4, seed);
                for &factory in &m.factories {
                    assert!(
                        m.grid.is_open(factory),
                        "{cols}×{rows} seed {seed}: factory in a wall at {factory:?}"
                    );
                    assert_ne!(
                        factory, m.spawn,
                        "{cols}×{rows} seed {seed}: factory on the spawn cell"
                    );
                }
            }
        }
    }

    #[test]
    fn no_two_factories_are_placed_on_the_same_cell() {
        for seed in 0..SEEDS {
            let m = generate(MazeParams::LEVEL_ONE, seed);
            for (index, &factory) in m.factories.iter().enumerate() {
                assert!(
                    !m.factories[index + 1..].contains(&factory),
                    "seed {seed}: {factory:?} placed twice"
                );
            }
        }
    }

    #[test]
    fn factories_keep_their_distance_from_the_spawn_and_from_each_other() {
        // At level-one size there is room for the full clearances, so nothing
        // should have had to fall back on a relaxed sweep.
        for seed in 0..SEEDS {
            let m = generate(MazeParams::LEVEL_ONE, seed);
            for (index, &factory) in m.factories.iter().enumerate() {
                let from_spawn = distance(factory, m.spawn);
                assert!(
                    from_spawn >= SPAWN_CLEARANCE,
                    "seed {seed}: {factory:?} is only {from_spawn} cells from the spawn"
                );
                for &other in &m.factories[index + 1..] {
                    let apart = distance(factory, other);
                    assert!(
                        apart >= FACTORY_SPACING,
                        "seed {seed}: {factory:?} and {other:?} are only {apart} cells apart"
                    );
                }
            }
        }
    }

    #[test]
    fn no_factory_walls_off_part_of_the_maze() {
        // The invariant that makes a level winnable: a factory is wide enough to
        // plug a corridor, so with every one of them treated as solid the player
        // must still be able to reach all of them.
        for seed in 0..SEEDS {
            for (cols, rows) in SIZES {
                for factories in [1, 4, 8] {
                    let m = with_factories(cols, rows, factories, seed);
                    let mut sealed = m.grid.clone();
                    for &factory in &m.factories {
                        sealed.set(factory, Cell::Wall);
                    }
                    assert!(
                        sealed.is_connected(),
                        "{cols}×{rows} seed {seed} with {:?} cut the maze into pieces",
                        m.factories
                    );
                    // And reachable specifically from where the player starts.
                    assert_eq!(
                        sealed.reachable_from(m.spawn),
                        sealed.open_count(),
                        "{cols}×{rows} seed {seed}: not everything is reachable from the spawn"
                    );
                    // And winnable, checked by playing it out. A factory next to
                    // an open cell can be shot at, and killing it opens that cell
                    // up in turn — which is how a factory hemmed in by nothing but
                    // other factories, as happens in a maze small enough for the
                    // spacing to collapse, is still reachable in the end.
                    let mut standing = m.factories.clone();
                    while let Some(index) = standing
                        .iter()
                        .position(|cell| sealed.has_open_neighbour(*cell))
                    {
                        sealed.set(standing.swap_remove(index), Cell::Open);
                    }
                    assert!(
                        standing.is_empty(),
                        "{cols}×{rows} seed {seed}: {standing:?} can never be got at"
                    );
                }
            }
        }
    }

    #[test]
    fn asking_for_no_factories_places_none() {
        let m = with_factories(33, 25, 0, 5);
        assert!(m.factories.is_empty());
        // And a negative request is the same as none rather than a panic.
        assert!(with_factories(33, 25, -3, 5).factories.is_empty());
        assert_eq!(with_factories(33, 25, -3, 5).params.factories, 0);
    }

    #[test]
    fn a_maze_with_nowhere_to_put_a_factory_comes_back_with_fewer() {
        // One open cell, and the player is standing in it. There is no honest
        // answer other than an empty list, and the caller has to survive it.
        let m = with_factories(MIN_SPAN, MIN_SPAN, 4, 11);
        assert_eq!(m.grid.open_count(), 1);
        assert!(m.factories.is_empty());
    }

    #[test]
    fn a_cramped_maze_crowds_its_factories_rather_than_dropping_them() {
        // 11×11 has nothing like eight cells that are all 8 apart from the spawn
        // and 6 from each other, so the spacing has to give way.
        let m = with_factories(11, 11, 8, 77);
        assert_eq!(m.factories.len(), 8, "got {:?}", m.factories);
    }
}
