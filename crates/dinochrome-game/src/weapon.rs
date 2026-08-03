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
//! once per shell. With M3's drone caps that is at most a couple of dozen targets
//! against a handful of shells in the air, which is still nothing; the fix when it
//! stops being nothing is a broad phase over the grid the maze is already stored
//! in, not a change to any of the geometry here.
//!
//! # Whose shell is whose
//!
//! Every gun and every target carries a [`Faction`], and a shell inherits its
//! shooter's. A shell passes straight through anything on its own side, which is
//! what lets a factory shoot over the drones it built and a drone fire down a
//! corridor with two others in it. Walls make no such distinction — every shell
//! stops on those.

use bevy::prelude::*;
use dinochrome_core::{FIXED_DT, collision, health, weapon};

use crate::maze::Maze;
use crate::player::{GridCollider, Velocity};
use crate::turret::Turret;
use crate::{palette, player};

/// Which side of the war something is on.
///
/// Guns, targets and shells all carry one. It decides exactly one thing — whether
/// a shell may hurt what it touches — and deliberately nothing else, so that
/// "hostile" stays a fact about a shell rather than a second place for behaviour
/// to live.
#[derive(Component, Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Faction {
    /// The player's tank and everything it fires.
    #[default]
    Player,
    /// Factories, drones, and everything they fire.
    Hostile,
}

/// How far this entity's muzzle is from its centre, in pixels.
///
/// Per entity rather than a constant, because a drone is a third of the tank's
/// size and a shell appearing a tank's barrel-length away from a small one would
/// look like it came from nowhere. Far enough out that a round is visibly leaving
/// the barrel; [`fire_weapons`] sweeps to it, so it can never put a shell through
/// a wall the shooter is nose-up against.
#[derive(Component, Debug, Deref, DerefMut)]
pub struct Muzzle(pub f32);

/// A gun, and how long until it can fire again.
#[derive(Component, Debug, Default, Deref, DerefMut)]
pub struct Weapon(pub weapon::Weapon);

/// Whether the trigger is being held.
#[derive(Component, Debug, Default, Deref, DerefMut)]
pub struct FireCommand(pub bool);

/// How much punishment an entity can still take.
///
/// Anything carrying this is a target: [`move_shells`] damages whatever it hits
/// that has one and is not on its own side. That is the tank, the factories and
/// every drone.
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

/// Every gun on the field: the tank's, the drones', and the factories'.
type Shooters<'w, 's> = Query<
    'w,
    's,
    (
        &'static Transform,
        &'static Turret,
        &'static mut Weapon,
        &'static FireCommand,
        &'static Muzzle,
        &'static Faction,
    ),
>;

/// Ticks every gun's cooldown and spawns a shell for each one that fired.
pub fn fire_weapons(mut commands: Commands, maze: Res<Maze>, mut shooters: Shooters) {
    for (transform, turret, mut weapon, trigger, muzzle, faction) in &mut shooters {
        // The cooldown runs down whether or not the trigger is held, so letting go
        // of it is never a way to reload faster.
        weapon.tick(FIXED_DT);
        if !trigger.0 || !weapon.fire() {
            continue;
        }

        let params = weapon.params();
        let heading = Vec2::from_angle(turret.0);
        let from = transform.translation.truncate();
        // Normally the muzzle is out at the end of the barrel. With the shooter
        // nose-in against a wall it is inside that wall, and sweeping to it puts
        // the shell wherever it actually fits — flush against the wall, which it
        // then hits on its first tick. Firing into a wall you are touching is
        // allowed to be a wasted shell; it is not allowed to spawn one inside the
        // masonry.
        let at = collision::sweep(&maze.grid, from, SHELL_RADIUS, heading * muzzle.0);

        commands.spawn((
            Shell {
                damage: params.damage,
                range_left: params.range,
            },
            *faction,
            Velocity(heading * params.shell_speed),
            GridCollider(SHELL_RADIUS),
            Transform::from_xyz(at.position.x, at.position.y, Z_SHELL),
        ));
    }
}

/// Shells in flight.
type Shells<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut Transform,
        &'static Velocity,
        &'static GridCollider,
        &'static mut Shell,
        &'static Faction,
    ),
>;

/// Everything a shell could hurt.
type Targets<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Transform,
        &'static GridCollider,
        &'static mut Health,
        &'static Faction,
    ),
    Without<Shell>,
>;

/// Advances every shell by one tick, and applies what it ran into.
pub fn move_shells(
    mut commands: Commands,
    maze: Res<Maze>,
    mut shells: Shells,
    mut targets: Targets,
) {
    for (entity, mut transform, velocity, collider, mut shell, side) in &mut shells {
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
        for (target, at, target_collider, _, target_side) in targets.iter() {
            // Its own side is not there as far as this shell is concerned, so a
            // factory can shoot over its drones and a drone down a corridor with
            // two others in it.
            if target_side == side {
                continue;
            }
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
            if let Ok((_, _, _, mut health, _)) = targets.get_mut(target) {
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

/// Shells that have not been drawn yet.
type UndrawnShells<'w, 's> =
    Query<'w, 's, (Entity, &'static Faction), (With<Shell>, Without<Sprite>)>;

/// Gives every shell its sprite, in the colour of whoever fired it.
///
/// Presentation, like the tank's: the simulation has to be able to run a firefight
/// with no renderer attached.
pub fn attach_shell_sprites(mut commands: Commands, shells: UndrawnShells) {
    for (entity, faction) in &shells {
        let color = match faction {
            Faction::Player => palette::SHELL,
            Faction::Hostile => palette::HOSTILE_SHELL,
        };
        commands
            .entity(entity)
            .insert(Sprite::from_color(color, SHELL_SIZE));
    }
}
