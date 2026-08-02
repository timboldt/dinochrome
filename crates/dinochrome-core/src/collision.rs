//! Moving a circle through the maze grid.
//!
//! There are two ways to move, and the game needs both. A tank *slides*: it is
//! resolved one axis at a time and keeps whatever part of its move a wall did not
//! refuse. A shell *sweeps*: it goes in a straight line until something stops it,
//! at which point it is done travelling forever. [`slide`] and [`sweep`] are those
//! two.
//!
//! # Sliding
//!
//! Entities are circles; walls are axis-aligned boxes. A move is resolved one
//! axis at a time — X first with the old Y, then Y with the new X — which is
//! what gives wall sliding for free: a diagonal push into a wall loses the
//! component that is blocked and keeps the one that is not, with no special case
//! for it anywhere.
//!
//! The boxes are not the grid cells. A wall cell is solid only along a thin bar
//! through its middle ([`Grid::wall_boxes`]), so the grid is used to decide
//! *which* cells to look at and the boxes decide what is solid once there. That
//! keeps the broad phase the same cheap integer cell walk it always was.
//!
//! Resolution against a single wall is exact circle geometry rather than a box
//! approximation. Where the circle's centre is level with a wall face the
//! constraint is that face; where it is off the end of the face, the constraint
//! is the wall's corner, and the circle is allowed to come nearer along the axis
//! of travel by exactly as much as the Pythagorean slack allows. That is what
//! lets a tank round the corner of a corridor mouth instead of catching on it.
//!
//! Buildings are not in the grid — a factory stands in an open cell — so
//! [`slide_around`] takes them as a slice of [`Blocker`]s and resolves them as
//! circles.
//!
//! # Tunnelling
//!
//! Long slides are split into substeps short enough that the swept region always
//! overlaps every wall in between, so callers do not have to clamp velocities.
//! See [`MAX_SUBSTEP`]. Sweeps do not substep at all: they solve for the impact
//! point directly, so a shell is exact at any speed.

use glam::{BVec2, IVec2, Vec2};

use crate::grid::{CELL_SIZE, Grid, WALL_THICKNESS};

/// Longest distance resolved in one substep, in world units.
///
/// Half a wall's thickness, so a substep this short cannot straddle one: the
/// swept span always intersects any wall it crosses, and the clamp sees it. It
/// is well under a blocker's radius too, so nothing steps clean over a building
/// either.
///
/// Tied to [`WALL_THICKNESS`] rather than to the cell, because it is the solid
/// part a move must not skip and walls are a quarter of a cell thick. The tank
/// covers three pixels in a tick, so this costs nothing in practice; it is the
/// guard that lets callers not think about velocity at all.
pub const MAX_SUBSTEP: f32 = WALL_THICKNESS * 0.5;

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

/// A round obstacle that is not part of the grid.
///
/// Drone factories are buildings standing in open cells: the maze does not know
/// they are there, so whatever has to drive around one is handed them as these.
/// A blocker is immovable — a slide is pushed out of it, never the other way
/// round.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blocker {
    /// World-space centre.
    pub center: Vec2,
    /// Radius, in world units.
    pub radius: f32,
}

/// Where a move ended up, and which axes were stopped short.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slide {
    /// The resolved position.
    pub position: Vec2,
    /// Per-axis: true if something clamped this axis at any point during the move.
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
    slide_around(grid, position, radius, delta, &[])
}

/// [`slide`], but also refusing to pass through a set of round obstacles.
pub fn slide_around(
    grid: &Grid,
    position: Vec2,
    radius: f32,
    delta: Vec2,
    blockers: &[Blocker],
) -> Slide {
    let distance = delta.length();
    let mut result = Slide {
        position,
        blocked: BVec2::FALSE,
    };
    if distance == 0.0 {
        return result;
    }

    // `ceil` of a positive quotient is at least 1, so there is always a step. A
    // substep is shorter than a blocker's radius as well as than a wall's
    // thickness, so nothing steps clean over a building either.
    let steps = (distance / MAX_SUBSTEP).ceil() as u32;
    let step = delta / steps as f32;

    for _ in 0..steps {
        let from = result.position;
        let mut to = from;
        to.x = resolve(grid, to, radius, step.x, Axis::X);
        to.y = resolve(grid, to, radius, step.y, Axis::Y);

        // A blocker is round, so the way out of one is along the line from its
        // centre. That keeps whatever part of the move was tangential to it,
        // which is what lets a tank scrape around a factory instead of stopping
        // dead on its side. The push can only ever be *away* from the blocker, so
        // the one thing it can go wrong against is a wall on the far side — and
        // an entity wedged between the two has nowhere to be, so the substep is
        // refused outright and nothing further is attempted along this heading.
        let pushed = push_out(to, radius, blockers);
        if pushed != to && !is_clear(grid, pushed, radius) {
            result.blocked = BVec2::TRUE;
            break;
        }
        result.position = pushed;

        // Where the substep was aiming, had nothing been in the way. `SKIN`
        // rather than an exact comparison because parking flush against a face
        // is only exact to within `f32` noise at maze-sized coordinates.
        let aimed = from + step;
        result.blocked.x |= (result.position.x - aimed.x).abs() > SKIN;
        result.blocked.y |= (result.position.y - aimed.y).abs() > SKIN;
    }
    result
}

/// Where a straight, non-sliding move ended, and what stopped it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sweep {
    /// Fraction of the requested `delta` actually travelled, in `0.0..=1.0`.
    pub travel: f32,
    /// The position reached.
    pub position: Vec2,
    /// The wall cell struck, or `None` if the move ran its full length.
    pub hit: Option<IVec2>,
}

/// Moves a circle of `radius` from `from` by `delta` until a wall stops it.
///
/// This is what a shell does: no sliding, and no substepping either — the impact
/// point is solved for rather than crept up on, so it is exact however fast the
/// thing is going and there is nothing for it to tunnel through. `travel` is
/// reported alongside the position so a caller can compare a wall hit against
/// whatever else it is testing (see [`hit_circle`]) and take the nearer one.
pub fn sweep(grid: &Grid, from: Vec2, radius: f32, delta: Vec2) -> Sweep {
    let mut travel = 1.0;
    let mut hit = None;

    if delta != Vec2::ZERO {
        // Every cell the swept circle could touch, from either endpoint outwards.
        let to = from + delta;
        let low = grid.cell_at(from.min(to) - Vec2::splat(radius));
        let high = grid.cell_at(from.max(to) + Vec2::splat(radius));
        for y in low.y..=high.y {
            for x in low.x..=high.x {
                let cell = IVec2::new(x, y);
                for wall in grid.wall_boxes(cell).iter() {
                    if let Some(t) = hit_box(from, delta, radius, wall.min, wall.max)
                        && t < travel
                    {
                        travel = t;
                        hit = Some(cell);
                    }
                }
            }
        }
    }

    Sweep {
        travel,
        position: from + delta * travel,
        hit,
    }
}

/// Earliest fraction of `delta` at which a circle of `radius` starting at `from`
/// touches a stationary circle of `target_radius` at `target`.
///
/// `None` if it never does within the move. `Some(0.0)` if the two already
/// overlap, which is a real answer rather than a degenerate one: a shell spawned
/// inside its target has hit it.
pub fn hit_circle(
    from: Vec2,
    delta: Vec2,
    radius: f32,
    target: Vec2,
    target_radius: f32,
) -> Option<f32> {
    // Solving |offset + delta·t| == clearance for the smaller root of the
    // quadratic in t — the larger one is where it would come back out the far
    // side.
    let clearance = radius + target_radius;
    let offset = from - target;
    let c = offset.length_squared() - clearance * clearance;
    if c <= 0.0 {
        return Some(0.0);
    }
    let a = delta.length_squared();
    if a == 0.0 {
        return None;
    }
    let b = 2.0 * offset.dot(delta);
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return None;
    }
    let t = (-b - discriminant.sqrt()) / (2.0 * a);
    (0.0..=1.0).contains(&t).then_some(t)
}

/// Earliest fraction of `delta` at which a circle of `radius` starting at `from`
/// touches the box `min..=max`.
///
/// The circle is shrunk to a point and the box grown to compensate — their
/// Minkowski sum, which is the box inflated by `radius` with its corners rounded
/// off to that radius — so this reduces to a ray test. A slab test finds where the
/// ray enters the inflated *square*; if that happens level with one of the faces
/// it is the answer, and if it happens off the end of both then the real contact
/// is against a rounded corner, which is a ray/circle solve.
fn hit_box(from: Vec2, delta: Vec2, radius: f32, min: Vec2, max: Vec2) -> Option<f32> {
    let grown_min = min - Vec2::splat(radius);
    let grown_max = max + Vec2::splat(radius);

    // Clamped to the move: `enter` starting at zero means "already inside" comes
    // back as contact at once, and `exit` starting at one means a wall beyond the
    // end of the move is not a wall this move hits.
    let mut enter = 0.0f32;
    let mut exit = 1.0f32;
    for axis in 0..2 {
        let speed = delta[axis];
        let start = from[axis];
        if speed == 0.0 {
            // Never crosses this pair of slab faces: either it is between them
            // for the whole move, or it never is at all.
            if start < grown_min[axis] || start > grown_max[axis] {
                return None;
            }
            continue;
        }
        let near = (grown_min[axis] - start) / speed;
        let far = (grown_max[axis] - start) / speed;
        enter = enter.max(near.min(far));
        exit = exit.min(near.max(far));
        if enter > exit {
            return None;
        }
    }

    let touch = from + delta * enter;
    let past_x = touch.x < min.x || touch.x > max.x;
    let past_y = touch.y < min.y || touch.y > max.y;
    if !(past_x && past_y) {
        return Some(enter);
    }
    // Off the end of a face on both axes, so the nearest part of the box is the
    // corner `touch` clamps onto — and the inflated square's sharp corner is not
    // where a circle actually makes contact with it.
    hit_circle(from, delta, radius, touch.clamp(min, max), 0.0)
}

/// Moves a circle out of every blocker it overlaps, along the line of centres.
///
/// Blockers are placed far enough apart that a circle cannot be inside two at
/// once (see `maze::generate`), so one pass settles it.
fn push_out(position: Vec2, radius: f32, blockers: &[Blocker]) -> Vec2 {
    let mut position = position;
    for blocker in blockers {
        let clearance = radius + blocker.radius;
        let offset = position - blocker.center;
        let distance = offset.length();
        if distance >= clearance {
            continue;
        }
        // Exactly on the centre leaves no line to push along, so any direction
        // will do. It takes something spawning dead on top of a building.
        let out = if distance > 0.0 {
            offset / distance
        } else {
            Vec2::Y
        };
        position = blocker.center + out * clearance;
    }
    position
}

/// True if a circle of `radius` at `position` overlaps no wall.
pub fn is_clear(grid: &Grid, position: Vec2, radius: f32) -> bool {
    let low = grid.cell_at(position - Vec2::splat(radius));
    let high = grid.cell_at(position + Vec2::splat(radius));
    let allowed = (radius - SKIN).max(0.0);
    for y in low.y..=high.y {
        for x in low.x..=high.x {
            for wall in grid.wall_boxes(IVec2::new(x, y)).iter() {
                let nearest = wall.nearest(position);
                if position.distance_squared(nearest) < allowed * allowed {
                    return false;
                }
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
            // The cell says where to look; the boxes say what is actually solid
            // there. A cell contributes at most two, and each is constrained
            // against independently — their union is the wall's real shape, and
            // taking the tightest clamp over them is that union exactly.
            for wall in grid.wall_boxes(cell).iter() {
                let (along_min, along_max, cross_min, cross_max) = match axis {
                    Axis::X => (wall.min.x, wall.max.x, wall.min.y, wall.max.y),
                    Axis::Y => (wall.min.y, wall.max.y, wall.min.x, wall.max.x),
                };

                // How far the circle's surface reaches along the axis of travel,
                // at the offset `fixed` sits at relative to this wall.
                let Some(reach) = axis_reach(radius, fixed, cross_min, cross_max) else {
                    continue;
                };

                // Only the face being approached constrains anything. A wall the
                // circle is already past on this axis is behind it, and clamping
                // to that wall's near face would drag the circle backwards
                // across it — which is what happens when a tank slides along the
                // top of a block and then tries to move further along it.
                //
                // The comparison is the circle's *current* coordinate, not its
                // target: the question is which side it started on. `SKIN` keeps
                // a circle already resting flush against the face on the near
                // side of it, so contact does not become passage.
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

    /// The tank's collider: comfortably inside a corridor.
    const R: f32 = 22.0;

    /// How far a wall line reaches either side of its cell centre.
    const HALF_WALL: f32 = WALL_THICKNESS * 0.5;

    /// World coordinate of the low face of the wall line in cell `index`.
    ///
    /// Walls run down the middle of their cells, so a wall's face is *not* the
    /// cell boundary — it is half a thickness off the cell centre. Every test
    /// that wants to know where a circle comes to rest goes through these.
    fn face_min(index: i32) -> f32 {
        (index as f32 + 0.5) * C - HALF_WALL
    }

    /// World coordinate of the high face of the wall line in cell `index`.
    fn face_max(index: i32) -> f32 {
        (index as f32 + 0.5) * C + HALF_WALL
    }

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
        // The wall line in cell (4,1) presents its low face half a thickness
        // short of the cell centre — not at the cell boundary.
        assert!(
            (result.position.x - (face_min(4) - R)).abs() < 1e-3,
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
        // Exactly flush against the left wall of the corridor, whose face is
        // half a thickness past the centre of cell column 0.
        let start = Vec2::new(face_max(0) + R, at(1, 1).y);
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
        // Down and to the right; down is into the corridor's bottom wall. The
        // drop has to clear half a cell more than it used to: the wall line sits
        // in the middle of the border row, not along its inner edge.
        let result = slide(&grid, start, R, Vec2::new(C, -C));
        assert!(
            result.position.x > start.x + C * 0.4,
            "should have slid along the wall: got {:?}",
            result.position
        );
        assert!(
            (result.position.y - (face_max(0) + R)).abs() < 1e-3,
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
        let rest = face_max(0) + R;
        assert!(
            (result.position - Vec2::splat(rest)).length() < 1e-3,
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
    fn a_circle_that_exactly_fits_a_corridor_runs_down_it_without_scraping() {
        // Wall lines down cell columns 1 and 3. Because the lines are thin and
        // sit at the cell centres, the corridor between them is a whole cell
        // wider than the single open column suggests.
        let grid = Grid::from_rows(&[
            "#####", //
            "##.##", //
            "##.##", //
            "##.##", //
            "#####", //
        ]);
        let free = face_min(3) - face_max(1);
        assert_eq!(free, 2.0 * C - WALL_THICKNESS, "corridor free width");

        // A circle of exactly half that width fits with nothing to spare, and
        // "nothing to spare" has to mean it still passes.
        let start = at(2, 1);
        let result = slide(&grid, start, free * 0.5, Vec2::new(0.0, C));
        assert!(
            (result.position.y - (start.y + C)).abs() < 1e-3,
            "an exact fit should travel its whole move: got {}",
            result.position.y
        );
        assert_eq!(
            result.blocked,
            BVec2::FALSE,
            "an exact fit should not scrape"
        );
    }

    #[test]
    fn a_circle_too_wide_for_a_corridor_jams_in_its_mouth() {
        let grid = Grid::from_rows(&[
            "#####", //
            "##.##", //
            "##.##", //
            "##.##", //
            "#####", //
        ]);
        let start = at(2, 1);
        let too_fat = (face_min(3) - face_max(1)) * 0.5 + 1.0;
        let result = slide(&grid, start, too_fat, Vec2::new(0.0, C));
        assert!(
            result.position.y < start.y + C - 1.0,
            "should have wedged rather than driven through: got {}",
            result.position.y
        );
        assert!(result.blocked.y);
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
    /// Nothing that slides is currently fast enough for this to be interesting —
    /// `HullParams::TANK` at 180 px/s covers 3 px in a 1/60 s tick — but the drones
    /// M3 brings are faster than the tank and nothing about the design stops that
    /// number from growing. The substep guard has to hold without callers clamping
    /// anything. (Shells are faster still, but they [`sweep`] rather than slide;
    /// `no_sweep_speed_gets_through_a_wall` is their half of this.)
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
    fn a_sweep_with_nothing_in_the_way_runs_its_full_length() {
        let grid = room();
        let from = at(1, 1);
        let delta = Vec2::new(C * 2.0, 0.0);
        let result = sweep(&grid, from, 4.0, delta);
        assert_eq!(result.travel, 1.0);
        assert_eq!(result.hit, None);
        assert!((result.position - (from + delta)).length() < 1e-3);
    }

    #[test]
    fn a_zero_length_sweep_hits_nothing_even_from_against_a_wall() {
        let grid = junction();
        let from = Vec2::new(C + 4.0, at(1, 1).y);
        let result = sweep(&grid, from, 4.0, Vec2::ZERO);
        assert_eq!(result.travel, 1.0);
        assert_eq!(result.position, from);
        assert_eq!(result.hit, None);
    }

    #[test]
    fn a_sweep_stops_flush_against_the_wall_face_it_hit() {
        let grid = junction();
        let radius = 4.0;
        let from = at(1, 1);
        // Far past the end of the corridor, which is walled at cell (4,1).
        let result = sweep(&grid, from, radius, Vec2::new(C * 10.0, 0.0));
        assert_eq!(result.hit, Some(IVec2::new(4, 1)));
        assert!(
            (result.position.x - (face_min(4) - radius)).abs() < 1e-3,
            "got {}",
            result.position.x
        );
        // Flush is contact, not overlap.
        assert!(is_clear(&grid, result.position, radius));
    }

    #[test]
    fn a_sweep_reports_the_first_wall_it_meets_and_not_a_later_one() {
        // Two walls in a row along the sweep: the near one has to win.
        let grid = Grid::from_rows(&[
            "#####", //
            "#..##", //
            "#####", //
        ]);
        let result = sweep(&grid, at(1, 1), 4.0, Vec2::new(C * 4.0, 0.0));
        assert_eq!(result.hit, Some(IVec2::new(3, 1)));
    }

    #[test]
    fn a_sweep_grazing_a_convex_corner_stops_on_the_corner_and_not_the_face() {
        // The block in cell (2,2) has nothing to join onto, so it is a lone post
        // one thickness square about the cell centre. This passes 4 px over its
        // top-left corner. A square-cornered test would have clamped to the
        // post's left face, a radius too early.
        let grid = room();
        let radius = 6.0;
        let corner = Vec2::new(face_min(2), face_max(2));
        let from = Vec2::new(C * 1.5, corner.y + 4.0);
        let result = sweep(&grid, from, radius, Vec2::new(C * 2.0, 0.0));
        assert_eq!(result.hit, Some(IVec2::new(2, 2)));
        // Contact is with the corner point, so the centre ends up exactly
        // `radius` from it rather than `radius` short of the face.
        let touch = result.position.distance(corner);
        assert!((touch - radius).abs() < 1e-3, "corner distance {touch}");
        assert!(
            result.position.x > corner.x - radius + 1.0,
            "a flat-face clamp would have stopped at x == {}: got {:?}",
            corner.x - radius,
            result.position
        );
    }

    #[test]
    fn a_sweep_that_only_just_misses_a_corner_carries_straight_on() {
        let grid = room();
        let radius = 6.0;
        // A hair further over the same corner than the radius reaches.
        let from = Vec2::new(C * 1.5, face_max(2) + radius + 0.5);
        let result = sweep(&grid, from, radius, Vec2::new(C * 2.0, 0.0));
        assert_eq!(result.hit, None, "landed at {:?}", result.position);
    }

    #[test]
    fn a_sweep_starting_inside_a_wall_goes_nowhere() {
        let grid = room();
        let inside = at(2, 2);
        let result = sweep(&grid, inside, 4.0, Vec2::new(C, 0.0));
        assert_eq!(result.travel, 0.0);
        assert_eq!(result.position, inside);
        assert_eq!(result.hit, Some(IVec2::new(2, 2)));
    }

    #[test]
    fn no_sweep_speed_gets_through_a_wall() {
        // The whole point of solving rather than stepping: a shell can be as fast
        // as the design ever wants without a clamp anywhere.
        let grid = room();
        let radius = 4.0;
        for speed in [640.0, 5_000.0, 100_000.0, 1.0e7] {
            for direction in [
                Vec2::X,
                Vec2::NEG_X,
                Vec2::Y,
                Vec2::NEG_Y,
                Vec2::new(1.0, 1.0).normalize(),
                Vec2::new(-0.3, 1.0).normalize(),
            ] {
                for start in [at(1, 1), at(3, 1), at(1, 3), at(3, 3)] {
                    let result = sweep(&grid, start, radius, direction * speed * crate::FIXED_DT);
                    assert!(
                        is_clear(&grid, result.position, radius),
                        "{speed} px/s along {direction:?} from {start:?} ended inside a wall at \
                         {:?}",
                        result.position
                    );
                }
            }
        }
    }

    #[test]
    fn a_sweep_across_a_whole_maze_always_ends_up_somewhere_legal() {
        let maze = crate::maze::generate(crate::maze::MazeParams::LEVEL_ONE, 20260729);
        let radius = 4.0;
        let from = maze.grid.cell_center(maze.spawn);
        for step in 0..360 {
            let angle = step as f32 * std::f32::consts::TAU / 360.0;
            let delta = Vec2::new(angle.cos(), angle.sin()) * 4000.0;
            let result = sweep(&maze.grid, from, radius, delta);
            assert!(
                is_clear(&maze.grid, result.position, radius),
                "bearing {step} ended inside a wall at {:?}",
                result.position
            );
            assert!(
                result.hit.is_some(),
                "a maze is closed, so a 4000 px sweep has to hit something"
            );
        }
    }

    #[test]
    fn hit_circle_finds_the_moment_of_contact_and_not_of_overlap() {
        // Head-on: a radius-2 circle 12 away from a radius-2 target touches after
        // 8 of the 10 units of travel.
        let t = hit_circle(
            Vec2::ZERO,
            Vec2::new(10.0, 0.0),
            2.0,
            Vec2::new(12.0, 0.0),
            2.0,
        )
        .expect("should connect");
        assert!((t - 0.8).abs() < 1e-5, "got {t}");
    }

    #[test]
    fn hit_circle_reports_an_existing_overlap_as_contact_at_once() {
        let t = hit_circle(
            Vec2::ZERO,
            Vec2::new(10.0, 0.0),
            4.0,
            Vec2::new(3.0, 0.0),
            4.0,
        );
        assert_eq!(t, Some(0.0));
    }

    #[test]
    fn hit_circle_misses_what_it_stops_short_of_or_passes_beside() {
        // Stops short: 20 away, only 10 units of travel.
        assert_eq!(
            hit_circle(
                Vec2::ZERO,
                Vec2::new(10.0, 0.0),
                1.0,
                Vec2::new(20.0, 0.0),
                1.0
            ),
            None
        );
        // Passes beside: offset by more than the two radii together.
        assert_eq!(
            hit_circle(
                Vec2::ZERO,
                Vec2::new(40.0, 0.0),
                1.0,
                Vec2::new(20.0, 5.0),
                1.0
            ),
            None
        );
        // Moving away from something it is already clear of.
        assert_eq!(
            hit_circle(
                Vec2::ZERO,
                Vec2::new(-40.0, 0.0),
                1.0,
                Vec2::new(20.0, 0.0),
                1.0
            ),
            None
        );
    }

    #[test]
    fn hit_circle_on_a_stationary_circle_is_a_contact_test() {
        let touching = hit_circle(Vec2::ZERO, Vec2::ZERO, 2.0, Vec2::new(3.0, 0.0), 2.0);
        assert_eq!(touching, Some(0.0));
        let apart = hit_circle(Vec2::ZERO, Vec2::ZERO, 2.0, Vec2::new(9.0, 0.0), 2.0);
        assert_eq!(apart, None);
    }

    /// A factory-sized blocker in the middle of the open arena.
    fn blocker(x: i32, y: i32) -> Blocker {
        Blocker {
            center: at(x, y),
            radius: 22.0,
        }
    }

    /// An open room with no interior walls, so only blockers are in the way.
    fn open_room() -> Grid {
        Grid::from_rows(&["......."; 7])
    }

    #[test]
    fn a_slide_with_no_blockers_is_the_plain_slide() {
        let grid = room();
        let start = at(1, 1);
        let delta = Vec2::new(C * 0.7, -C * 0.7);
        assert_eq!(
            slide_around(&grid, start, R, delta, &[]),
            slide(&grid, start, R, delta)
        );
    }

    #[test]
    fn a_blocker_stops_a_tank_driving_straight_into_it() {
        let grid = open_room();
        let obstacle = blocker(3, 3);
        let start = at(1, 3);
        let mut position = start;
        for _ in 0..200 {
            position = slide_around(&grid, position, R, Vec2::new(3.0, 0.0), &[obstacle]).position;
        }
        let gap = position.distance(obstacle.center);
        assert!(
            (gap - (R + obstacle.radius)).abs() < 1e-2,
            "should be resting against the building, got a gap of {gap}"
        );
    }

    #[test]
    fn driving_into_a_blocker_reports_the_axis_as_blocked() {
        let grid = open_room();
        let obstacle = blocker(3, 3);
        // Already flush against its left side.
        let start = obstacle.center - Vec2::new(R + obstacle.radius, 0.0);
        let result = slide_around(&grid, start, R, Vec2::new(3.0, 0.0), &[obstacle]);
        assert!(result.blocked.x, "a banked velocity would launch the tank");
        assert!(!result.blocked.y);
    }

    #[test]
    fn a_tank_scrapes_around_a_blocker_instead_of_stopping_on_it() {
        let grid = open_room();
        let obstacle = blocker(3, 3);
        // Coming in slightly off centre, which is what makes it a graze.
        let start = at(1, 3) + Vec2::new(0.0, 6.0);
        let mut position = start;
        for _ in 0..400 {
            position = slide_around(&grid, position, R, Vec2::new(3.0, 0.0), &[obstacle]).position;
            assert!(
                position.distance(obstacle.center) > R + obstacle.radius - SKIN,
                "ended up inside the building at {position:?}"
            );
        }
        assert!(
            position.x > obstacle.center.x + obstacle.radius,
            "should have got past it rather than parking on it: {position:?}"
        );
    }

    #[test]
    fn nothing_tunnels_through_a_blocker_however_fast_it_is_going() {
        let grid = open_room();
        let obstacle = blocker(3, 3);
        let start = at(1, 3);
        for speed in [180.0, 1_000.0, 20_000.0, 1.0e6] {
            let delta = Vec2::new(speed * crate::FIXED_DT, 0.0);
            let result = slide_around(&grid, start, R, delta, &[obstacle]);
            assert!(
                result.position.x <= obstacle.center.x,
                "{speed} px/s stepped clean over the building to {:?}",
                result.position
            );
        }
    }

    #[test]
    fn a_tank_wedged_between_a_blocker_and_a_wall_stops_rather_than_clipping() {
        // A one-cell corridor with a building in it: the gap either side is 10 px
        // and the tank needs 20, so there is no way past and no way through.
        let grid = Grid::from_rows(&[
            "#####", //
            "#...#", //
            "#####", //
        ]);
        let obstacle = blocker(3, 1);
        let start = at(1, 1);
        let mut position = start;
        for _ in 0..300 {
            position = slide_around(&grid, position, R, Vec2::new(3.0, 0.0), &[obstacle]).position;
            assert!(
                is_clear(&grid, position, R),
                "squeezed into the wall at {position:?}"
            );
            assert!(
                position.distance(obstacle.center) > R + obstacle.radius - SKIN,
                "squeezed into the building at {position:?}"
            );
        }
    }

    #[test]
    fn a_blocker_never_pushes_anything_out_of_the_maze() {
        // Flush against the bottom wall with a building directly above: the push
        // out of the building must not become a push through the floor.
        let grid = Grid::from_rows(&[
            "#####", //
            "#...#", //
            "#...#", //
            "#####", //
        ]);
        let obstacle = Blocker {
            center: at(2, 2) - Vec2::new(0.0, 8.0),
            radius: 22.0,
        };
        let mut position = Vec2::new(at(2, 1).x, C + R);
        for _ in 0..200 {
            position = slide_around(&grid, position, R, Vec2::new(0.0, 3.0), &[obstacle]).position;
            assert!(
                is_clear(&grid, position, R),
                "pushed through the floor to {position:?}"
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
