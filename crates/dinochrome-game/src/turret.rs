//! The tank's turret: aiming it, and drawing where it points.
//!
//! The angle lives on the tank entity, because everything that reads it — the
//! gun, and in M3 a factory's line of sight — is asking about the tank rather
//! than about a sprite. The barrel is a child entity whose only job is to be
//! rotated to match, so the simulation can run with no barrel at all.
//!
//! The split follows the same rule as the hull: [`sample_aim_input`] runs once per
//! rendered frame and writes a command, and [`slew_turrets`] runs on the fixed
//! timestep and reads only that.

use bevy::prelude::*;
use bevy::sprite::Anchor;
use dinochrome_core::{FIXED_DT, hull, turret};

use crate::palette;
use crate::player::Tank;

/// Which way a turret points, in radians counter-clockwise from +X.
#[derive(Component, Debug, Default, Deref, DerefMut)]
pub struct Turret(pub f32);

/// Turret traverse tuning for this entity.
#[derive(Component, Debug, Default, Deref, DerefMut)]
pub struct Traverse(pub turret::TurretParams);

/// The direction the player wants the turret to point, magnitude 0..=1.
///
/// Zero means "no instruction", not "point at zero degrees" — a turret with
/// nothing asked of it holds its bearing.
#[derive(Component, Debug, Default, Deref, DerefMut)]
pub struct AimCommand(pub Vec2);

/// Marks the barrel sprite hanging off a turret.
#[derive(Component)]
pub struct Barrel;

/// How far the muzzle is from the tank's centre, in pixels.
///
/// A little past the hull, so a shell is visibly leaving the barrel rather than
/// appearing out of the middle of the tank.
pub const MUZZLE_OFFSET: f32 = 28.0;

/// Size of the barrel sprite, in pixels.
const BARREL_SIZE: Vec2 = Vec2::new(MUZZLE_OFFSET, 6.0);

/// Draw order for the barrel, relative to the hull it is a child of.
const Z_BARREL: f32 = 0.1;

/// The barrel sprite, as a child of the tank it belongs to.
///
/// Anchored at its left edge rather than its centre, so the child's own transform
/// needs nothing but the turret's rotation: the rectangle then grows out of the
/// tank's centre along +X, which is the bearing zero means.
pub fn barrel() -> impl Bundle {
    (
        Barrel,
        Sprite::from_color(palette::TURRET, BARREL_SIZE),
        Anchor::CENTER_LEFT,
        Transform::from_xyz(0.0, 0.0, Z_BARREL),
    )
}

/// Samples the keyboard into each turret's aim command.
///
/// Arrow keys, as an absolute bearing rather than a rotate-left/rotate-right pair.
/// It costs a little precision and buys the thing that matters: it is the same
/// control as a gamepad's right stick, so M5 can add twin-stick support without
/// a second control scheme to tune.
pub fn sample_aim_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut turrets: Query<&mut AimCommand, With<Tank>>,
) {
    let mut raw = Vec2::ZERO;
    if keys.pressed(KeyCode::ArrowUp) {
        raw.y += 1.0;
    }
    if keys.pressed(KeyCode::ArrowDown) {
        raw.y -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowLeft) {
        raw.x -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowRight) {
        raw.x += 1.0;
    }

    // Only the direction is used, but clamping keeps the command in the same
    // shape as the hull's so a gamepad can be fed into either.
    let aim = hull::clamp_drive(raw);
    for mut command in &mut turrets {
        command.0 = aim;
    }
}

/// Clears every aim command, so a turret cannot keep slewing on stale input.
pub fn clear_aim_input(mut turrets: Query<&mut AimCommand>) {
    for mut command in &mut turrets {
        command.0 = Vec2::ZERO;
    }
}

/// Swings every turret one tick toward what it has been asked to point at.
pub fn slew_turrets(mut turrets: Query<(&mut Turret, &Traverse, &AimCommand)>) {
    for (mut angle, params, command) in &mut turrets {
        let Some(desired) = turret::aim_angle(command.0) else {
            continue;
        };
        angle.0 = turret::step_angle(angle.0, desired, params.0, FIXED_DT);
    }
}

/// Points each barrel sprite along its turret's bearing.
///
/// Presentation only: the barrel is a picture of the angle, never the other way
/// round.
pub fn sync_barrels(
    turrets: Query<&Turret>,
    mut barrels: Query<(&ChildOf, &mut Transform), With<Barrel>>,
) {
    for (parent, mut transform) in &mut barrels {
        if let Ok(turret) = turrets.get(parent.parent()) {
            transform.rotation = Quat::from_rotation_z(turret.0);
        }
    }
}
