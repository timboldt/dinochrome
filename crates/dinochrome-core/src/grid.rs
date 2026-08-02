//! The maze grid: cell storage, world-space mapping, and connectivity.
//!
//! Two coordinate spaces meet here. *Grid* coordinates are integer cell indices
//! ([`IVec2`]); *world* coordinates are the continuous pixel space entities move
//! in ([`Vec2`]). Cell `(0, 0)` occupies the world-space square
//! `[0, CELL_SIZE) × [0, CELL_SIZE)`, so the maze lives entirely in the positive
//! quadrant and the mapping in both directions is a plain scale — no centring
//! offset to get the sign of wrong.
//!
//! Anything outside the grid reads back as [`Cell::Wall`]. That is not a
//! convenience: it means collision, line of sight and pathfinding all treat the
//! world as bounded without a single explicit bounds check.
//!
//! # Cells are not walls
//!
//! A [`Cell::Wall`] says *there is wall here*, not *this whole square is solid*.
//! The solid part is a thin line down the middle of the cell, and
//! [`Grid::wall_boxes`] derives it from the cell's neighbours. Keeping the grid
//! coarse and the geometry thin gets both halves of what the game wants: maze
//! generation, connectivity, line of sight and pathfinding all stay cheap
//! integer work on cells, while the tank drives down corridors nearly two cells
//! wide instead of squeezing through one.

use glam::{IVec2, Vec2};

/// World-space edge length of one grid cell, in pixels.
pub const CELL_SIZE: f32 = 64.0;

/// World-space thickness of a wall, in pixels.
///
/// A wall is a *line down the middle* of a cell the generator marked solid, not
/// the cell itself. The grid stays what the generator, the renderer and later
/// the pathfinder all index by, while what a tank actually bumps into is a
/// quarter of a cell thick. See [`Grid::wall_boxes`] for the geometry that falls
/// out of that.
///
/// The visible consequence is corridor width. Wall lines sit two cells apart, so
/// a corridor's free width is `2 * CELL_SIZE - WALL_THICKNESS` — 112 px against
/// the tank's 40 px, room enough to turn around in and to dodge in, where a
/// full-cell wall left only 64.
pub const WALL_THICKNESS: f32 = CELL_SIZE * 0.25;

/// The four orthogonal steps in grid space.
pub const ORTHOGONAL: [IVec2; 4] = [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y];

/// An axis-aligned box in world space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Aabb {
    /// Low corner.
    pub min: Vec2,
    /// High corner.
    pub max: Vec2,
}

impl Aabb {
    /// The point of the box nearest `point` — `point` itself if it is inside.
    pub fn nearest(self, point: Vec2) -> Vec2 {
        point.clamp(self.min, self.max)
    }

    /// Centre of the box.
    pub fn center(self) -> Vec2 {
        (self.min + self.max) * 0.5
    }

    /// Width and height.
    pub fn size(self) -> Vec2 {
        self.max - self.min
    }
}

/// The solid geometry of one wall cell.
///
/// At most two boxes: a horizontal bar and a vertical bar crossing at the cell
/// centre. Every shape a wall cell can take — cross, tee, elbow, straight run,
/// lone post — is the union of those two, so this is a fixed-size value and
/// nothing allocates to ask a cell what it looks like.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WallShape {
    boxes: [Aabb; 2],
    len: u8,
}

impl WallShape {
    /// The boxes making up the shape. Empty for an open cell.
    pub fn iter(self) -> impl Iterator<Item = Aabb> {
        self.boxes.into_iter().take(self.len as usize)
    }

    /// True if the cell contributes no solid geometry at all.
    pub fn is_empty(self) -> bool {
        self.len == 0
    }

    fn one(only: Aabb) -> Self {
        Self {
            boxes: [only, Aabb::default()],
            len: 1,
        }
    }

    fn two(first: Aabb, second: Aabb) -> Self {
        Self {
            boxes: [first, second],
            len: 2,
        }
    }
}

/// What occupies a single grid cell.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Cell {
    /// Solid. Blocks movement, shells and sight.
    #[default]
    Wall,
    /// Empty floor.
    Open,
}

impl Cell {
    /// True for [`Cell::Wall`].
    pub fn is_wall(self) -> bool {
        self == Cell::Wall
    }
}

/// A rectangular grid of open and wall cells.
///
/// Dimensions are stored as `i32` rather than `u32` because every consumer is
/// doing signed coordinate arithmetic — neighbour offsets, negative indices from
/// out-of-bounds world positions — and a grid large enough for the sign to
/// matter would not fit in memory anyway.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Grid {
    width: i32,
    height: i32,
    cells: Vec<Cell>,
}

impl Grid {
    /// Creates a grid of solid wall, ready to be carved.
    ///
    /// # Panics
    ///
    /// If either dimension is zero.
    pub fn solid(width: i32, height: i32) -> Self {
        assert!(
            width > 0 && height > 0,
            "grid must be at least 1×1, got {width}×{height}"
        );
        Self {
            width,
            height,
            cells: vec![Cell::Wall; (width * height) as usize],
        }
    }

    /// Builds a grid from rows of `#` (wall) and anything else (open).
    ///
    /// The **first row is the top** of the maze — the highest `y` — so that a
    /// grid written as a string literal in a test looks like what it draws. All
    /// rows must be the same length.
    ///
    /// # Panics
    ///
    /// If `rows` is empty or the rows are not all the same length.
    pub fn from_rows(rows: &[&str]) -> Self {
        let height = rows.len() as i32;
        assert!(height > 0, "grid must have at least one row");
        let width = rows[0].len() as i32;
        assert!(width > 0, "grid rows must not be empty");

        let mut grid = Self::solid(width, height);
        for (from_top, row) in rows.iter().enumerate() {
            assert_eq!(
                row.len() as i32,
                width,
                "row {from_top} has a different length than the first row"
            );
            let y = height - 1 - from_top as i32;
            for (x, byte) in row.bytes().enumerate() {
                if byte != b'#' {
                    grid.set(IVec2::new(x as i32, y), Cell::Open);
                }
            }
        }
        grid
    }

    /// Width in cells.
    pub fn width(&self) -> i32 {
        self.width
    }

    /// Height in cells.
    pub fn height(&self) -> i32 {
        self.height
    }

    /// True if `cell` is inside the grid.
    pub fn contains(&self, cell: IVec2) -> bool {
        cell.x >= 0 && cell.y >= 0 && cell.x < self.width && cell.y < self.height
    }

    /// Reads a cell, treating everything outside the grid as solid wall.
    pub fn get(&self, cell: IVec2) -> Cell {
        if self.contains(cell) {
            self.cells[self.index(cell)]
        } else {
            Cell::Wall
        }
    }

    /// True if `cell` is a wall or lies outside the grid.
    pub fn is_wall(&self, cell: IVec2) -> bool {
        self.get(cell).is_wall()
    }

    /// True if `cell` is inside the grid and open.
    pub fn is_open(&self, cell: IVec2) -> bool {
        !self.is_wall(cell)
    }

    /// Writes a cell.
    ///
    /// # Panics
    ///
    /// If `cell` is outside the grid. Unlike reading, writing out of bounds is
    /// always a bug in the caller rather than a case to be handled.
    pub fn set(&mut self, cell: IVec2, value: Cell) {
        assert!(
            self.contains(cell),
            "cell {cell:?} is outside a {}×{} grid",
            self.width,
            self.height
        );
        let index = self.index(cell);
        self.cells[index] = value;
    }

    /// Iterates over every cell, in row-major order from the bottom row up.
    pub fn iter(&self) -> impl Iterator<Item = (IVec2, Cell)> + '_ {
        (0..self.height).flat_map(move |y| {
            (0..self.width).map(move |x| {
                let cell = IVec2::new(x, y);
                (cell, self.get(cell))
            })
        })
    }

    /// Iterates over the coordinates of every wall cell.
    pub fn walls(&self) -> impl Iterator<Item = IVec2> + '_ {
        self.iter()
            .filter(|(_, cell)| cell.is_wall())
            .map(|(at, _)| at)
    }

    /// Iterates over the coordinates of every open cell.
    pub fn open(&self) -> impl Iterator<Item = IVec2> + '_ {
        self.iter()
            .filter(|(_, cell)| !cell.is_wall())
            .map(|(at, _)| at)
    }

    /// Number of open cells.
    pub fn open_count(&self) -> usize {
        self.cells.iter().filter(|cell| !cell.is_wall()).count()
    }

    /// True if any orthogonal neighbour of `cell` is open.
    ///
    /// Wall removal during generation leans on this: a wall with an open
    /// neighbour can always be opened without splitting the maze, because the
    /// new cell necessarily joins the component that neighbour is already in.
    pub fn has_open_neighbour(&self, cell: IVec2) -> bool {
        ORTHOGONAL.iter().any(|step| self.is_open(cell + *step))
    }

    /// Counts the open cells reachable from `start` by orthogonal steps.
    ///
    /// Returns zero if `start` is itself a wall.
    pub fn reachable_from(&self, start: IVec2) -> usize {
        if self.is_wall(start) {
            return 0;
        }

        let mut seen = vec![false; self.cells.len()];
        let mut stack = vec![start];
        seen[self.index(start)] = true;
        let mut count = 0;

        while let Some(cell) = stack.pop() {
            count += 1;
            for step in ORTHOGONAL {
                let next = cell + step;
                if self.is_wall(next) {
                    continue;
                }
                let index = self.index(next);
                if !seen[index] {
                    seen[index] = true;
                    stack.push(next);
                }
            }
        }
        count
    }

    /// True if every open cell can be reached from every other.
    ///
    /// A grid with no open cells at all is vacuously connected.
    pub fn is_connected(&self) -> bool {
        match self.open().next() {
            None => true,
            Some(start) => self.reachable_from(start) == self.open_count(),
        }
    }

    /// Grid coordinate containing the world position `world`.
    ///
    /// May be outside the grid, which is exactly what callers want: [`get`]
    /// will report it as wall.
    ///
    /// [`get`]: Self::get
    pub fn cell_at(&self, world: Vec2) -> IVec2 {
        (world / CELL_SIZE).floor().as_ivec2()
    }

    /// World position of the centre of `cell`.
    pub fn cell_center(&self, cell: IVec2) -> Vec2 {
        (cell.as_vec2() + Vec2::splat(0.5)) * CELL_SIZE
    }

    /// World position of the low corner of `cell`.
    pub fn cell_min(&self, cell: IVec2) -> Vec2 {
        cell.as_vec2() * CELL_SIZE
    }

    /// World position of the high corner of `cell`.
    pub fn cell_max(&self, cell: IVec2) -> Vec2 {
        self.cell_min(cell) + Vec2::splat(CELL_SIZE)
    }

    /// The solid geometry `cell` contributes: empty if the cell is open.
    ///
    /// A wall is a [`WALL_THICKNESS`]-wide line through the cell centre. It is
    /// run out to the cell edge on each side whose neighbour is also wall, so
    /// that adjacent cells' bars meet flush and a run of wall reads as one
    /// unbroken line. A side facing open floor stops at half thickness instead,
    /// which is what makes a dead end finish in a squared-off stub rather than
    /// swelling back out to a full block.
    ///
    /// Only neighbours *inside* the grid join up. The void beyond the border is
    /// solid to collision, but letting a bar run out into it would sprout a
    /// tooth off every cell of the border ring, pointing out of the maze at
    /// something already solid.
    ///
    /// Cells outside the grid are solid in their entirety, so [`is_wall`]'s
    /// promise that the world is bounded holds without this shape logic having
    /// to be right about the void. Nothing can reach that far anyway — the
    /// border ring stops it half a cell earlier.
    ///
    /// [`is_wall`]: Self::is_wall
    pub fn wall_boxes(&self, cell: IVec2) -> WallShape {
        if !self.contains(cell) {
            return WallShape::one(Aabb {
                min: self.cell_min(cell),
                max: self.cell_max(cell),
            });
        }
        if self.is_open(cell) {
            return WallShape::default();
        }

        let (lo, hi) = (self.cell_min(cell), self.cell_max(cell));
        let mid = self.cell_center(cell);
        let half = WALL_THICKNESS * 0.5;

        let joins = |step: IVec2| self.contains(cell + step) && self.is_wall(cell + step);
        let (left, right) = (joins(IVec2::NEG_X), joins(IVec2::X));
        let (down, up) = (joins(IVec2::NEG_Y), joins(IVec2::Y));

        let horizontal = Aabb {
            min: Vec2::new(if left { lo.x } else { mid.x - half }, mid.y - half),
            max: Vec2::new(if right { hi.x } else { mid.x + half }, mid.y + half),
        };
        let vertical = Aabb {
            min: Vec2::new(mid.x - half, if down { lo.y } else { mid.y - half }),
            max: Vec2::new(mid.x + half, if up { hi.y } else { mid.y + half }),
        };

        match (left || right, down || up) {
            (true, true) => WallShape::two(horizontal, vertical),
            (false, true) => WallShape::one(vertical),
            // A horizontal run — or a lone post with no wall neighbour at all,
            // which `horizontal` has already collapsed to a thickness square.
            _ => WallShape::one(horizontal),
        }
    }

    /// World-space size of the whole grid, in pixels.
    ///
    /// The grid spans `Vec2::ZERO` to this point.
    pub fn world_size(&self) -> Vec2 {
        Vec2::new(self.width as f32, self.height as f32) * CELL_SIZE
    }

    fn index(&self, cell: IVec2) -> usize {
        (cell.y * self.width + cell.x) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three disjoint open regions: a three-cell corridor along the bottom and
    /// two single cells sealed off above it.
    const SPLIT: &[&str] = &[
        "#####", //
        "#.#.#", //
        "#####", //
        "#...#", //
        "#####", //
    ];

    #[test]
    fn from_rows_reads_the_first_row_as_the_top() {
        let grid = Grid::from_rows(&[
            "..#", //
            "###", //
        ]);
        assert_eq!(grid.width(), 3);
        assert_eq!(grid.height(), 2);
        // The bottom row (y == 0) is the one written last.
        assert!(grid.is_wall(IVec2::new(0, 0)));
        // The top row (y == 1) is the one written first.
        assert!(grid.is_open(IVec2::new(0, 1)));
        assert!(grid.is_wall(IVec2::new(2, 1)));
    }

    #[test]
    fn a_fresh_grid_is_entirely_wall() {
        let grid = Grid::solid(4, 3);
        assert_eq!(grid.open_count(), 0);
        assert_eq!(grid.walls().count(), 12);
    }

    #[test]
    fn everything_outside_the_grid_is_wall() {
        let grid = Grid::from_rows(&["..", ".."]);
        assert_eq!(grid.open_count(), 4);
        for outside in [
            IVec2::new(-1, 0),
            IVec2::new(0, -1),
            IVec2::new(2, 0),
            IVec2::new(0, 2),
            IVec2::new(-9000, 9000),
        ] {
            assert!(grid.is_wall(outside), "{outside:?} should read as wall");
            assert!(!grid.contains(outside));
        }
    }

    #[test]
    #[should_panic(expected = "outside a 2×2 grid")]
    fn writing_outside_the_grid_panics() {
        Grid::solid(2, 2).set(IVec2::new(2, 0), Cell::Open);
    }

    #[test]
    fn cell_and_world_coordinates_round_trip() {
        let grid = Grid::solid(8, 8);
        for cell in [IVec2::ZERO, IVec2::new(3, 5), IVec2::new(7, 7)] {
            assert_eq!(grid.cell_at(grid.cell_center(cell)), cell, "{cell:?}");
        }
    }

    #[test]
    fn cell_bounds_are_half_open_so_neighbours_do_not_overlap() {
        let grid = Grid::solid(4, 4);
        let cell = IVec2::new(1, 2);
        assert_eq!(grid.cell_min(cell), Vec2::new(64.0, 128.0));
        assert_eq!(grid.cell_max(cell), Vec2::new(128.0, 192.0));
        // The shared edge belongs to the higher cell.
        assert_eq!(grid.cell_at(Vec2::new(128.0, 192.0)), IVec2::new(2, 3));
    }

    #[test]
    fn world_positions_left_of_the_origin_map_to_negative_cells() {
        // Truncation instead of floor would map -1.0 to cell 0 and let entities
        // walk straight out of the maze.
        let grid = Grid::solid(4, 4);
        assert_eq!(grid.cell_at(Vec2::new(-1.0, -1.0)), IVec2::new(-1, -1));
        assert_eq!(grid.cell_at(Vec2::new(-64.0, -65.0)), IVec2::new(-1, -2));
    }

    #[test]
    fn world_size_covers_every_cell() {
        assert_eq!(Grid::solid(33, 25).world_size(), Vec2::new(2112.0, 1600.0));
    }

    #[test]
    fn reachable_from_reaches_a_whole_component_and_no_more() {
        let grid = Grid::from_rows(SPLIT);
        // The bottom corridor: three cells, all mutually reachable.
        assert_eq!(grid.reachable_from(IVec2::new(2, 1)), 3);
        // The sealed cells above it are components of one.
        assert_eq!(grid.reachable_from(IVec2::new(1, 3)), 1);
    }

    #[test]
    fn reachable_from_never_steps_diagonally() {
        // The two open cells touch at a corner and nowhere else.
        let grid = Grid::from_rows(&[
            "####", //
            "#.##", //
            "##.#", //
            "####", //
        ]);
        assert_eq!(grid.open_count(), 2);
        assert_eq!(grid.reachable_from(IVec2::new(1, 2)), 1);
        assert!(!grid.is_connected());
    }

    #[test]
    fn reachable_from_a_wall_is_nothing() {
        assert_eq!(Grid::from_rows(SPLIT).reachable_from(IVec2::ZERO), 0);
    }

    #[test]
    fn is_connected_rejects_a_split_grid() {
        let grid = Grid::from_rows(SPLIT);
        assert_eq!(grid.open_count(), 5);
        assert!(!grid.is_connected());
    }

    #[test]
    fn is_connected_accepts_a_single_component() {
        let grid = Grid::from_rows(&[
            "#####", //
            "#...#", //
            "###.#", //
            "#...#", //
            "#####", //
        ]);
        assert!(grid.is_connected());
    }

    #[test]
    fn a_grid_with_no_open_cells_is_vacuously_connected() {
        assert!(Grid::solid(3, 3).is_connected());
    }

    /// Half the wall thickness, the amount a bar sticks out past a cell centre.
    const HALF: f32 = WALL_THICKNESS * 0.5;

    /// The one box of a cell that has exactly one.
    fn only_box(grid: &Grid, cell: IVec2) -> Aabb {
        let shape = grid.wall_boxes(cell);
        let boxes: Vec<Aabb> = shape.iter().collect();
        assert_eq!(boxes.len(), 1, "{cell:?} should be a single box");
        boxes[0]
    }

    #[test]
    fn an_open_cell_has_no_solid_geometry() {
        let grid = Grid::from_rows(&["...", ".#.", "..."]);
        assert!(grid.wall_boxes(IVec2::new(0, 0)).is_empty());
        assert_eq!(grid.wall_boxes(IVec2::new(0, 0)).iter().count(), 0);
    }

    #[test]
    fn a_lone_wall_is_a_post_of_exactly_one_thickness() {
        // Nothing to join onto in any direction.
        let grid = Grid::from_rows(&["...", ".#.", "..."]);
        let post = only_box(&grid, IVec2::new(1, 1));
        assert_eq!(post.size(), Vec2::splat(WALL_THICKNESS));
        assert_eq!(post.center(), grid.cell_center(IVec2::new(1, 1)));
    }

    #[test]
    fn a_straight_run_is_one_bar_per_cell_and_they_meet_flush() {
        // Three walls in a row across the middle.
        let grid = Grid::from_rows(&[".....", ".###.", "....."]);
        let mid = only_box(&grid, IVec2::new(2, 1));
        // The middle of the run spans its cell edge to edge, so the next cell's
        // bar starts exactly where this one ends — no seam.
        assert_eq!(mid.min.x, grid.cell_min(IVec2::new(2, 1)).x);
        assert_eq!(mid.max.x, grid.cell_max(IVec2::new(2, 1)).x);
        assert_eq!(mid.size().y, WALL_THICKNESS);

        let left_end = only_box(&grid, IVec2::new(1, 1));
        assert_eq!(left_end.max.x, grid.cell_max(IVec2::new(1, 1)).x);
        // The open end stops half a thickness past the centre: a stub, not a
        // block that swells back out to fill the cell.
        assert_eq!(left_end.min.x, grid.cell_center(IVec2::new(1, 1)).x - HALF);
    }

    #[test]
    fn a_run_is_thin_across_its_length_whichever_way_it_points() {
        let across = Grid::from_rows(&[".....", ".###.", "....."]);
        let down = Grid::from_rows(&["..#..", "..#..", "..#.."]);
        assert_eq!(only_box(&across, IVec2::new(2, 1)).size().y, WALL_THICKNESS);
        assert_eq!(only_box(&down, IVec2::new(2, 1)).size().x, WALL_THICKNESS);
    }

    #[test]
    fn a_crossing_is_two_bars_and_covers_the_whole_junction() {
        let grid = Grid::from_rows(&[
            "..#..", //
            "..#..", //
            "#####", //
            "..#..", //
            "..#..", //
        ]);
        let centre = IVec2::new(2, 2);
        let boxes: Vec<Aabb> = grid.wall_boxes(centre).iter().collect();
        assert_eq!(
            boxes.len(),
            2,
            "a four-way junction is a horizontal and a vertical bar"
        );
        // Between them they reach every edge of the cell, so the arms join up.
        let (lo, hi) = (grid.cell_min(centre), grid.cell_max(centre));
        assert!(boxes.iter().any(|b| b.min.x == lo.x && b.max.x == hi.x));
        assert!(boxes.iter().any(|b| b.min.y == lo.y && b.max.y == hi.y));
        // And both are one thickness across.
        assert!(boxes.iter().any(|b| b.size().y == WALL_THICKNESS));
        assert!(boxes.iter().any(|b| b.size().x == WALL_THICKNESS));
    }

    #[test]
    fn an_elbow_reaches_only_the_two_edges_its_arms_leave_by() {
        // Wall coming in from the left and turning up: a corner at (2,1).
        let grid = Grid::from_rows(&[
            "..#..", //
            ".##..", //
            ".....", //
        ]);
        let corner = IVec2::new(2, 1);
        let boxes: Vec<Aabb> = grid.wall_boxes(corner).iter().collect();
        assert_eq!(boxes.len(), 2);
        let (lo, hi) = (grid.cell_min(corner), grid.cell_max(corner));
        // Left and up are wall, so those edges are reached.
        assert!(boxes.iter().any(|b| b.min.x == lo.x));
        assert!(boxes.iter().any(|b| b.max.y == hi.y));
        // Right and down face open floor, so nothing reaches those edges.
        assert!(boxes.iter().all(|b| b.max.x < hi.x));
        assert!(boxes.iter().all(|b| b.min.y > lo.y));
    }

    #[test]
    fn the_border_ring_never_reaches_into_the_void() {
        // Every box of every border cell has to stay inside the grid, or the
        // maze draws teeth along its outside.
        let grid = Grid::from_rows(&[
            "#####", //
            "#.#.#", //
            "#...#", //
            "#.#.#", //
            "#####", //
        ]);
        let world = grid.world_size();
        for cell in grid.walls() {
            for wall in grid.wall_boxes(cell).iter() {
                assert!(
                    wall.min.x >= 0.0 && wall.min.y >= 0.0,
                    "{cell:?} reaches below the origin: {wall:?}"
                );
                assert!(
                    wall.max.x <= world.x && wall.max.y <= world.y,
                    "{cell:?} reaches past the far edge: {wall:?}"
                );
            }
        }
    }

    #[test]
    fn the_border_ring_is_a_closed_thin_rectangle() {
        let grid = Grid::from_rows(&[
            "#####", //
            "#...#", //
            "#...#", //
            "#...#", //
            "#####", //
        ]);
        // The left border column: each cell contributes a vertical bar, and
        // consecutive bars abut exactly, so nothing can slip between them.
        for y in 0..grid.height() - 1 {
            let here = grid.wall_boxes(IVec2::new(0, y));
            let above = grid.wall_boxes(IVec2::new(0, y + 1));
            let top = here.iter().map(|b| b.max.y).fold(f32::MIN, f32::max);
            let bottom = above.iter().map(|b| b.min.y).fold(f32::MAX, f32::min);
            assert!(
                top >= bottom,
                "gap in the border between y {y} and {}",
                y + 1
            );
        }
        // And the ring sits half a cell in from the world edge, which is what
        // widens every corridor including the outermost one.
        let side = only_box(&grid, IVec2::new(0, 2));
        assert_eq!(side.max.x, CELL_SIZE * 0.5 + HALF);
    }

    #[test]
    fn outside_the_grid_is_solid_to_the_last_corner() {
        // The shape logic must not thin the void: it is the backstop that makes
        // the world bounded.
        let grid = Grid::from_rows(&["..", ".."]);
        let outside = IVec2::new(-1, 0);
        let solid = only_box(&grid, outside);
        assert_eq!(solid.min, grid.cell_min(outside));
        assert_eq!(solid.max, grid.cell_max(outside));
    }

    #[test]
    fn every_box_stays_within_its_own_cell() {
        // Bars extend to a cell edge but never across it, so a cell's geometry
        // can be found by looking at that cell alone.
        let grid = Grid::from_rows(&[
            "#####", //
            "#.#.#", //
            "#.#.#", //
            "#...#", //
            "#####", //
        ]);
        for cell in grid.walls() {
            let (lo, hi) = (grid.cell_min(cell), grid.cell_max(cell));
            for wall in grid.wall_boxes(cell).iter() {
                assert!(
                    wall.min.cmpge(lo).all() && wall.max.cmple(hi).all(),
                    "{cell:?} box {wall:?} escapes {lo:?}..{hi:?}"
                );
            }
        }
    }

    #[test]
    fn nearest_clamps_onto_the_box_and_leaves_interior_points_alone() {
        let b = Aabb {
            min: Vec2::new(10.0, 20.0),
            max: Vec2::new(30.0, 40.0),
        };
        assert_eq!(b.nearest(Vec2::new(20.0, 30.0)), Vec2::new(20.0, 30.0));
        assert_eq!(b.nearest(Vec2::new(0.0, 30.0)), Vec2::new(10.0, 30.0));
        assert_eq!(b.nearest(Vec2::new(100.0, 100.0)), Vec2::new(30.0, 40.0));
        assert_eq!(b.center(), Vec2::new(20.0, 30.0));
        assert_eq!(b.size(), Vec2::new(20.0, 20.0));
    }

    #[test]
    fn has_open_neighbour_ignores_diagonals_and_the_cell_itself() {
        let grid = Grid::from_rows(&[
            "#.#", //
            "###", //
            "#.#", //
        ]);
        // Centre: neighbours above and below are open.
        assert!(grid.has_open_neighbour(IVec2::new(1, 1)));
        // Middle of the left edge: both open cells are diagonal from it, and
        // its own two out-of-grid neighbours read as wall.
        assert!(!grid.has_open_neighbour(IVec2::new(0, 1)));
    }
}
