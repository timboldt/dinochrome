//! Drone factories: the things a level is about clearing.
//!
//! A factory is a building standing in an open cell. The maze grid does not know
//! it is there — walls and factories come and go on completely different
//! timescales — so it is a circular obstacle handed to the collision layer as a
//! [`Blocker`] instead, which is what makes a tank drive around one and a shell
//! stop dead on one.
//!
//! It is wide enough to plug a one-cell corridor, so where it may stand is not a
//! free choice: `maze::pick_factories` places them only on cells the rest of the
//! maze does not route through, which is what keeps a level winnable.
//!
//! A factory does two things besides standing there and taking damage: it builds
//! drones on a timer, and it shoots back at anything that gets close enough to be
//! camping it.
//!
//! Both of those are limits rather than features. Unbounded production would let a
//! player who ignored a factory come back to a maze full of drones, so a factory
//! may only have so many of its own alive at once ([`LIVE_CAP`]); and a factory
//! that could not defend itself would make parking outside its door the whole
//! game, so it has a gun that reaches four cells and no further.
//!
//! [`Blocker`]: dinochrome_core::collision::Blocker

use bevy::prelude::*;
use dinochrome_core::collision::Blocker;
use dinochrome_core::{FIXED_DT, drone as core_drone, health, los, turret as core_turret, weapon};

use crate::drone::{self, BuiltBy};
use crate::maze::{Maze, MazeConfig, SimRandom};
use crate::palette;
use crate::player::{GridCollider, Tank};
use crate::state::AppState;
use crate::turret::{AimCommand, Traverse, Turret};
use crate::weapon::{Faction, FireCommand, Health, Muzzle, Weapon};

/// Marks a drone factory, and holds its production line.
#[derive(Component, Debug)]
pub struct Factory {
    /// Seconds until the next drone rolls out.
    pub countdown: f32,
    /// How many this factory has built since the level started.
    ///
    /// Not a limit — it only ever goes up. It is the serial number that staggers
    /// its drones' route recomputes so a fleet does not all think on the same tick.
    pub built: u32,
}

/// How many of its own drones a factory may have alive at once.
///
/// The answer to camping from the other direction. A player who parks somewhere
/// safe and farms a factory should run out of things to shoot, not be buried; six
/// is enough to make a factory's neighbourhood genuinely dangerous and few enough
/// that clearing it out stays possible.
pub const LIVE_CAP: usize = 6;

/// Seconds between drones, when there is room for another.
const SPAWN_INTERVAL: f32 = 3.5;

/// Seconds before a factory at its cap tries again.
///
/// Short, and deliberately not the full interval: the point of the cap is to bound
/// how many drones exist, not to reward the player with a lull for having killed
/// one. But it is not zero either, because retrying every tick would mean counting
/// the live drones sixty times a second for nothing.
const CAP_RETRY: f32 = 0.5;

/// How far a factory can see to shoot, in pixels.
///
/// Its gun's full range, so it never declines a shot it could land — see
/// [`weapon::WeaponParams::FACTORY`] for why that range is as short as it is.
const DEFENCE_RANGE: f32 = weapon::WeaponParams::FACTORY.range;

/// How far off the bearing a factory's gun may be and still fire, in radians.
const AIM_TOLERANCE: f32 = 0.12;

/// How far a factory's muzzle sits from its centre, in pixels.
const FACTORY_MUZZLE: f32 = FACTORY_RADIUS + 6.0;

/// Marks a factory's core sprite: the bright middle you aim at.
#[derive(Component)]
pub struct FactoryCore;

/// Radius of a factory's collider, in pixels.
///
/// A 44 px building in a 64 px cell leaves ten pixels either side of it — too
/// narrow for the tank's twenty-pixel radius, so it has to be driven around, and
/// wide enough for a shell, so there is a shot to line up down a corridor past one.
pub const FACTORY_RADIUS: f32 = 22.0;

/// Size of the factory sprite. Matched to the collider.
const FACTORY_SIZE: Vec2 = Vec2::splat(FACTORY_RADIUS * 2.0);

/// Size of the core sprite inside it.
const CORE_SIZE: Vec2 = Vec2::splat(16.0);

/// A factory's hit points.
///
/// Five shells of the tank's twenty damage. Long enough that a factory has to be
/// committed to rather than driven past, short enough that it is not a chore.
const FACTORY_HEALTH: i32 = 100;

/// Draw order: above the maze, below the tank driving around it.
const Z_FACTORY: f32 = -0.5;

/// Draw order for the core, relative to the body it is a child of.
const Z_CORE: f32 = 0.1;

/// Creates a factory on each of the maze's factory cells.
///
/// Every factory starts with a full interval on the clock rather than shipping a
/// drone the instant the level begins, so the opening seconds are navigation
/// rather than an ambush.
pub fn spawn_factories(mut commands: Commands, maze: Res<Maze>) {
    for &cell in &maze.factories {
        let at = maze.grid.cell_center(cell);
        commands.spawn((
            Factory {
                countdown: SPAWN_INTERVAL,
                built: 0,
            },
            Faction::Hostile,
            Health(health::Health::new(FACTORY_HEALTH)),
            GridCollider(FACTORY_RADIUS),
            Turret::default(),
            Traverse(core_turret::TurretParams::FACTORY),
            AimCommand::default(),
            Weapon(weapon::Weapon::new(weapon::WeaponParams::FACTORY)),
            Muzzle(FACTORY_MUZZLE),
            FireCommand::default(),
            Transform::from_xyz(at.x, at.y, Z_FACTORY),
        ));
    }
}

/// Builds drones, on a timer and up to a cap.
///
/// A drone is put down on an open cell *next to* the factory rather than inside
/// it. The building is wide enough to fill most of its own cell, so a drone
/// spawned on top of one would start the game being shoved out of it.
pub fn build_drones(
    mut commands: Commands,
    maze: Res<Maze>,
    config: Res<MazeConfig>,
    mut rng: ResMut<SimRandom>,
    mut factories: Query<(Entity, &Transform, &mut Factory)>,
    built: Query<&BuiltBy>,
) {
    for (entity, transform, mut factory) in &mut factories {
        factory.countdown -= FIXED_DT;
        if factory.countdown > 0.0 {
            continue;
        }

        // Counted rather than tracked on the factory, because a drone can die to
        // anything at any time and a counter maintained from three places is a
        // counter that eventually disagrees with the world. At a cap of six
        // against a handful of factories this is a few dozen comparisons, twice a
        // second, per factory at its limit.
        let live = built.iter().filter(|by| by.0 == entity).count();
        if live >= LIVE_CAP {
            factory.countdown = CAP_RETRY;
            continue;
        }

        let at = transform.translation.truncate();
        let cell = maze.grid.cell_at(at);
        let doors: Vec<_> = maze.grid.open_neighbours(cell).collect();
        let Some(door) = rng.below(doors.len()).map(|index| doors[index]) else {
            // A factory with no open cell beside it cannot ship, and never will —
            // but maze generation refuses to place one anywhere that could be
            // walled in, so this is a belt-and-braces case rather than a real one.
            factory.countdown = SPAWN_INTERVAL;
            continue;
        };

        let kind = core_drone::kind_at(config.level, rng.unit());
        drone::spawn(
            &mut commands,
            kind,
            maze.grid.cell_center(door),
            door,
            entity,
            factory.built,
        );
        factory.built += 1;
        factory.countdown = SPAWN_INTERVAL;
    }
}

/// Every factory's gun, and where it is pointing.
type Defences<'w, 's> = Query<
    'w,
    's,
    (
        &'static Transform,
        &'static Turret,
        &'static mut AimCommand,
        &'static mut FireCommand,
    ),
    (With<Factory>, Without<Tank>),
>;

/// Points each factory's gun at the player, and fires it if the player is close
/// enough to be worth the ammunition.
pub fn aim_defences(
    maze: Res<Maze>,
    tanks: Query<&Transform, With<Tank>>,
    mut factories: Defences,
) {
    let quarry = tanks.iter().next().map(|at| at.translation.truncate());

    for (transform, turret, mut aim, mut fire) in &mut factories {
        let at = transform.translation.truncate();
        let Some(quarry) =
            quarry.filter(|quarry| los::visible(&maze.grid, at, *quarry, DEFENCE_RANGE))
        else {
            // A turret with nothing asked of it holds its bearing, so a factory
            // stays pointed where it last saw you. That is the warning that you
            // are coming back into its arc.
            aim.0 = Vec2::ZERO;
            fire.0 = false;
            continue;
        };

        let bearing = quarry - at;
        aim.0 = bearing;
        let off = core_turret::wrap_angle(bearing.to_angle() - turret.0);
        fire.0 = off.abs() <= AIM_TOLERANCE;
    }
}

/// Collects the standing factories as obstacles for anything that has to drive
/// around them.
pub fn blockers(factories: &Query<(&Transform, &GridCollider), With<Factory>>) -> Vec<Blocker> {
    factories
        .iter()
        .map(|(transform, collider)| Blocker {
            center: transform.translation.truncate(),
            radius: collider.0,
        })
        .collect()
}

/// Clears out the factories that have been shot to pieces, and ends the level with
/// the last of them.
pub fn destroy_dead_factories(
    mut commands: Commands,
    factories: Query<(Entity, &Health), With<Factory>>,
    mut next: ResMut<NextState<AppState>>,
) {
    let mut standing = 0;
    let mut destroyed = 0;
    for (entity, health) in &factories {
        if health.is_dead() {
            destroyed += 1;
            commands.entity(entity).despawn();
        } else {
            standing += 1;
        }
    }

    // Only the shot that takes out the *last* factory clears the level. Checking
    // for an empty world instead would clear a level that never had a factory in
    // it — and a maze with nowhere to put one is a legitimate thing to generate.
    if destroyed > 0 && standing == 0 {
        info!("sector cleared");
        next.set(AppState::LevelComplete);
    }
}

/// Factories that have not been drawn yet.
type UndrawnFactories<'w, 's> =
    Query<'w, 's, (Entity, &'static Health), (With<Factory>, Without<Sprite>)>;

/// Factories whose health moved since the last frame, with the sprite that shows it.
type DamagedFactories<'w, 's> =
    Query<'w, 's, (&'static Health, &'static mut Sprite), (With<Factory>, Changed<Health>)>;

/// Gives every factory its sprites: a body, a core inside it, and the gun it
/// defends itself with.
pub fn attach_factory_sprites(mut commands: Commands, factories: UndrawnFactories) {
    for (entity, health) in &factories {
        commands.entity(entity).insert((
            Sprite::from_color(damage_tint(health), FACTORY_SIZE),
            children![
                (
                    FactoryCore,
                    Sprite::from_color(palette::FACTORY_CORE, CORE_SIZE),
                    Transform::from_xyz(0.0, 0.0, Z_CORE),
                ),
                crate::turret::barrel_for(FACTORY_MUZZLE),
            ],
        ));
    }
}

/// Darkens a factory as it takes damage.
///
/// The only feedback M2 has that a shell connected — there is no HUD until M4 — so
/// it earns its place rather than being polish.
pub fn show_factory_damage(mut factories: DamagedFactories) {
    for (health, mut sprite) in &mut factories {
        sprite.color = damage_tint(health);
    }
}

/// A factory's body colour at its current health.
fn damage_tint(health: &Health) -> Color {
    palette::FACTORY_WRECKED.mix(&palette::FACTORY, health.fraction())
}
