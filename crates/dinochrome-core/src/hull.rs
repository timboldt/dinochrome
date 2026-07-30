//! Hover-tank hull kinematics.
//!
//! The hull is driven by a direction vector rather than a target velocity, so
//! the same code serves keyboard input (a unit vector along the pressed keys)
//! and an analog stick (a partially deflected vector). Velocity chases the
//! commanded velocity at a bounded rate, which is what gives the tank its
//! weight; it never overshoots, so releasing the controls settles at exactly
//! zero instead of jittering around it.
//!
//! This module only decides how fast the hull *wants* to go. Turning that into a
//! new position is [`crate::collision::slide`]'s job, because the answer depends
//! on the maze.

use glam::Vec2;

/// Tunable hull characteristics, in world units (pixels) per second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HullParams {
    /// Speed at full deflection.
    pub max_speed: f32,
    /// Rate at which velocity chases the commanded velocity while driving.
    pub accel: f32,
    /// Rate at which velocity decays toward zero when the controls are released.
    pub brake: f32,
}

impl HullParams {
    /// Hull tuning for the player's tank.
    pub const TANK: Self = Self {
        max_speed: 180.0,
        accel: 720.0,
        brake: 960.0,
    };
}

impl Default for HullParams {
    fn default() -> Self {
        Self::TANK
    }
}

/// Normalizes a raw drive command to a magnitude of at most 1.
///
/// Keyboard input arrives as a sum of axis unit vectors, so holding two keys
/// yields a length of `sqrt(2)`; without this, diagonal movement would be 41%
/// faster. Analog input already inside the unit circle is passed through
/// unchanged so partial deflection still means partial speed.
pub fn clamp_drive(input: Vec2) -> Vec2 {
    let len_sq = input.length_squared();
    if len_sq > 1.0 {
        input / len_sq.sqrt()
    } else {
        input
    }
}

/// Advances hull velocity by one tick.
///
/// `drive` is a command direction; callers are expected to have passed it
/// through [`clamp_drive`] already.
pub fn step_velocity(vel: Vec2, drive: Vec2, params: HullParams, dt: f32) -> Vec2 {
    let target = drive * params.max_speed;
    let rate = if drive == Vec2::ZERO {
        params.brake
    } else {
        params.accel
    };
    move_toward(vel, target, rate * dt)
}

/// Moves `from` toward `to` by at most `max_delta`, landing exactly on `to`
/// rather than overshooting it.
fn move_toward(from: Vec2, to: Vec2, max_delta: f32) -> Vec2 {
    let delta = to - from;
    let dist_sq = delta.length_squared();
    if dist_sq <= max_delta * max_delta {
        to
    } else {
        from + delta * (max_delta / dist_sq.sqrt())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FIXED_DT;

    const P: HullParams = HullParams::TANK;

    /// Runs `ticks` simulation steps with a constant drive command.
    fn run(mut vel: Vec2, drive: Vec2, ticks: u32) -> Vec2 {
        for _ in 0..ticks {
            vel = step_velocity(vel, drive, P, FIXED_DT);
        }
        vel
    }

    #[test]
    fn clamp_drive_leaves_analog_input_inside_the_unit_circle_alone() {
        for input in [Vec2::ZERO, Vec2::new(0.5, 0.0), Vec2::new(0.3, -0.4)] {
            assert_eq!(clamp_drive(input), input, "input {input:?}");
        }
    }

    #[test]
    fn clamp_drive_normalizes_diagonal_keyboard_input() {
        let clamped = clamp_drive(Vec2::new(1.0, 1.0));
        assert!((clamped.length() - 1.0).abs() < 1e-6, "got {clamped:?}");
        // Direction is preserved: still exactly 45 degrees.
        assert!((clamped.x - clamped.y).abs() < 1e-6, "got {clamped:?}");
    }

    #[test]
    fn diagonal_travel_is_not_faster_than_axis_travel() {
        let straight = run(Vec2::ZERO, clamp_drive(Vec2::X), 240).length();
        let diagonal = run(Vec2::ZERO, clamp_drive(Vec2::new(1.0, 1.0)), 240).length();
        assert!(
            (straight - diagonal).abs() < 1e-3,
            "straight {straight} vs diagonal {diagonal}"
        );
    }

    #[test]
    fn full_deflection_converges_on_max_speed_and_stays_there() {
        let vel = run(Vec2::ZERO, Vec2::X, 240);
        assert!(
            (vel.x - P.max_speed).abs() < 1e-3,
            "expected {} got {}",
            P.max_speed,
            vel.x
        );
        // Already at target: another tick must not push past it.
        assert_eq!(step_velocity(vel, Vec2::X, P, FIXED_DT), vel);
    }

    #[test]
    fn partial_deflection_converges_on_a_proportional_speed() {
        let vel = run(Vec2::ZERO, Vec2::new(0.5, 0.0), 240);
        assert!(
            (vel.x - P.max_speed * 0.5).abs() < 1e-3,
            "expected {} got {}",
            P.max_speed * 0.5,
            vel.x
        );
    }

    #[test]
    fn releasing_the_controls_settles_at_exactly_zero() {
        let moving = run(Vec2::ZERO, Vec2::X, 240);
        assert!(moving.length() > 0.0);
        // Braking never overshoots into a reversed velocity, and reaches a hard
        // zero so a parked tank does not drift.
        let stopped = run(moving, Vec2::ZERO, 240);
        assert_eq!(stopped, Vec2::ZERO);
    }

    #[test]
    fn braking_takes_at_least_one_tick_from_full_speed() {
        let moving = Vec2::new(P.max_speed, 0.0);
        let after_one = step_velocity(moving, Vec2::ZERO, P, FIXED_DT);
        assert!(
            after_one.x > 0.0 && after_one.x < moving.x,
            "expected a partial slowdown, got {after_one:?}"
        );
    }

    #[test]
    fn a_reversed_command_passes_cleanly_through_zero() {
        let moving = Vec2::new(P.max_speed, 0.0);
        let reversed = run(moving, Vec2::NEG_X, 240);
        assert!(
            (reversed.x + P.max_speed).abs() < 1e-3,
            "expected {} got {}",
            -P.max_speed,
            reversed.x
        );
    }

    #[test]
    fn stepping_is_deterministic() {
        let drive = clamp_drive(Vec2::new(1.0, -0.25));
        assert_eq!(run(Vec2::ZERO, drive, 137), run(Vec2::ZERO, drive, 137));
    }
}
