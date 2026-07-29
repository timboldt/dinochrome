//! The player's tank: spawning, input sampling, and movement.
//!
//! The split here is the one the whole project follows: input sampling runs in
//! `Update` (once per rendered frame, because that is when the OS gives us key
//! state), and writes a *command* onto the tank. Movement runs in `FixedUpdate`
//! and reads only that command, so the simulation advances identically no matter
//! what the frame rate is. No simulation system reads `Time::delta`.

use bevy::prelude::*;
use dinochrome_core::{FIXED_DT, hull};

/// Marks the player-controlled tank.
#[derive(Component, Debug, Default)]
pub struct Tank;

/// Current world-space velocity, in pixels per second.
#[derive(Component, Debug, Default, Deref, DerefMut)]
pub struct Velocity(pub Vec2);

/// The drive direction the player is asking for, magnitude 0..=1.
#[derive(Component, Debug, Default, Deref, DerefMut)]
pub struct DriveCommand(pub Vec2);

/// Hull tuning for this entity.
#[derive(Component, Debug, Default, Deref, DerefMut)]
pub struct Hull(pub hull::HullParams);

/// Side length of the placeholder tank rectangle, in pixels.
///
/// Sized against the 64 px maze cell from the design so M1's maze drops in
/// around a tank that already looks right.
const TANK_SIZE: Vec2 = Vec2::new(40.0, 48.0);

/// Creates the tank at the world origin.
///
/// M0 has no maze, so there is no spawn point to speak of; M1 replaces this with
/// placement on a known-open cell.
pub fn spawn_tank(mut commands: Commands) {
    commands.spawn((
        Tank,
        Velocity::default(),
        DriveCommand::default(),
        Hull(hull::HullParams::TANK),
        Transform::default(),
    ));
}

/// Removes the tank when the run ends.
pub fn despawn_tank(mut commands: Commands, tanks: Query<Entity, With<Tank>>) {
    for entity in &tanks {
        commands.entity(entity).despawn();
    }
}

/// Samples the keyboard into each tank's drive command.
///
/// WASD drives the hull. Arrow keys are left free for the turret in M2.
pub fn sample_drive_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut tanks: Query<&mut DriveCommand, With<Tank>>,
) {
    let mut raw = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyW) {
        raw.y += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        raw.y -= 1.0;
    }
    if keys.pressed(KeyCode::KeyA) {
        raw.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) {
        raw.x += 1.0;
    }

    let drive = hull::clamp_drive(raw);
    for mut command in &mut tanks {
        command.0 = drive;
    }
}

/// Clears every drive command, so a tank cannot coast on stale input.
///
/// Without this, pausing mid-throttle would leave the command set and the tank
/// would lurch forward the instant the game resumes.
pub fn clear_drive_input(mut tanks: Query<&mut DriveCommand>) {
    for mut command in &mut tanks {
        command.0 = Vec2::ZERO;
    }
}

/// Advances every tank by one simulation tick.
///
/// M1 replaces the position update with an axis-separated slide against the
/// maze grid; the velocity update stays as it is.
pub fn move_tanks(mut tanks: Query<(&mut Transform, &mut Velocity, &DriveCommand, &Hull)>) {
    for (mut transform, mut velocity, command, params) in &mut tanks {
        velocity.0 = hull::step_velocity(velocity.0, command.0, params.0, FIXED_DT);

        let pos = hull::step_position(transform.translation.truncate(), velocity.0, FIXED_DT);
        transform.translation.x = pos.x;
        transform.translation.y = pos.y;
    }
}

/// Gives every tank its placeholder sprite.
///
/// This lives in the presentation layer rather than in [`spawn_tank`] so the
/// simulation can be stepped headlessly, with no renderer and no image assets.
pub fn attach_tank_sprite(
    mut commands: Commands,
    tanks: Query<Entity, (With<Tank>, Without<Sprite>)>,
) {
    for entity in &tanks {
        commands
            .entity(entity)
            .insert(Sprite::from_color(crate::palette::TANK, TANK_SIZE));
    }
}
