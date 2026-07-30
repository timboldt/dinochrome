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

use glam::{IVec2, Vec2};

/// World-space edge length of one grid cell, in pixels.
pub const CELL_SIZE: f32 = 64.0;

/// The four orthogonal steps in grid space.
pub const ORTHOGONAL: [IVec2; 4] = [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y];

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
