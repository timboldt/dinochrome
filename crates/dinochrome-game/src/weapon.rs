//! The gun, the shells it fires, and what they do when they arrive.
//!
//! Shells do not slide. A shell's tick is a straight sweep that ends the first
//! time it touches anything, which [`collision::sweep`] solves for exactly, so
//! there is no speed at which one passes through a wall and no clamp on the
//! design to keep that true.
//!
//! The one thing a shell has to decide for itself is which of several things it
//! reached first. Walls come back as a fraction of the tick's travel and so does
//! every target it could have hit, so "first" is a comparison rather than a
//! special case.
//!
//! Finding those candidates is a linear scan over everything with a [`Health`], run
//! once per shell. At M2's handful of shells and four factories that is nothing; by
//! the time M3's drones make the product large enough to care about, the fix is a
//! broad-phase over the grid the maze is already stored in, not a change to any of
//! the geometry here.

use bevy::prelude::*;
use dinochrome_core::{FIXED_DT, collision, health, weapon};

use crate::maze::Maze;
use crate::player::{GridCollider, Velocity};
use crate::turret::{MUZZLE_OFFSET, Turret};
use crate::{palette, player};

/// A gun, and how long until it can fire again.
#[derive(Component, Debug, Default, Deref, DerefMut)]
pub struct Weapon(pub weapon::Weapon);

/// Whether the trigger is being held.
#[derive(Component, Debug, Default, Deref, DerefMut)]
pub struct FireCommand(pub bool);

/// How much punishment an entity can still take.
///
/// Anything carrying this is a target: [`move_shells`] damages whatever it hits
/// that has one. In M2 that is the factories; in M3 it is the player and the
/// drones as well.
#[derive(Component, Debug, Deref, DerefMut)]
pub struct Health(pub health::Health);

/// A shell in flight.
#[derive(Component, Debug)]
pub struct Shell {
    /// Damage it does on impact.
    pub damage: i32,
    /// How much further it can travel before it fizzles out, in pixels.
    pub range_left: f32,
}

/// Radius of a shell's collider, in pixels.
///
/// Small enough to fit through the ten-pixel gap either side of a factory, which
/// is a shot worth being able to line up, and to make hitting one a matter of
/// aiming rather than of being roughly right.
pub const SHELL_RADIUS: f32 = 4.0;

/// Size of the shell sprite. Matched to the collider: what you see is what hits.
const SHELL_SIZE: Vec2 = Vec2::splat(SHELL_RADIUS * 2.0);

/// Draw order for shells: over everything, so a shot is never lost behind a tank.
const Z_SHELL: f32 = 1.0;

/// Samples the keyboard into each weapon's trigger.
///
/// Held rather than tapped: the cooldown is the rate limit, so there is nothing to
/// gain from mashing the key and no reason to make the player do it.
pub fn sample_fire_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut shooters: Query<&mut FireCommand, With<player::Tank>>,
) {
    let firing = keys.pressed(KeyCode::Space);
    for mut command in &mut shooters {
        command.0 = firing;
    }
}

/// Releases every trigger, so a gun cannot keep firing on stale input.
pub fn clear_fire_input(mut shooters: Query<&mut FireCommand>) {
    for mut command in &mut shooters {
        command.0 = false;
    }
}

/// Ticks every gun's cooldown and spawns a shell for each one that fired.
pub fn fire_weapons(
    mut commands: Commands,
    maze: Res<Maze>,
    mut shooters: Query<(&Transform, &Turret, &mut Weapon, &FireCommand)>,
) {
    for (transform, turret, mut weapon, trigger) in &mut shooters {
        // The cooldown runs down whether or not the trigger is held, so letting go
        // of it is never a way to reload faster.
        weapon.tick(FIXED_DT);
        if !trigger.0 || !weapon.fire() {
            continue;
        }

        let params = weapon.params();
        let heading = Vec2::from_angle(turret.0);
        let from = transform.translation.truncate();
        // Normally the muzzle is out at the end of the barrel. With the tank
        // nose-in against a wall it is inside that wall, and sweeping to it puts
        // the shell wherever it actually fits — flush against the wall, which it
        // then hits on its first tick. Firing into a wall you are touching is
        // allowed to be a wasted shell; it is not allowed to spawn one inside the
        // masonry.
        let muzzle = collision::sweep(&maze.grid, from, SHELL_RADIUS, heading * MUZZLE_OFFSET);

        commands.spawn((
            Shell {
                damage: params.damage,
                range_left: params.range,
            },
            Velocity(heading * params.shell_speed),
            GridCollider(SHELL_RADIUS),
            Transform::from_xyz(muzzle.position.x, muzzle.position.y, Z_SHELL),
        ));
    }
}

/// Advances every shell by one tick, and applies what it ran into.
pub fn move_shells(
    mut commands: Commands,
    maze: Res<Maze>,
    mut shells: Query<(Entity, &mut Transform, &Velocity, &GridCollider, &mut Shell)>,
    mut targets: Query<(Entity, &Transform, &GridCollider, &mut Health), Without<Shell>>,
) {
    for (entity, mut transform, velocity, collider, mut shell) in &mut shells {
        let from = transform.translation.truncate();
        let full_step = velocity.0 * FIXED_DT;
        let step_length = full_step.length();
        // Range is a distance, so the last tick of a shell's life is a short one
        // rather than a whole one that happens to be the last.
        let delta = if step_length > shell.range_left {
            full_step * (shell.range_left / step_length)
        } else {
            full_step
        };

        let wall = collision::sweep(&maze.grid, from, collider.0, delta);
        // Anything nearer than the wall — or nearer than the end of the move, if
        // there was no wall — is what the shell actually hit.
        let mut travel = wall.travel;
        let mut struck = None;
        for (target, at, target_collider, _) in targets.iter() {
            let center = at.translation.truncate();
            if let Some(t) =
                collision::hit_circle(from, delta, collider.0, center, target_collider.0)
                && t <= travel
            {
                travel = t;
                struck = Some(target);
            }
        }

        let end = from + delta * travel;
        transform.translation.x = end.x;
        transform.translation.y = end.y;

        if let Some(target) = struck {
            if let Ok((_, _, _, mut health)) = targets.get_mut(target) {
                health.damage(shell.damage);
            }
            commands.entity(entity).despawn();
            continue;
        }
        if wall.hit.is_some() {
            commands.entity(entity).despawn();
            continue;
        }

        shell.range_left -= delta.length();
        if shell.range_left <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}

/// Gives every shell its sprite.
///
/// Presentation, like the tank's: the simulation has to be able to run a firefight
/// with no renderer attached.
pub fn attach_shell_sprites(
    mut commands: Commands,
    shells: Query<Entity, (With<Shell>, Without<Sprite>)>,
) {
    for entity in &shells {
        commands
            .entity(entity)
            .insert(Sprite::from_color(palette::SHELL, SHELL_SIZE));
    }
}
