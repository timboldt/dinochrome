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
//! In M2 they are targets and nothing more. M3 gives them the drones.
//!
//! [`Blocker`]: dinochrome_core::collision::Blocker

use bevy::prelude::*;
use dinochrome_core::collision::Blocker;
use dinochrome_core::health;

use crate::maze::Maze;
use crate::palette;
use crate::player::GridCollider;
use crate::state::AppState;
use crate::weapon::Health;

/// Marks a drone factory.
#[derive(Component, Debug)]
pub struct Factory;

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
pub fn spawn_factories(mut commands: Commands, maze: Res<Maze>) {
    for &cell in &maze.factories {
        let at = maze.grid.cell_center(cell);
        commands.spawn((
            Factory,
            Health(health::Health::new(FACTORY_HEALTH)),
            GridCollider(FACTORY_RADIUS),
            Transform::from_xyz(at.x, at.y, Z_FACTORY),
        ));
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

/// Gives every factory its sprites: a body, and a core inside it.
pub fn attach_factory_sprites(mut commands: Commands, factories: UndrawnFactories) {
    for (entity, health) in &factories {
        commands.entity(entity).insert((
            Sprite::from_color(damage_tint(health), FACTORY_SIZE),
            children![(
                FactoryCore,
                Sprite::from_color(palette::FACTORY_CORE, CORE_SIZE),
                Transform::from_xyz(0.0, 0.0, Z_CORE),
            )],
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
