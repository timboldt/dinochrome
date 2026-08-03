//! Turret traverse.
//!
//! The turret rotates independently of the hull. That is kept from the 1982
//! original and it is most of what makes the tank interesting to drive: where you
//! are going and where you are pointing are two decisions, and in a maze they are
//! usually different.
//!
//! Like the hull, the turret is commanded by a *direction* rather than a rate, so
//! the same code serves the arrow keys (a unit vector along whatever is held) and
//! a gamepad's right stick (a partially deflected one). Unlike the hull, partial
//! deflection does not mean a slower traverse — where you point is where it goes,
//! at the one rate the mount can manage.
//!
//! Angles are radians counter-clockwise from +X, which is what [`Vec2::to_angle`]
//! and `Vec2::from_angle` use, and are kept folded into `(-PI, PI]` so that the
//! same bearing is always the same number.

use std::f32::consts::{PI, TAU};

use glam::Vec2;

/// Tunable turret characteristics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurretParams {
    /// How fast the turret can swing, in radians per second.
    pub traverse: f32,
}

impl TurretParams {
    /// Turret tuning for the player's tank.
    ///
    /// Half a turn per second: quick enough to answer something that has just
    /// come round a corner, slow enough that turning to face the other way is a
    /// decision with a cost.
    pub const TANK: Self = Self { traverse: PI };

    /// Traverse for a factory's defence gun.
    ///
    /// Slower than a tank's, because a building should be beatable by moving. A
    /// player who circles one faster than it can follow has earned the shot.
    pub const FACTORY: Self = Self { traverse: PI * 0.6 };
}

impl Default for TurretParams {
    fn default() -> Self {
        Self::TANK
    }
}

/// Folds an angle into `(-PI, PI]`.
pub fn wrap_angle(angle: f32) -> f32 {
    let folded = angle.rem_euclid(TAU);
    if folded > PI { folded - TAU } else { folded }
}

/// The bearing an aim command points along, or `None` if it is not asking for
/// anything.
///
/// A released stick means "hold where you are", not "return to zero", so the
/// absence of a command has to be distinguishable from a command of zero degrees.
pub fn aim_angle(command: Vec2) -> Option<f32> {
    (command != Vec2::ZERO).then(|| command.to_angle())
}

/// Advances a turret angle by one tick.
///
/// Turns whichever way round is shorter, and lands exactly on `desired` rather
/// than stepping past it and hunting back and forth across it forever.
pub fn step_angle(current: f32, desired: f32, params: TurretParams, dt: f32) -> f32 {
    let error = wrap_angle(desired - current);
    let limit = params.traverse * dt;
    if error.abs() <= limit {
        wrap_angle(desired)
    } else {
        // Exactly half a turn away is a tie, and `copysign` breaks it the same
        // way every time, which is all the simulation needs of it.
        wrap_angle(current + limit.copysign(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FIXED_DT;

    const P: TurretParams = TurretParams::TANK;

    /// Runs `ticks` simulation steps aiming at a constant bearing.
    fn run(mut angle: f32, desired: f32, ticks: u32) -> f32 {
        for _ in 0..ticks {
            angle = step_angle(angle, desired, P, FIXED_DT);
        }
        angle
    }

    /// Bearings spread around the circle, including both ends of the fold.
    const BEARINGS: [f32; 8] = [0.0, 0.7, PI * 0.5, 2.5, PI, -2.5, -PI * 0.5, -0.7];

    #[test]
    fn wrap_angle_leaves_angles_already_in_range_alone() {
        for angle in [0.0, 0.5, -0.5, PI, -PI + 0.001] {
            assert!(
                (wrap_angle(angle) - angle).abs() < 1e-6,
                "{angle} came back as {}",
                wrap_angle(angle)
            );
        }
    }

    #[test]
    fn wrap_angle_folds_whole_turns_away() {
        for turns in [-3.0, -1.0, 1.0, 2.0, 7.0] {
            let folded = wrap_angle(0.75 + turns * TAU);
            assert!((folded - 0.75).abs() < 1e-4, "{turns} turns gave {folded}");
        }
    }

    #[test]
    fn wrap_angle_always_lands_in_the_documented_half_open_range() {
        // Half a degree at a time, twice round in each direction.
        for step in -1440..=1440 {
            let angle = step as f32 * TAU / 720.0;
            let folded = wrap_angle(angle);
            assert!(
                folded > -PI - 1e-6 && folded <= PI + 1e-6,
                "{angle} folded to {folded}"
            );
        }
    }

    #[test]
    fn aim_angle_reads_a_direction_as_a_bearing() {
        assert_eq!(aim_angle(Vec2::X), Some(0.0));
        assert_eq!(aim_angle(Vec2::Y), Some(PI * 0.5));
        assert_eq!(aim_angle(Vec2::NEG_Y), Some(-PI * 0.5));
        // Length does not matter, only direction.
        let partial = aim_angle(Vec2::new(0.1, 0.1)).expect("a command");
        assert!((partial - PI * 0.25).abs() < 1e-6, "got {partial}");
    }

    #[test]
    fn no_aim_command_is_not_a_command_to_aim_at_zero() {
        assert_eq!(aim_angle(Vec2::ZERO), None);
    }

    #[test]
    fn a_bearing_within_reach_is_reached_exactly() {
        // One tick of traverse is PI/60; a hair less than that is one tick away.
        let desired = 0.05;
        assert_eq!(step_angle(0.0, desired, P, FIXED_DT), desired);
        // And it stays there rather than hunting around it.
        assert_eq!(step_angle(desired, desired, P, FIXED_DT), desired);
    }

    #[test]
    fn one_tick_never_swings_further_than_the_traverse_rate() {
        let limit = P.traverse * FIXED_DT;
        for from in BEARINGS {
            for to in BEARINGS {
                let stepped = step_angle(from, to, P, FIXED_DT);
                let swung = wrap_angle(stepped - from).abs();
                assert!(
                    swung <= limit + 1e-6,
                    "{from} to {to} swung {swung}, over the {limit} it may"
                );
            }
        }
    }

    #[test]
    fn the_turret_takes_the_short_way_round_the_fold() {
        // Just short of half a turn one way to just short of half a turn the
        // other: 0.28 radians the short way, and 6.0 the long way.
        let from = 3.0;
        let to = -3.0;
        let stepped = step_angle(from, to, P, FIXED_DT);
        assert!(
            wrap_angle(stepped - from) > 0.0,
            "should have kept turning the same way through the fold, got {stepped}"
        );
        // And it gets there in the few ticks the short way needs, not the
        // hundred and fifteen the long way would.
        let arrived = run(from, to, 6);
        assert!((arrived - to).abs() < 1e-4, "got {arrived}");
    }

    #[test]
    fn every_bearing_is_reachable_from_every_other_within_half_a_turn_of_traverse() {
        // The furthest any bearing can be from any other is half a turn, and half
        // a turn at PI rad/s takes a second. So 60 ticks is the worst case with
        // nothing to spare, which is the point of checking it: a turret that took
        // the long way round would need twice that.
        for from in BEARINGS {
            for to in BEARINGS {
                let arrived = run(from, to, 61);
                assert!(
                    (wrap_angle(arrived - to)).abs() < 1e-4,
                    "{from} never got to {to}, stalled at {arrived}"
                );
            }
        }
    }

    #[test]
    fn stepping_is_deterministic() {
        assert_eq!(run(0.3, -2.9, 17), run(0.3, -2.9, 17));
    }
}
