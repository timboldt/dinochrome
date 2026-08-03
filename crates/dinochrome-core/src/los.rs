//! Line of sight through the maze.
//!
//! "Can this thing see that thing" is the question a torpedo drone asks before
//! firing and a factory asks before shooting back, and the answer has to agree
//! with what a shell would actually do — a drone that fires at a player it cannot
//! hit is just noise on the screen.
//!
//! So sight is tested against the same thin wall boxes [`crate::collision`]
//! resolves against, not against whole cells. A [`Cell::Wall`] is a bar through
//! the middle of its cell with open floor either side of it, and a sight line can
//! legitimately pass through the open part: refusing that would blind everything
//! diagonally across a junction where a shell flies straight through.
//!
//! [`crate::collision::sweep`] would answer this correctly already — a zero-radius
//! sweep that hits nothing is a clear line. What it would not do is answer it
//! *cheaply*: its broad phase is the bounding box of the whole ray, which for a
//! ten-cell diagonal is a hundred cells to reject one at a time. This walks only
//! the cells the ray actually crosses, which for the same ray is twenty. The
//! geometry underneath is shared, so the two can never disagree about a wall.
//!
//! [`Cell::Wall`]: crate::grid::Cell::Wall

use glam::{IVec2, Vec2};

use crate::collision::hit_box;
use crate::grid::{CELL_SIZE, Grid};

/// True if nothing solid stands between `from` and `to`.
///
/// Endpoints are world positions, not cells, so a drone sitting off-centre in a
/// corridor sees what it would see from where it is standing. A point inside a
/// wall can see nothing, which is the right answer for the only way to get one:
/// asking about something that has already been destroyed.
pub fn line_of_sight(grid: &Grid, from: Vec2, to: Vec2) -> bool {
    let delta = to - from;
    let goal = grid.cell_at(to);
    let mut cell = grid.cell_at(from);

    // Amanatides & Woo: `t_max` is the fraction of the ray at which it leaves the
    // current cell across each axis, `t_delta` how much a whole cell costs. An
    // axis the ray does not move along never expires, so it sits at infinity and
    // the other axis drives the walk.
    let step = IVec2::new(sign(delta.x), sign(delta.y));
    let t_delta = Vec2::new(axis_delta(delta.x), axis_delta(delta.y));
    let mut t_max = Vec2::new(
        axis_exit(from.x, delta.x, grid.cell_min(cell).x),
        axis_exit(from.y, delta.y, grid.cell_min(cell).y),
    );

    loop {
        if blocked(grid, cell, from, delta) {
            return false;
        }
        if cell == goal {
            return true;
        }
        // Whichever boundary comes first is the one crossed next. Past the end of
        // the ray there is nothing left to cross: every cell it touches has been
        // tested and none of them stopped it.
        if t_max.x < t_max.y {
            if t_max.x > 1.0 {
                return true;
            }
            cell.x += step.x;
            t_max.x += t_delta.x;
        } else {
            if t_max.y > 1.0 {
                return true;
            }
            cell.y += step.y;
            t_max.y += t_delta.y;
        }
    }
}

/// True if `to` is within `range` of `from` and in plain sight of it.
///
/// The range test is squared and comes first, because it rejects most candidates
/// for the cost of a subtraction and a dot product and the sight walk is the
/// expensive half.
pub fn visible(grid: &Grid, from: Vec2, to: Vec2, range: f32) -> bool {
    from.distance_squared(to) <= range * range && line_of_sight(grid, from, to)
}

/// True if any wall box in `cell` intersects the ray `from + delta * t`, `t` in
/// `0..=1`.
fn blocked(grid: &Grid, cell: IVec2, from: Vec2, delta: Vec2) -> bool {
    grid.wall_boxes(cell)
        .iter()
        .any(|wall| hit_box(from, delta, 0.0, wall.min, wall.max).is_some())
}

/// Which way the walk steps along an axis: `0` if the ray does not move along it.
fn sign(component: f32) -> i32 {
    if component > 0.0 {
        1
    } else if component < 0.0 {
        -1
    } else {
        0
    }
}

/// Fraction of the ray spent crossing one whole cell along an axis.
fn axis_delta(component: f32) -> f32 {
    if component == 0.0 {
        f32::INFINITY
    } else {
        CELL_SIZE / component.abs()
    }
}

/// Fraction of the ray at which it leaves the starting cell across an axis.
fn axis_exit(start: f32, component: f32, cell_low: f32) -> f32 {
    if component > 0.0 {
        (cell_low + CELL_SIZE - start) / component
    } else if component < 0.0 {
        (cell_low - start) / component
    } else {
        f32::INFINITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision;
    use crate::grid::WALL_THICKNESS;
    use crate::maze::{self, MazeParams};

    /// A room with one wall cell in the middle of it.
    ///
    /// The bar is a lone post — nothing adjoins it — so it is `WALL_THICKNESS`
    /// square at the centre of cell `(2, 2)`, with open floor all round.
    fn post() -> Grid {
        Grid::from_rows(&[
            ".....", //
            ".....", "..#..", ".....", ".....",
        ])
    }

    /// A room split in two by a solid wall running the full width.
    fn partition() -> Grid {
        Grid::from_rows(&[
            ".....", //
            ".....", "#####", ".....", ".....",
        ])
    }

    fn center(grid: &Grid, x: i32, y: i32) -> Vec2 {
        grid.cell_center(IVec2::new(x, y))
    }

    #[test]
    fn an_empty_room_hides_nothing() {
        let grid = Grid::from_rows(&["....", "....", "....", "...."]);
        for from in grid.open() {
            for to in grid.open() {
                assert!(
                    line_of_sight(&grid, grid.cell_center(from), grid.cell_center(to)),
                    "{from:?} could not see {to:?}"
                );
            }
        }
    }

    #[test]
    fn sight_is_blocked_straight_through_a_wall() {
        let grid = partition();
        assert!(!line_of_sight(
            &grid,
            center(&grid, 2, 0),
            center(&grid, 2, 4)
        ));
    }

    #[test]
    fn sight_runs_freely_along_a_corridor_beside_a_wall() {
        let grid = partition();
        assert!(line_of_sight(
            &grid,
            center(&grid, 0, 0),
            center(&grid, 4, 0)
        ));
    }

    #[test]
    fn sight_is_symmetric() {
        let grid = maze::generate(MazeParams::LEVEL_ONE, 7).grid;
        let cells: Vec<IVec2> = grid.open().step_by(17).collect();
        for &a in &cells {
            for &b in &cells {
                let (from, to) = (grid.cell_center(a), grid.cell_center(b));
                assert_eq!(
                    line_of_sight(&grid, from, to),
                    line_of_sight(&grid, to, from),
                    "{a:?} and {b:?} disagree about each other"
                );
            }
        }
    }

    #[test]
    fn everything_can_see_itself() {
        let grid = maze::generate(MazeParams::LEVEL_ONE, 11).grid;
        for cell in grid.open() {
            let at = grid.cell_center(cell);
            assert!(
                line_of_sight(&grid, at, at),
                "{cell:?} could not see itself"
            );
        }
    }

    #[test]
    fn a_lone_post_blocks_only_what_is_behind_it() {
        let grid = post();
        // Dead in line with the post.
        assert!(!line_of_sight(
            &grid,
            center(&grid, 0, 2),
            center(&grid, 4, 2)
        ));
        // One cell over, past the end of a bar that is only a quarter of a cell
        // wide. A shell fired along this line would sail past, so sight must too.
        assert!(line_of_sight(
            &grid,
            center(&grid, 0, 1),
            center(&grid, 4, 1)
        ));
    }

    #[test]
    fn sight_passes_through_the_open_part_of_a_wall_cell() {
        // The bar in cell (2, 2) is 16 px through a 64 px cell, so a line grazing
        // the cell's edge crosses the cell without crossing anything solid. A
        // whole-cell raycast would call this blocked and be wrong about it.
        let grid = post();
        let just_inside = grid.cell_min(IVec2::new(2, 2)).y + 1.0;
        let from = Vec2::new(center(&grid, 0, 2).x, just_inside);
        let to = Vec2::new(center(&grid, 4, 2).x, just_inside);
        assert!(line_of_sight(&grid, from, to));
    }

    #[test]
    fn the_void_outside_the_maze_is_opaque() {
        let grid = post();
        let outside = Vec2::new(-4.0 * CELL_SIZE, center(&grid, 0, 2).y);
        assert!(!line_of_sight(&grid, outside, center(&grid, 4, 2)));
    }

    #[test]
    fn sight_agrees_with_where_a_shell_would_actually_get_to() {
        // The contract that makes line of sight worth having: if a drone can see
        // you, a shell it fires along that line reaches you. Swept at zero radius,
        // because that is the line being asked about — a fatter shell can still
        // clip a corner this line cleared.
        let grid = maze::generate(MazeParams::LEVEL_ONE, 3).grid;
        let cells: Vec<IVec2> = grid.open().step_by(23).collect();
        for &a in &cells {
            for &b in &cells {
                let (from, to) = (grid.cell_center(a), grid.cell_center(b));
                let swept = collision::sweep(&grid, from, 0.0, to - from);
                assert_eq!(
                    line_of_sight(&grid, from, to),
                    swept.hit.is_none(),
                    "{a:?} to {b:?}: sight and sweep disagree"
                );
            }
        }
    }

    #[test]
    fn range_is_measured_before_anything_is_looked_through() {
        let grid = Grid::from_rows(&["....", "....", "....", "...."]);
        let from = center(&grid, 0, 0);
        let to = center(&grid, 3, 0);
        let apart = from.distance(to);
        assert!(visible(&grid, from, to, apart + 1.0));
        assert!(!visible(&grid, from, to, apart - 1.0));
    }

    #[test]
    fn a_sight_line_along_a_wall_face_is_not_blocked_by_it() {
        // Exactly tangent to the bar: touching is not seeing through. Nudged out
        // by a hair so the answer does not ride on which way a float rounds.
        let grid = post();
        let bar_edge = center(&grid, 2, 2).y + WALL_THICKNESS * 0.5;
        let y = bar_edge + 0.5;
        assert!(line_of_sight(
            &grid,
            Vec2::new(center(&grid, 0, 2).x, y),
            Vec2::new(center(&grid, 4, 2).x, y),
        ));
    }

    #[test]
    fn a_long_diagonal_across_a_real_maze_terminates() {
        // The walk is driven by floating-point boundary crossings, so the thing
        // worth asserting is that it always ends — a missed goal cell would spin
        // until `t` ran past the end of the ray, and a mishandled zero axis would
        // not even do that.
        let grid = maze::generate(MazeParams::LEVEL_ONE, 5).grid;
        let size = grid.world_size();
        for corner in [
            Vec2::ZERO,
            Vec2::new(size.x, 0.0),
            size,
            Vec2::new(0.0, size.y),
        ] {
            line_of_sight(&grid, corner, size - corner);
        }
    }
}
