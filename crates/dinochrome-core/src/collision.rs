//! Moving a circle through the maze grid.
//!
//! Entities are circles; walls are the axis-aligned squares of the grid. A move
//! is resolved one axis at a time — X first with the old Y, then Y with the new
//! X — which is what gives wall sliding for free: a diagonal push into a wall
//! loses the component that is blocked and keeps the one that is not, with no
//! special case for it anywhere.
//!
//! Resolution against a single wall is exact circle geometry rather than a box
//! approximation. Where the circle's centre is level with a wall face the
//! constraint is that face; where it is off the end of the face, the constraint
//! is the wall's corner, and the circle is allowed to come nearer along the axis
//! of travel by exactly as much as the Pythagorean slack allows. That is what
//! lets a tank round the corner of a corridor mouth instead of catching on it.
//!
//! # Tunnelling
//!
//! Long moves are split into substeps short enough that the swept region always
//! overlaps every wall in between. Callers therefore do not have to clamp
//! velocities: shells in M2 can be as fast as they like without passing through
//! walls. See [`MAX_SUBSTEP`].

use glam::{BVec2, IVec2, Vec2};

use crate::grid::{CELL_SIZE, Grid};

/// Longest distance resolved in one substep, in world units.
///
/// Half a cell. A wall is a full cell thick, so a substep this short cannot
/// straddle one: the swept span always intersects any wall it crosses, and the
/// clamp sees it.
pub const MAX_SUBSTEP: f32 = CELL_SIZE * 0.5;

/// Slack, in world units, before a contact counts as an overlap.
///
/// Resolution parks a circle exactly against a wall face, and "exactly" in `f32`
/// at maze-sized coordinates means within a fraction of a pixel either way. Two
/// places need to tolerate that: [`is_clear`], which would otherwise report the
/// resolver's own output as a collision, and the approach test in [`resolve`],
/// which would otherwise let a circle resting against a wall drift through it.
///
/// A sixty-fourth of a pixel — far below anything visible, far above `f32` noise
/// at these magnitudes, and four thousand times thinner than a wall.
pub const SKIN: f32 = 1.0 / 64.0;

/// Where a move ended up, and which axes were stopped short.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slide {
    /// The resolved position.
    pub position: Vec2,
    /// Per-axis: true if a wall clamped this axis at any point during the move.
    ///
    /// Callers use this to zero the corresponding velocity component. Without
    /// that, a tank held against a wall keeps accumulating speed into it and
    /// shoots off the moment it turns away.
    pub blocked: BVec2,
}

/// Moves a circle of `radius` from `position` by `delta`, sliding along walls.
///
/// `position` is assumed not to already overlap a wall. Anything outside the
/// grid counts as solid, so the maze is bounded without a separate check.
pub fn slide(grid: &Grid, position: Vec2, radius: f32, delta: Vec2) -> Slide {
    let distance = delta.length();
    let mut result = Slide {
        position,
        blocked: BVec2::FALSE,
    };
    if distance == 0.0 {
        return result;
    }

    // `ceil` of a positive quotient is at least 1, so there is always a step.
    let steps = (distance / MAX_SUBSTEP).ceil() as u32;
    let step = delta / steps as f32;

    for _ in 0..steps {
        let x = resolve(grid, result.position, radius, step.x, Axis::X);
        result.blocked.x |= x != result.position.x + step.x;
        result.position.x = x;

        let y = resolve(grid, result.position, radius, step.y, Axis::Y);
        result.blocked.y |= y != result.position.y + step.y;
        result.position.y = y;
    }
    result
}

/// True if a circle of `radius` at `position` overlaps no wall.
pub fn is_clear(grid: &Grid, position: Vec2, radius: f32) -> bool {
    let low = grid.cell_at(position - Vec2::splat(radius));
    let high = grid.cell_at(position + Vec2::splat(radius));
    for y in low.y..=high.y {
        for x in low.x..=high.x {
            let cell = IVec2::new(x, y);
            if !grid.is_wall(cell) {
                continue;
            }
            let min = grid.cell_min(cell);
            let max = grid.cell_max(cell);
            let nearest = position.clamp(min, max);
            let allowed = (radius - SKIN).max(0.0);
            if position.distance_squared(nearest) < allowed * allowed {
                return false;
            }
        }
    }
    true
}

/// Which axis a resolution step is moving along.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Axis {
    X,
    Y,
}

/// Resolves a single-axis move, returning the coordinate on that axis.
///
/// The other axis of `position` is held fixed, which is what makes this
/// axis-separated: the circle's extent across the direction of travel is
/// whatever it is right now, not what it will be after the full move.
fn resolve(grid: &Grid, position: Vec2, radius: f32, step: f32, axis: Axis) -> f32 {
    let (moving, fixed) = match axis {
        Axis::X => (position.x, position.y),
        Axis::Y => (position.y, position.x),
    };
    if step == 0.0 {
        return moving;
    }

    let target = moving + step;
    // The whole span the circle sweeps, so a wall cannot hide between where the
    // circle started and where it is trying to end up.
    let (low, high) = if step > 0.0 {
        (moving - radius, target + radius)
    } else {
        (target - radius, moving + radius)
    };
    let sweep = cell_span(low, high);
    let across = cell_span(fixed - radius, fixed + radius);

    let mut resolved = target;
    for along in sweep.0..=sweep.1 {
        for cross in across.0..=across.1 {
            let cell = match axis {
                Axis::X => IVec2::new(along, cross),
                Axis::Y => IVec2::new(cross, along),
            };
            if !grid.is_wall(cell) {
                continue;
            }

            // How far the circle's surface reaches along the axis of travel, at
            // the offset `fixed` sits at relative to this wall.
            let (cross_min, cross_max) = cell_bounds(cross);
            let Some(reach) = axis_reach(radius, fixed, cross_min, cross_max) else {
                continue;
            };

            // Only the face being approached constrains anything. A wall the
            // circle is already past on this axis is behind it, and clamping to
            // that wall's near face would drag the circle backwards across the
            // whole cell — which is what happens when a tank slides along the
            // top of a block and then tries to move further along it.
            //
            // The comparison is the circle's *current* coordinate, not its
            // target: the question is which side it started on. `SKIN` keeps a
            // circle already resting flush against the face on the near side of
            // it, so contact does not become passage.
            let (along_min, along_max) = cell_bounds(along);
            if step > 0.0 {
                let limit = along_min - reach;
                if moving <= limit + SKIN {
                    resolved = resolved.min(limit);
                }
            } else {
                let limit = along_max + reach;
                if moving >= limit - SKIN {
                    resolved = resolved.max(limit);
                }
            }
        }
    }
    resolved
}

/// How far a circle's surface extends along one axis, given how far its centre
/// sits outside a wall's span on the other.
///
/// `None` when the circle cannot touch the wall at all on this row or column.
/// Level with the face the answer is the full radius; off the end of it, the
/// answer shrinks to the corner distance, which is what rounds corners.
fn axis_reach(radius: f32, center: f32, span_min: f32, span_max: f32) -> Option<f32> {
    let offset = if center < span_min {
        span_min - center
    } else if center > span_max {
        center - span_max
    } else {
        0.0
    };
    if offset >= radius {
        None
    } else {
        Some((radius * radius - offset * offset).sqrt())
    }
}

/// Inclusive range of cell indices covering the world span `low..=high`.
fn cell_span(low: f32, high: f32) -> (i32, i32) {
    (
        (low / CELL_SIZE).floor() as i32,
        (high / CELL_SIZE).floor() as i32,
    )
}

/// World span of the cell at index `index`, on either axis.
fn cell_bounds(index: i32) -> (f32, f32) {
    let min = index as f32 * CELL_SIZE;
    (min, min + CELL_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::CELL_SIZE as C;

    /// A 5×5 room: solid border, open interior, with one wall block in the
    /// middle of the interior.
    ///
    ///  ```text
    ///  #####
    ///  #...#
    ///  #.#.#
    ///  #...#
    ///  #####
    ///  ```
    fn room() -> Grid {
        Grid::from_rows(&[
            "#####", //
            "#...#", //
            "#.#.#", //
            "#...#", //
            "#####", //
        ])
    }

    /// A corridor one cell wide running left to right along `y == 1`, opening
    /// into a side passage going up at `x == 3`.
    ///
    /// ```text
    /// #####
    /// ###.#
    /// ###.#
    /// #...#
    /// #####
    /// ```
    fn junction() -> Grid {
        Grid::from_rows(&[
            "#####", //
            "###.#", //
            "###.#", //
            "#...#", //
            "#####", //
        ])
    }

    /// Centre of cell `(x, y)` in world units.
    fn at(x: i32, y: i32) -> Vec2 {
        Vec2::new(x as f32 + 0.5, y as f32 + 0.5) * C
    }

    /// The tank's collider: comfortably inside a one-cell corridor.
    const R: f32 = 22.0;

    #[test]
    fn a_zero_move_changes_nothing_and_blocks_nothing() {
        let grid = room();
        let start = at(1, 1);
        let result = slide(&grid, start, R, Vec2::ZERO);
        assert_eq!(result.position, start);
        assert_eq!(result.blocked, BVec2::FALSE);
    }

    #[test]
    fn an_unobstructed_move_lands_exactly_where_it_was_aimed() {
        let grid = room();
        // Cell (1,1) to cell (3,1) is clear along the bottom row.
        let start = at(1, 1);
        let delta = Vec2::new(C * 2.0, 0.0);
        let result = slide(&grid, start, R, delta);
        assert!(
            (result.position - (start + delta)).length() < 1e-4,
            "got {:?}",
            result.position
        );
        assert_eq!(result.blocked, BVec2::FALSE);
    }

    #[test]
    fn driving_into_a_wall_stops_flush_against_its_face() {
        let grid = junction();
        let start = at(1, 1);
        // Far more than the two cells of corridor there is room for.
        let result = slide(&grid, start, R, Vec2::new(C * 10.0, 0.0));
        // The wall at cell (4,1) starts at x == 4 * CELL_SIZE.
        assert!(
            (result.position.x - (4.0 * C - R)).abs() < 1e-3,
            "got {}",
            result.position.x
        );
        assert!(result.blocked.x, "x should report as blocked");
        assert!(!result.blocked.y, "y was never obstructed");
        assert!(is_clear(&grid, result.position, R));
    }

    #[test]
    fn a_circle_already_touching_a_wall_cannot_push_into_it() {
        let grid = junction();
        // Exactly flush against the left wall of the corridor, which ends at
        // x == CELL_SIZE.
        let start = Vec2::new(C + R, at(1, 1).y);
        let result = slide(&grid, start, R, Vec2::new(-C, 0.0));
        assert!(
            (result.position.x - start.x).abs() < 1e-4,
            "an exact-edge contact should not move: got {}",
            result.position.x
        );
        assert!(result.blocked.x);
    }

    #[test]
    fn a_diagonal_push_into_a_flat_wall_keeps_the_tangential_component() {
        let grid = junction();
        let start = at(2, 1);
        // Down and to the right; down is into the corridor's bottom wall.
        let result = slide(&grid, start, R, Vec2::new(C * 0.5, -C * 0.5));
        assert!(
            result.position.x > start.x + C * 0.4,
            "should have slid along the wall: got {:?}",
            result.position
        );
        assert!(
            (result.position.y - (C + R)).abs() < 1e-3,
            "should rest on the wall face: got {}",
            result.position.y
        );
        assert!(result.blocked.y && !result.blocked.x);
    }

    #[test]
    fn a_diagonal_push_into_an_inside_corner_stops_on_both_axes() {
        let grid = junction();
        let start = at(1, 1);
        // Into the corner where the left wall and the bottom wall meet.
        let result = slide(&grid, start, R, Vec2::new(-C * 2.0, -C * 2.0));
        assert!(
            (result.position - Vec2::new(C + R, C + R)).length() < 1e-3,
            "got {:?}",
            result.position
        );
        assert_eq!(result.blocked, BVec2::TRUE);
        assert!(is_clear(&grid, result.position, R));
    }

    #[test]
    fn a_convex_corner_is_rounded_rather_than_caught_on() {
        let grid = junction();
        // Sitting in the corridor left of the side passage, aiming up and right
        // to turn the corner at the block in cell (2,2).
        let start = at(2, 1);
        let mut position = start;
        for _ in 0..120 {
            position = slide(&grid, position, R, Vec2::new(1.5, 1.5)).position;
        }
        // Box-versus-box resolution would have jammed on the corner of cell
        // (2,2) and never left the bottom row.
        assert!(
            position.y > 2.0 * C,
            "should have got up into the side passage: {position:?}"
        );
        assert!(is_clear(&grid, position, R));
    }

    #[test]
    fn a_circle_squeezes_between_two_walls_it_exactly_fits_through() {
        // The gap at x == 2 is one cell wide, so a radius of exactly half a cell
        // fits with nothing to spare.
        let grid = Grid::from_rows(&[
            "#####", //
            "#.#.#", //
            "#...#", //
            "#.#.#", //
            "#####", //
        ]);
        let start = at(2, 2);
        let result = slide(&grid, start, C * 0.5, Vec2::new(0.0, C));
        assert!(
            (result.position.y - start.y).abs() < 1e-3,
            "a circle that only just fits the gap cannot enter it: got {}",
            result.position.y
        );
    }

    #[test]
    fn nothing_can_leave_the_grid() {
        // A one-cell grid with no border at all: the void around it is solid.
        let grid = Grid::from_rows(&["."]);
        let start = at(0, 0);
        for direction in [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y] {
            let result = slide(&grid, start, R, direction * C * 20.0);
            assert!(
                is_clear(&grid, result.position, R),
                "{direction:?} escaped to {:?}",
                result.position
            );
        }
    }

    /// The tunnelling guard, over speeds from a crawling tank to absurd.
    ///
    /// The tank's own worst case is trivial — `HullParams::TANK` at 180 px/s
    /// covers 3 px in a 1/60 s tick — but shells in M2 are an order of magnitude
    /// faster, and nothing about the design stops that number from growing. The
    /// substep guard has to hold without callers clamping anything.
    #[test]
    fn no_speed_tunnels_through_a_wall() {
        let grid = room();
        let dt = crate::FIXED_DT;
        for speed in [180.0, 1_000.0, 5_000.0, 60_000.0, 1.0e6] {
            for direction in [
                Vec2::X,
                Vec2::NEG_X,
                Vec2::Y,
                Vec2::NEG_Y,
                Vec2::new(1.0, 1.0).normalize(),
                Vec2::new(-1.0, 0.6).normalize(),
            ] {
                // Start in each corner of the room, so the run at the interior
                // block comes from every side.
                for start in [at(1, 1), at(3, 1), at(1, 3), at(3, 3)] {
                    let result = slide(&grid, start, R, direction * speed * dt);
                    assert!(
                        is_clear(&grid, result.position, R),
                        "{speed} px/s along {direction:?} from {start:?} ended inside a wall \
                         at {:?}",
                        result.position
                    );
                }
            }
        }
    }

    #[test]
    fn substepping_does_not_change_where_an_unobstructed_move_ends_up() {
        // Wide open, and big enough that the move stays well inside it — the
        // void beyond the last cell is solid and would clamp the result.
        let grid = Grid::from_rows(&["........."; 9]);
        let start = at(2, 2);
        // Long enough to be split into many substeps.
        let delta = Vec2::new(C * 3.0, C * 2.0);
        let result = slide(&grid, start, R, delta);
        assert!(
            (result.position - (start + delta)).length() < 1e-3,
            "got {:?}, wanted {:?}",
            result.position,
            start + delta
        );
    }

    #[test]
    fn sliding_is_deterministic() {
        let grid = room();
        let run = || {
            let mut position = at(1, 1);
            for tick in 0..500 {
                let angle = tick as f32 * 0.37;
                let delta = Vec2::new(angle.cos(), angle.sin()) * 4.0;
                position = slide(&grid, position, R, delta).position;
            }
            position
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn a_long_random_walk_never_ends_up_inside_a_wall() {
        // The property that actually matters, checked against a real maze
        // rather than a hand-written fixture.
        let maze = crate::maze::generate(crate::maze::MazeParams::LEVEL_ONE, 12345);
        let mut position = maze.grid.cell_center(maze.spawn);
        assert!(is_clear(&maze.grid, position, R), "spawn is not clear");

        for tick in 0..4000 {
            let angle = tick as f32 * 2.399_963; // golden-angle turn, never repeats
            let delta = Vec2::new(angle.cos(), angle.sin()) * 3.0;
            position = slide(&maze.grid, position, R, delta).position;
            assert!(
                is_clear(&maze.grid, position, R),
                "tick {tick} ended inside a wall at {position:?}"
            );
        }
    }

    #[test]
    fn axis_reach_is_the_full_radius_level_with_a_face() {
        assert_eq!(axis_reach(10.0, 5.0, 0.0, 10.0), Some(10.0));
        assert_eq!(axis_reach(10.0, 0.0, 0.0, 10.0), Some(10.0));
        assert_eq!(axis_reach(10.0, 10.0, 0.0, 10.0), Some(10.0));
    }

    #[test]
    fn axis_reach_shrinks_past_the_end_of_a_face() {
        // 3-4-5: six units past the corner of a five-unit circle leaves four.
        let reach = axis_reach(5.0, -3.0, 0.0, 10.0).expect("still in contact");
        assert!((reach - 4.0).abs() < 1e-5, "got {reach}");
    }

    #[test]
    fn axis_reach_is_none_when_the_circle_cannot_touch_the_wall() {
        assert_eq!(axis_reach(5.0, -5.0, 0.0, 10.0), None);
        assert_eq!(axis_reach(5.0, 20.0, 0.0, 10.0), None);
    }
}
