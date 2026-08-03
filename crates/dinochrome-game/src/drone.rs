//! Drones: the things factories build, and what they do once they are out.
//!
//! A drone is the player's tank with something other than a keyboard writing its
//! controls. It has the same [`Velocity`], [`DriveCommand`], [`Hull`] and
//! [`GridCollider`] the tank does, so it goes through the same
//! [`player::move_hulls`] and the same collision resolution; it has the same
//! [`Turret`], [`AimCommand`] and [`Weapon`], so it fires through the same
//! [`weapon::fire_weapons`]. Everything in this module writes commands. Nothing in
//! it moves anything.
//!
//! That is the whole design, and it buys the thing that matters: there is exactly
//! one implementation of driving a hull through a maze, so a drone can never be
//! stopped by a wall the tank drives through or vice versa.
//!
//! # Steering by cells
//!
//! All four kinds pick a *cell* to head for and then drive at its centre, and they
//! differ only in how they pick it — see [`dinochrome_core::drone`]. Cells rather
//! than free-form steering, for two reasons. It keeps drones down the middle of
//! corridors instead of scraping along the walls, which is what makes them
//! dodgeable. And it means the dumb kinds and the clever one are the same
//! machinery with a different choice function, so an assassin is not a second
//! movement system that happens to be better.
//!
//! [`Hull`]: crate::player::Hull
//! [`GridCollider`]: crate::player::GridCollider

use bevy::math::IVec2;
use bevy::prelude::*;
use dinochrome_core::drone::{Pursuit, Trigger};
use dinochrome_core::grid::ORTHOGONAL;
use dinochrome_core::{
    DroneKind, FIXED_DT, Grid, drone, health, hull, los, path, turret as core_turret, weapon,
};

use crate::maze::{Maze, SimRandom};
use crate::palette;
use crate::player::{DriveCommand, GridCollider, Hull, Tank, Velocity};
use crate::turret::{AimCommand, Traverse, Turret};
use crate::weapon::{Faction, FireCommand, Health, Muzzle, Weapon};

/// Marks a drone, and says which kind it is.
#[derive(Component, Debug)]
pub struct Drone {
    /// Which of the four this is. Everything else about it follows from this.
    pub kind: DroneKind,
}

/// The factory that built this drone.
///
/// Kept so a factory can count its own live output and stop building at the cap.
/// A drone outlives the factory that made it — blowing up the plant does not
/// recall what has already shipped — so this is deliberately not a parent-child
/// relationship, which would take the drones down with it.
#[derive(Component, Debug, Deref)]
pub struct BuiltBy(pub Entity);

/// The cell a drone is currently driving at, and the one it came from.
///
/// `previous` is what stops a wanderer rattling back and forth in one corridor: a
/// drone will take any opening except the one behind it, unless that is the only
/// one there is.
#[derive(Component, Debug)]
pub struct Waypoint {
    /// Where it is headed.
    pub cell: IVec2,
    /// Where it came from, if it has been anywhere.
    pub previous: Option<IVec2>,
}

/// An assassin's route, and how long until it works out a new one.
///
/// Only the kinds that pathfind carry this; the rest decide where to go from what
/// is next to them and have nothing to remember.
#[derive(Component, Debug, Default)]
pub struct Plan {
    /// Cells still to walk, **nearest last**, so the next step is a `pop`.
    pub route: Vec<IVec2>,
    /// Seconds until the route is thrown away and recomputed.
    pub countdown: f32,
}

/// Radius of a drone's collider, in pixels.
///
/// Small enough that two abreast fit down a corridor, so a corridor with a drone
/// in it is a fight rather than a wall. Half the tank's, which also makes the size
/// difference on screen the thing that tells you which one is you.
pub const DRONE_RADIUS: f32 = 14.0;

/// Size of a drone's sprite. Matched to the collider.
const DRONE_SIZE: Vec2 = Vec2::splat(DRONE_RADIUS * 2.0);

/// How far a drone's muzzle sits from its centre, in pixels.
const DRONE_MUZZLE: f32 = DRONE_RADIUS + 6.0;

/// How near a drone must get to a cell's centre to count as having arrived, in
/// pixels.
///
/// Loose enough that a drone shoved off line by a factory still registers the
/// arrival and picks its next cell, rather than circling the centre it cannot
/// quite touch; tight enough that it is genuinely in the cell when it decides
/// where to go next.
const ARRIVED: f32 = 6.0;

/// How far off the bearing a turret may be and still be allowed to fire, in
/// radians.
///
/// Without this a drone would loose a round the instant its target came into
/// sight, while the turret was still slewing, and the shell would go somewhere
/// off to the side. About seven degrees: close enough that the shot is honest, wide
/// enough that a drone is not disarmed by a target that keeps moving.
const AIM_TOLERANCE: f32 = 0.12;

/// Draw order for drones: level with the tank, above the maze and the factories.
const Z_DRONE: f32 = 0.0;

/// Creates a drone of `kind` at a world position, owned by `factory`.
///
/// `serial` is the factory's production count, used only to stagger route
/// recomputes across a fleet — see [`drone::repath_phase`].
///
/// Returns the drone, so a caller that needs to find it again later does not have
/// to go looking for whichever one is new.
pub fn spawn(
    commands: &mut Commands,
    kind: DroneKind,
    at: Vec2,
    cell: IVec2,
    factory: Entity,
    serial: u32,
) -> Entity {
    let params = kind.params();
    // Grouped into what it is, how it moves and how it shoots, because a flat
    // tuple of all sixteen is one past the longest bundle Bevy implements.
    let mut drone = commands.spawn((
        (
            Drone { kind },
            BuiltBy(factory),
            Faction::Hostile,
            Health(health::Health::new(params.health)),
        ),
        (
            Velocity::default(),
            DriveCommand::default(),
            Hull(params.hull),
            GridCollider(DRONE_RADIUS),
            Waypoint {
                cell,
                previous: None,
            },
        ),
        (
            Turret::default(),
            Traverse(core_turret::TurretParams::TANK),
            AimCommand::default(),
            Weapon(weapon::Weapon::new(params.weapon)),
            Muzzle(DRONE_MUZZLE),
            FireCommand::default(),
        ),
        Transform::from_xyz(at.x, at.y, Z_DRONE),
    ));

    if params.pursuit == Pursuit::Path {
        drone.insert(Plan {
            route: Vec::new(),
            countdown: drone::repath_phase(params.repath, serial),
        });
    }
    drone.id()
}

/// Everything a drone needs to decide what to do this tick.
type Steerable<'w, 's> = Query<
    'w,
    's,
    (
        &'static Drone,
        &'static Transform,
        &'static Turret,
        &'static mut Waypoint,
        Option<&'static mut Plan>,
        &'static mut DriveCommand,
        &'static mut AimCommand,
        &'static mut FireCommand,
    ),
    Without<Tank>,
>;

/// Writes every drone's controls for this tick.
///
/// One system for all four kinds, because the differences between them are
/// [`Pursuit`] and [`Trigger`] and nothing else. Splitting it per kind would mean
/// four copies of the waypoint bookkeeping, which is the part with the edge cases
/// in it.
pub fn steer_drones(
    maze: Res<Maze>,
    mut rng: ResMut<SimRandom>,
    tanks: Query<&Transform, With<Tank>>,
    mut drones: Steerable,
) {
    // `None` once the tank is destroyed. Drones carry on wandering rather than
    // freezing, because the game-over screen is drawn over a field that is still
    // moving.
    let quarry = tanks.iter().next().map(|at| at.translation.truncate());
    let quarry_cell = quarry.map(|at| maze.grid.cell_at(at));

    for (drone, transform, turret, mut waypoint, plan, mut drive, mut aim, mut fire) in &mut drones
    {
        let params = drone.kind.params();
        let at = transform.translation.truncate();
        let cell = maze.grid.cell_at(at);
        let mut plan = plan;

        // A drone shoved off its route — squeezed past a factory, or spawned onto
        // a cell it was not expecting — re-anchors on where it actually is. Left
        // alone it would drive at a cell it can no longer reach in one step and
        // grind against whatever is between.
        if maze.grid.is_wall(waypoint.cell) || (cell - waypoint.cell).abs().max_element() > 1 {
            waypoint.cell = cell;
            waypoint.previous = None;
        }

        if let Some(plan) = plan.as_deref_mut() {
            plan.countdown -= FIXED_DT;
            if plan.countdown <= 0.0 {
                plan.countdown = params.repath;
                plan.route.clear();
                if let Some(goal) = quarry_cell
                    && let Some(route) = path::find_path(&maze.grid, cell, goal)
                {
                    // Reversed so that walking the route is a `pop`, and without
                    // the first cell, which is the one the drone is standing on.
                    plan.route = route.cells.into_iter().skip(1).rev().collect();
                }
            }
        }

        // Arrived, so choose the next cell. Everything above is upkeep; this is
        // the decision that makes a drone the kind of drone it is.
        if at.distance(maze.grid.cell_center(waypoint.cell)) <= ARRIVED {
            let from = waypoint.cell;
            let came_from = waypoint.previous;
            let roll = rng.unit();

            let planned = plan
                .as_deref_mut()
                .and_then(|plan| next_planned(&mut plan.route, from));
            let chosen = match params.pursuit {
                Pursuit::Wander => drone::wander_step(&maze.grid, from, came_from, roll),
                Pursuit::Greedy => greedy_or_wander(&maze.grid, from, quarry_cell, came_from, roll),
                // A route that has run out, or one computed before the drone got
                // shoved somewhere else, falls back on greed rather than on
                // standing still. An assassin with a stale plan is still an
                // assassin.
                Pursuit::Path => planned
                    .or_else(|| greedy_or_wander(&maze.grid, from, quarry_cell, came_from, roll)),
            };

            if let Some(next) = chosen {
                waypoint.previous = Some(from);
                waypoint.cell = next;
            }
        }

        // Drive at the middle of the cell being headed for. `normalize_or_zero`
        // rather than an unchecked normalize because a drone sitting exactly on
        // the centre of its own waypoint has no direction to go in, and asking for
        // one would produce a NaN that would spread into its position.
        let toward = maze.grid.cell_center(waypoint.cell) - at;
        drive.0 = hull::clamp_drive(toward.normalize_or_zero());

        let seen = quarry.filter(|quarry| los::visible(&maze.grid, at, *quarry, params.sight));
        match params.trigger {
            // Points where it is going and fires on the cooldown, at whatever
            // happens to be in front of it. Menacing by accident, which is the
            // whole character of the thing.
            Trigger::Blind => {
                aim.0 = drive.0;
                fire.0 = true;
            }
            Trigger::OnSight => match seen {
                Some(quarry) => {
                    let bearing = quarry - at;
                    aim.0 = bearing;
                    // Held until the turret has caught up with the bearing, so a
                    // drone never looses a round sideways while still slewing.
                    let off = core_turret::wrap_angle(bearing.to_angle() - turret.0);
                    fire.0 = off.abs() <= AIM_TOLERANCE;
                }
                None => {
                    aim.0 = drive.0;
                    fire.0 = false;
                }
            },
        }
    }
}

/// Clears out the drones that have been shot to pieces.
pub fn destroy_dead_drones(mut commands: Commands, drones: Query<(Entity, &Health), With<Drone>>) {
    for (entity, health) in &drones {
        if health.is_dead() {
            commands.entity(entity).despawn();
        }
    }
}

/// The next cell of a route that is actually one step from `from`.
///
/// A route is thrown away and recomputed on a timer, so the one being held can
/// have been worked out from a cell the drone has since left. Cells that are no
/// longer a single step away are dropped rather than driven at.
fn next_planned(route: &mut Vec<IVec2>, from: IVec2) -> Option<IVec2> {
    while let Some(next) = route.pop() {
        if ORTHOGONAL.contains(&(next - from)) {
            return Some(next);
        }
    }
    None
}

/// Steps toward `quarry` if there is one, and wanders if there is not.
fn greedy_or_wander(
    grid: &Grid,
    from: IVec2,
    quarry: Option<IVec2>,
    came_from: Option<IVec2>,
    roll: f32,
) -> Option<IVec2> {
    match quarry {
        Some(quarry) => drone::greedy_step(grid, from, quarry, came_from, roll),
        None => drone::wander_step(grid, from, came_from, roll),
    }
}

/// Gives every drone its sprite, in the colour of its kind.
///
/// Presentation, like the tank's: the simulation has to be able to run a firefight
/// with no renderer attached. The colours run from a dull red for the harmless
/// kind up to near-gold for the assassin, so how worried to be is something the
/// player reads off the screen rather than off a legend.
pub fn attach_drone_sprites(
    mut commands: Commands,
    drones: Query<(Entity, &Drone), Without<Sprite>>,
) {
    for (entity, drone) in &drones {
        let color = match drone.kind {
            DroneKind::Drone => palette::DRONE,
            DroneKind::Torpedo => palette::DRONE_TORPEDO,
            DroneKind::Hunter => palette::DRONE_HUNTER,
            DroneKind::Assassin => palette::DRONE_ASSASSIN,
        };
        commands.entity(entity).insert((
            Sprite::from_color(color, DRONE_SIZE),
            children![crate::turret::barrel_for(DRONE_MUZZLE)],
        ));
    }
}
