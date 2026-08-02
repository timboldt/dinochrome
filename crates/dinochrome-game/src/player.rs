//! The player's tank: spawning, input sampling, and movement.
//!
//! The split here is the one the whole project follows: input sampling runs in
//! `Update` (once per rendered frame, because that is when the OS gives us key
//! state), and writes a *command* onto the tank. Movement runs in `FixedUpdate`
//! and reads only that command, so the simulation advances identically no matter
//! what the frame rate is. No simulation system reads `Time::delta`.

use bevy::prelude::*;
use dinochrome_core::{FIXED_DT, collision, hull, turret, weapon};

use crate::factory::{self, Factory};
use crate::maze::Maze;
use crate::turret::{AimCommand, Traverse, Turret};
use crate::weapon::{FireCommand, Weapon};

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

/// Radius of this entity's circular collider against the maze grid, in pixels.
#[derive(Component, Debug, Default, Deref, DerefMut)]
pub struct GridCollider(pub f32);

/// Radius of the tank's collider, in pixels.
///
/// Corridors are one 64 px cell wide, so this leaves 12 px of clearance on each
/// side of a corridor. Tight enough that driving is a thing you have to do, loose
/// enough that the corner rounding in `collision::slide` can save you.
const TANK_RADIUS: f32 = 20.0;

/// Size of the placeholder tank sprite, in pixels.
///
/// Square and matched to the collider, so what you see is what collides.
const TANK_SIZE: Vec2 = Vec2::splat(TANK_RADIUS * 2.0);

/// Creates the tank on the maze's spawn cell.
pub fn spawn_tank(mut commands: Commands, maze: Res<Maze>) {
    let at = maze.grid.cell_center(maze.spawn);
    commands.spawn((
        Tank,
        Velocity::default(),
        DriveCommand::default(),
        Hull(hull::HullParams::TANK),
        GridCollider(TANK_RADIUS),
        // The turret starts pointing where the hull does, which is +X because
        // nothing has told it otherwise yet.
        Turret::default(),
        Traverse(turret::TurretParams::TANK),
        AimCommand::default(),
        Weapon(weapon::Weapon::new(weapon::WeaponParams::TANK)),
        FireCommand::default(),
        Transform::from_xyz(at.x, at.y, 0.0),
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
/// WASD drives the hull; the arrow keys aim the turret, in `crate::turret`.
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
/// Factories are buildings the maze grid knows nothing about, so they come in
/// separately — the `Without<Factory>` is what proves to Bevy that this
/// `&mut Transform` and the factories' `&Transform` can never be the same entity.
pub fn move_tanks(
    maze: Res<Maze>,
    factories: Query<(&Transform, &GridCollider), With<Factory>>,
    mut tanks: Query<
        (
            &mut Transform,
            &mut Velocity,
            &DriveCommand,
            &Hull,
            &GridCollider,
        ),
        Without<Factory>,
    >,
) {
    let blockers = factory::blockers(&factories);
    for (mut transform, mut velocity, command, params, collider) in &mut tanks {
        velocity.0 = hull::step_velocity(velocity.0, command.0, params.0, FIXED_DT);

        let from = transform.translation.truncate();
        let moved = collision::slide_around(
            &maze.grid,
            from,
            collider.0,
            velocity.0 * FIXED_DT,
            &blockers,
        );
        transform.translation.x = moved.position.x;
        transform.translation.y = moved.position.y;

        // A blocked axis has to stop dead. Left alone, the hull would keep
        // accelerating into the wall it is already flush against and then launch
        // the moment the player steered away from it.
        if moved.blocked.x {
            velocity.x = 0.0;
        }
        if moved.blocked.y {
            velocity.y = 0.0;
        }
    }
}

/// Gives every tank its placeholder sprite, and the barrel that hangs off it.
///
/// This lives in the presentation layer rather than in [`spawn_tank`] so the
/// simulation can be stepped headlessly, with no renderer and no image assets.
/// Both sprites are attached together because both are keyed off the hull not
/// having one yet — split in two, whichever ran second would run every frame.
pub fn attach_tank_sprite(
    mut commands: Commands,
    tanks: Query<Entity, (With<Tank>, Without<Sprite>)>,
) {
    for entity in &tanks {
        commands.entity(entity).insert((
            Sprite::from_color(crate::palette::TANK, TANK_SIZE),
            children![crate::turret::barrel()],
        ));
    }
}
