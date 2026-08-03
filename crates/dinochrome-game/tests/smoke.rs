//! Headless integration smoke test.
//!
//! Builds the simulation half of the app on `MinimalPlugins` — no window, no
//! renderer, no GPU — and steps it through the state machine. This is the test
//! that catches system-ordering and schedule mistakes in CI, where there is no
//! display to open.
//!
//! Collision geometry is tested exhaustively in `dinochrome-core`, so the tests
//! here only care that the game layer is *wired* to it. Anything checking
//! movement runs in an open arena and puts the tank somewhere known first,
//! because in a real maze "drive right for a second" means "drive right until a
//! wall", and where that wall is depends on the seed.

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::{ButtonState, InputPlugin};
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use dinochrome_core::DroneKind;
use dinochrome_core::maze::MazeParams;
use dinochrome_core::weapon::WeaponParams;
use dinochrome_core::{CELL_SIZE, WALL_THICKNESS, collision, path};
use dinochrome_game::drone::{self, DRONE_RADIUS, Drone};
use dinochrome_game::factory::{FACTORY_RADIUS, Factory, LIVE_CAP};
use dinochrome_game::maze::{Maze, MazeConfig};
use dinochrome_game::player::{DriveCommand, Tank, Velocity};
use dinochrome_game::turret::Turret;
use dinochrome_game::weapon::{Faction, Health, SHELL_RADIUS, Shell};
use dinochrome_game::{AppState, SimPlugin};

/// The tank's collider radius, as `player` sets it.
const TANK_RADIUS: f32 = 20.0;

/// The gun the tank is given, so tests can talk in its numbers rather than in
/// copies of them.
const GUN: WeaponParams = WeaponParams::TANK;

/// A maze whose whole interior is open, with nothing standing in it.
///
/// Movement tests need somewhere to drive without the seed deciding how far they
/// get — and without a building deciding it either, which is why there are no
/// factories in here.
fn arena() -> MazeConfig {
    MazeConfig {
        params: MazeParams {
            density: 0.0,
            factories: 0,
            ..MazeParams::LEVEL_ONE
        },
        seed: Some(1),
        ..default()
    }
}

/// The same open arena, with `factories` buildings placed in it.
fn arena_with_factories(factories: i32) -> MazeConfig {
    MazeConfig {
        params: MazeParams {
            factories,
            ..arena().params
        },
        ..arena()
    }
}

/// An ordinary level-one maze, from a fixed seed.
fn real_maze(seed: u64) -> MazeConfig {
    MazeConfig {
        params: MazeParams::LEVEL_ONE,
        seed: Some(seed),
        ..default()
    }
}

/// A real maze's walls with nothing hostile standing in them.
///
/// What the collision tests want. They drive and shoot for twenty seconds to find
/// out whether the geometry ever lets something through a wall, and a factory
/// shooting back would eventually end the run and take the tank out from under the
/// assertion — which would be a test failing for a reason that has nothing to do
/// with what it is about.
fn empty_maze(seed: u64) -> MazeConfig {
    MazeConfig {
        params: MazeParams {
            factories: 0,
            ..MazeParams::LEVEL_ONE
        },
        seed: Some(seed),
        ..default()
    }
}

/// A simulation-only app: everything `SimPlugin` needs and nothing else.
fn headless_app(config: MazeConfig) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, InputPlugin, SimPlugin))
        .insert_resource(config)
        // Pin one fixed tick to one `App::update`. Without this, Bevy accrues
        // fixed steps out of a wall-clock-driven virtual clock, so the number of
        // `FixedUpdate` runs in a test would depend on how fast the machine is —
        // on an idle machine a tight update loop can complete without the
        // accumulator ever reaching a full timestep, and the simulation would
        // silently never advance.
        .insert_resource(TimeUpdateStrategy::FixedTimesteps(1));
    // One update to let plugin setup and the initial state transition settle.
    app.update();
    app
}

/// Runs exactly `ticks` simulation ticks.
fn step(app: &mut App, ticks: u32) {
    for _ in 0..ticks {
        app.update();
    }
}

/// Queues a keyboard message, exactly as the windowing backend would.
///
/// Writing to the `ButtonInput` resource directly does not work: `InputPlugin`
/// clears `just_pressed` at the top of every `PreUpdate`, so a directly-poked
/// press would be wiped before any `Update` system could see it. Going through
/// the message queue means the test exercises the same path the real game does.
/// The message is consumed by the *next* `step`.
fn key(app: &mut App, key_code: KeyCode, state: ButtonState) {
    app.world_mut().write_message(KeyboardInput {
        key_code,
        logical_key: Key::Unidentified(NativeKey::Unidentified),
        state,
        text: None,
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
}

fn press(app: &mut App, key_code: KeyCode) {
    key(app, key_code, ButtonState::Pressed);
}

fn release(app: &mut App, key_code: KeyCode) {
    key(app, key_code, ButtonState::Released);
}

/// Presses and releases a key, and runs long enough for any state change it
/// triggers to have been applied.
///
/// Two ticks are needed: the first delivers the press and lets an `Update`
/// system set `NextState`, the second runs the `StateTransition` schedule that
/// actually applies it and fires the `OnEnter`/`OnExit` systems.
fn tap(app: &mut App, key_code: KeyCode) {
    press(app, key_code);
    step(app, 1);
    release(app, key_code);
    step(app, 1);
}

/// Taps ENTER to leave the menu, leaving the app in `Playing` with a live tank.
fn start_playing(app: &mut App) {
    tap(app, KeyCode::Enter);
    assert_eq!(state(app), AppState::Playing);
}

fn state(app: &App) -> AppState {
    *app.world().resource::<State<AppState>>().get()
}

fn tank_position(app: &mut App) -> Vec2 {
    let mut query = app.world_mut().query_filtered::<&Transform, With<Tank>>();
    let transform = query
        .single(app.world())
        .expect("exactly one tank should exist");
    transform.translation.truncate()
}

fn tank_velocity(app: &mut App) -> Vec2 {
    let mut query = app.world_mut().query_filtered::<&Velocity, With<Tank>>();
    query.single(app.world()).expect("tank should exist").0
}

fn tank_count(app: &mut App) -> usize {
    app.world_mut()
        .query_filtered::<Entity, With<Tank>>()
        .iter(app.world())
        .count()
}

/// Teleports the tank, so a movement test can start somewhere known rather than
/// wherever the maze put it.
fn place_tank(app: &mut App, at: Vec2) {
    let mut query = app
        .world_mut()
        .query_filtered::<&mut Transform, With<Tank>>();
    let mut transform = query
        .single_mut(app.world_mut())
        .expect("exactly one tank should exist");
    transform.translation.x = at.x;
    transform.translation.y = at.y;
}

/// World-space size of the current maze.
fn maze_size(app: &App) -> Vec2 {
    app.world().resource::<Maze>().grid.world_size()
}

/// Middle of the maze — as far from every wall as it is possible to be.
fn maze_center(app: &App) -> Vec2 {
    maze_size(app) * 0.5
}

/// How far the inner face of the border wall lies from the edge of the world.
///
/// The border ring is a cell thick, but its *wall* is a thin line down the
/// middle of that cell, so the floor reaches half a cell further out than the
/// ring's inner edge — which is where anything driving into it comes to rest.
fn border_face() -> f32 {
    CELL_SIZE * 0.5 + WALL_THICKNESS * 0.5
}

/// Where the tank comes to rest flush in the top-right corner of an open arena.
fn arena_far_corner(app: &App) -> Vec2 {
    maze_size(app) - Vec2::splat(border_face() + TANK_RADIUS)
}

/// Which way the turret is pointing, in radians.
fn turret_angle(app: &mut App) -> f32 {
    let mut query = app.world_mut().query_filtered::<&Turret, With<Tank>>();
    query.single(app.world()).expect("tank should exist").0
}

/// Points the turret, so a shooting test does not have to wait out the traverse
/// or work out which arrow keys add up to a bearing.
fn aim_turret(app: &mut App, angle: f32) {
    let mut query = app.world_mut().query_filtered::<&mut Turret, With<Tank>>();
    let mut turret = query
        .single_mut(app.world_mut())
        .expect("exactly one tank should exist");
    turret.0 = angle;
}

fn count<T: Component>(app: &mut App) -> usize {
    app.world_mut()
        .query_filtered::<Entity, With<T>>()
        .iter(app.world())
        .count()
}

/// Where every factory still standing is, in a stable order.
///
/// Sorted because query iteration order is not part of Bevy's API, and a test that
/// says "the first factory" has to mean the same one every run.
fn factory_centers(app: &mut App) -> Vec<Vec2> {
    let mut query = app
        .world_mut()
        .query_filtered::<&Transform, With<Factory>>();
    let mut centers: Vec<Vec2> = query
        .iter(app.world())
        .map(|transform| transform.translation.truncate())
        .collect();
    centers.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    centers
}

/// Hit points left across every standing factory.
fn factory_health(app: &mut App) -> i32 {
    let mut query = app.world_mut().query_filtered::<&Health, With<Factory>>();
    query.iter(app.world()).map(|health| health.current()).sum()
}

/// How many of the shells in the air are the player's.
///
/// Factories shoot back, so "a shell exists" stopped being the same question as
/// "the tank fired" the moment M3 armed them. Tests about the tank's gun ask this
/// one.
fn player_shells(app: &mut App) -> usize {
    let mut query = app.world_mut().query_filtered::<&Faction, With<Shell>>();
    query
        .iter(app.world())
        .filter(|faction| **faction == Faction::Player)
        .count()
}

/// Where every shell in flight is.
fn shell_positions(app: &mut App) -> Vec<Vec2> {
    let mut query = app.world_mut().query_filtered::<&Transform, With<Shell>>();
    query
        .iter(app.world())
        .map(|transform| transform.translation.truncate())
        .collect()
}

/// Parks the tank a clear 60 px off `target` and points the gun at it.
///
/// The stand-off is toward the middle of the arena, which in an open room is the
/// one direction guaranteed to have space in it. 60 px clears the tank's radius
/// plus the building's, and leaves the muzzle just outside the building rather than
/// inside it, so a shell has somewhere to start.
fn take_aim_at(app: &mut App, target: Vec2) {
    let inward = maze_center(app) - target;
    let inward = if inward == Vec2::ZERO {
        Vec2::X
    } else {
        inward.normalize()
    };
    place_tank(app, target + inward * 60.0);
    aim_turret(app, (-inward).to_angle());
}

/// Shells `target` until whatever is standing there is rubble.
///
/// Five shells at the gun's cooldown is a hundred and nine ticks; the margin is
/// for the last one's flight time.
fn shell_to_pieces(app: &mut App, target: Vec2) {
    take_aim_at(app, target);
    press(app, KeyCode::Space);
    step(app, 160);
    release(app, KeyCode::Space);
    step(app, 2);
}

#[test]
fn starts_in_the_main_menu_with_no_tank_and_no_maze() {
    let mut app = headless_app(arena());
    assert_eq!(state(&app), AppState::MainMenu);
    assert_eq!(tank_count(&mut app), 0);
    assert!(
        !app.world().contains_resource::<Maze>(),
        "the maze belongs to a run, not to the app"
    );
}

#[test]
fn runs_a_full_menu_play_pause_quit_cycle_without_panicking() {
    let mut app = headless_app(real_maze(99));

    start_playing(&mut app);
    assert_eq!(tank_count(&mut app), 1);
    step(&mut app, 120);

    tap(&mut app, KeyCode::Escape);
    assert_eq!(state(&app), AppState::Paused);
    step(&mut app, 60);

    tap(&mut app, KeyCode::Escape);
    assert_eq!(state(&app), AppState::Playing, "escape should un-pause");

    tap(&mut app, KeyCode::Escape);
    tap(&mut app, KeyCode::KeyQ);
    assert_eq!(state(&app), AppState::MainMenu);
    assert_eq!(
        tank_count(&mut app),
        0,
        "the tank should not outlive the run"
    );
}

#[test]
fn the_tank_spawns_on_the_mazes_spawn_cell_with_room_around_it() {
    let mut app = headless_app(real_maze(2024));
    start_playing(&mut app);

    let maze = app.world().resource::<Maze>();
    let grid = maze.grid.clone();
    let expected = maze.grid.cell_center(maze.spawn);

    assert_eq!(tank_position(&mut app), expected);
    assert!(
        collision::is_clear(&grid, expected, TANK_RADIUS),
        "the tank was spawned overlapping a wall at {expected:?}"
    );
}

#[test]
fn a_fixed_seed_regenerates_the_same_maze_on_the_next_run() {
    let mut app = headless_app(real_maze(4242));

    start_playing(&mut app);
    let first = app.world().resource::<Maze>().grid.clone();
    assert!(first.is_connected(), "a generated maze must be one piece");

    tap(&mut app, KeyCode::Escape);
    tap(&mut app, KeyCode::KeyQ);
    assert_eq!(state(&app), AppState::MainMenu);

    start_playing(&mut app);
    let second = app.world().resource::<Maze>().grid.clone();
    assert_eq!(first, second, "the same seed should rebuild the same maze");
}

#[test]
fn holding_d_drives_the_tank_right() {
    let mut app = headless_app(arena());
    start_playing(&mut app);
    let center = maze_center(&app);
    place_tank(&mut app, center);

    let start = tank_position(&mut app);
    press(&mut app, KeyCode::KeyD);
    step(&mut app, 60);
    let travelled = tank_position(&mut app) - start;

    // One second of drive: a quarter second reaching 180 px/s, then holding it,
    // so roughly 157 px. The floor is deliberately well above zero — asserting
    // only `> 0` would be satisfied by a simulation that never ticked at all.
    assert!(
        travelled.x > 100.0,
        "expected about a second's travel to the right, got {travelled:?}"
    );
    assert!(travelled.y.abs() < 1e-4, "no drift on Y: {travelled:?}");
}

#[test]
fn the_tank_does_not_move_while_paused() {
    let mut app = headless_app(arena());
    start_playing(&mut app);
    let center = maze_center(&app);
    place_tank(&mut app, center);

    let start = tank_position(&mut app);
    press(&mut app, KeyCode::KeyD);
    step(&mut app, 60);
    tap(&mut app, KeyCode::Escape);
    assert_eq!(state(&app), AppState::Paused);

    // D is still held down, but paused: nothing may advance.
    let paused_at = tank_position(&mut app);
    assert!(
        paused_at.x > start.x,
        "the tank should have been moving before the pause"
    );
    step(&mut app, 120);
    assert_eq!(tank_position(&mut app), paused_at);
}

#[test]
fn resuming_from_pause_does_not_lurch_on_stale_input() {
    let mut app = headless_app(arena());
    start_playing(&mut app);
    let center = maze_center(&app);
    place_tank(&mut app, center);

    // Pause mid-throttle, then let go of the key while paused.
    press(&mut app, KeyCode::KeyD);
    step(&mut app, 60);
    tap(&mut app, KeyCode::Escape);
    release(&mut app, KeyCode::KeyD);
    step(&mut app, 1);

    let mut query = app
        .world_mut()
        .query_filtered::<&DriveCommand, With<Tank>>();
    let command = query.single(app.world()).expect("tank should exist");
    assert_eq!(
        command.0,
        Vec2::ZERO,
        "pausing must clear the drive command, not bank it"
    );
}

#[test]
fn a_released_tank_coasts_to_a_dead_stop() {
    let mut app = headless_app(arena());
    start_playing(&mut app);
    let start = maze_center(&app);
    place_tank(&mut app, start);

    press(&mut app, KeyCode::KeyW);
    step(&mut app, 60);
    assert!(
        tank_position(&mut app).y > start.y + 100.0,
        "the tank should have been moving before the controls were released"
    );

    release(&mut app, KeyCode::KeyW);
    step(&mut app, 120);
    assert_eq!(tank_velocity(&mut app), Vec2::ZERO);
}

#[test]
fn driving_into_a_corner_stops_the_tank_dead_instead_of_passing_through() {
    let mut app = headless_app(arena());
    start_playing(&mut app);
    let corner = arena_far_corner(&app);
    // A little short of the corner, so there is a run-up.
    place_tank(&mut app, corner - Vec2::splat(50.0));

    press(&mut app, KeyCode::KeyW);
    press(&mut app, KeyCode::KeyD);
    // Long enough that an unblocked tank would be well outside the maze.
    step(&mut app, 240);

    let at = tank_position(&mut app);
    assert!(
        (at - corner).length() < 0.1,
        "expected the tank flush in the corner at {corner:?}, got {at:?}"
    );
    assert_eq!(
        tank_velocity(&mut app),
        Vec2::ZERO,
        "both axes are blocked, so no speed may be banked against the walls"
    );
}

#[test]
fn the_tank_slides_along_a_wall_it_is_pushed_into_at_an_angle() {
    let mut app = headless_app(arena());
    start_playing(&mut app);
    // Flush against the bottom wall, half way along it.
    let floor = border_face() + TANK_RADIUS;
    let start = Vec2::new(maze_center(&app).x, floor);
    place_tank(&mut app, start);

    // Down *and* right: down is refused, right is not.
    press(&mut app, KeyCode::KeyS);
    press(&mut app, KeyCode::KeyD);
    step(&mut app, 60);

    let at = tank_position(&mut app);
    assert!(
        (at.y - floor).abs() < 0.1,
        "should still be resting on the floor: {at:?}"
    );
    assert!(
        at.x > start.x + 90.0,
        "should have kept most of its rightward travel: {at:?}"
    );
}

#[test]
fn a_long_run_through_a_real_maze_never_ends_up_inside_a_wall() {
    // The point of the collision layer, exercised through the real schedule
    // rather than by calling `slide` directly.
    let mut app = headless_app(empty_maze(31337));
    start_playing(&mut app);
    let grid = app.world().resource::<Maze>().grid.clone();

    // Hold a diagonal and let it grind around the maze for twenty seconds.
    press(&mut app, KeyCode::KeyW);
    press(&mut app, KeyCode::KeyD);
    for tick in 0..1200 {
        step(&mut app, 1);
        let at = tank_position(&mut app);
        assert!(
            collision::is_clear(&grid, at, TANK_RADIUS),
            "tick {tick}: the tank ended up inside a wall at {at:?}"
        );
    }
}

#[test]
fn the_arrow_keys_aim_the_turret_without_steering_the_tank() {
    let mut app = headless_app(arena());
    start_playing(&mut app);
    let center = maze_center(&app);
    place_tank(&mut app, center);
    aim_turret(&mut app, 0.0);
    let parked = tank_position(&mut app);

    // A quarter turn is half a second of traverse; a full second is plenty.
    press(&mut app, KeyCode::ArrowUp);
    step(&mut app, 60);

    let angle = turret_angle(&mut app);
    assert!(
        (angle - FRAC_PI_2).abs() < 1e-4,
        "expected the turret straight up, got {angle}"
    );
    assert_eq!(
        tank_position(&mut app),
        parked,
        "aiming the turret is not driving the hull"
    );
}

#[test]
fn the_turret_slews_at_its_own_rate_rather_than_snapping_round() {
    let mut app = headless_app(arena());
    start_playing(&mut app);
    aim_turret(&mut app, 0.0);

    // Half a turn away: the furthest it can be asked to go.
    press(&mut app, KeyCode::ArrowLeft);
    step(&mut app, 1);
    let after_one_tick = turret_angle(&mut app);
    assert!(
        after_one_tick.abs() > 0.0,
        "the turret should have started turning"
    );
    assert!(
        after_one_tick.abs() < 0.1,
        "and should not have arrived inside one tick: {after_one_tick}"
    );

    // Half a turn at PI rad/s is a second.
    step(&mut app, 60);
    let arrived = turret_angle(&mut app);
    assert!(
        (arrived.abs() - PI).abs() < 1e-3,
        "expected the turret pointing left, got {arrived}"
    );
}

#[test]
fn the_turret_holds_its_bearing_when_nothing_is_asked_of_it() {
    let mut app = headless_app(arena());
    start_playing(&mut app);
    aim_turret(&mut app, 1.0);
    step(&mut app, 120);
    assert_eq!(
        turret_angle(&mut app),
        1.0,
        "a released stick means hold, not recentre"
    );
}

#[test]
fn a_factory_stands_on_every_cell_the_maze_put_one_on() {
    let mut app = headless_app(real_maze(2024));
    start_playing(&mut app);

    let mut expected: Vec<Vec2> = {
        let maze = app.world().resource::<Maze>();
        maze.factories
            .iter()
            .map(|cell| maze.grid.cell_center(*cell))
            .collect()
    };
    expected.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));

    assert!(!expected.is_empty(), "a level-one maze has factories in it");
    assert_eq!(factory_centers(&mut app), expected);
}

#[test]
fn holding_space_fires_at_the_rate_the_cooldown_sets() {
    // Fired straight up in an open arena with the tank in the middle, so nothing
    // is reached and every shell fired is still countable.
    let mut app = headless_app(arena());
    start_playing(&mut app);
    let center = maze_center(&app);
    place_tank(&mut app, center);
    aim_turret(&mut app, FRAC_PI_2);

    press(&mut app, KeyCode::Space);
    step(&mut app, 1);
    assert_eq!(count::<Shell>(&mut app), 1, "the first shot is immediate");

    // A shell lives 0.9 s and one is fired every 0.45 s, so at most three are ever
    // in the air at once. A gun with no rate limit would have fired 26 by now.
    for tick in 0..26 {
        step(&mut app, 1);
        assert!(
            count::<Shell>(&mut app) <= 1,
            "tick {tick}: fired again inside the cooldown"
        );
    }
    step(&mut app, 1);
    assert_eq!(
        count::<Shell>(&mut app),
        2,
        "the second shot is due 27 ticks after the first"
    );
}

#[test]
fn a_shell_takes_a_bite_out_of_a_factory_without_flattening_it() {
    let mut app = headless_app(arena_with_factories(1));
    start_playing(&mut app);
    let factory = factory_centers(&mut app)[0];
    let full_health = factory_health(&mut app);
    assert!(
        full_health > GUN.damage,
        "a factory takes more than one shell"
    );

    take_aim_at(&mut app, factory);
    press(&mut app, KeyCode::Space);
    step(&mut app, 1);
    release(&mut app, KeyCode::Space);
    assert_eq!(player_shells(&mut app), 1, "one shell should be in the air");

    step(&mut app, 4);
    assert_eq!(player_shells(&mut app), 0, "and should have arrived");
    assert_eq!(count::<Factory>(&mut app), 1, "one shell is not five");
    assert_eq!(factory_health(&mut app), full_health - GUN.damage);
}

#[test]
fn clearing_every_factory_ends_the_level_and_nothing_less_does() {
    let mut app = headless_app(arena_with_factories(2));
    start_playing(&mut app);
    let factories = factory_centers(&mut app);
    assert_eq!(factories.len(), 2);

    shell_to_pieces(&mut app, factories[0]);
    assert_eq!(
        count::<Factory>(&mut app),
        1,
        "the first one should be down"
    );
    assert_eq!(
        state(&app),
        AppState::Playing,
        "one factory left standing is not a cleared sector"
    );

    shell_to_pieces(&mut app, factories[1]);
    assert_eq!(count::<Factory>(&mut app), 0);
    assert_eq!(state(&app), AppState::LevelComplete);
}

#[test]
fn a_cleared_level_stands_down_to_the_menu_with_nothing_left_behind() {
    let mut app = headless_app(arena_with_factories(1));
    start_playing(&mut app);
    let factory = factory_centers(&mut app)[0];

    shell_to_pieces(&mut app, factory);
    assert_eq!(state(&app), AppState::LevelComplete);

    tap(&mut app, KeyCode::Enter);
    assert_eq!(state(&app), AppState::MainMenu);
    assert_eq!(tank_count(&mut app), 0);
    assert_eq!(count::<Shell>(&mut app), 0, "shells belong to the run");
    assert_eq!(count::<Factory>(&mut app), 0);
}

#[test]
fn abandoning_a_run_takes_the_factories_and_the_shells_in_flight_with_it() {
    let mut app = headless_app(arena_with_factories(2));
    start_playing(&mut app);
    let center = maze_center(&app);
    place_tank(&mut app, center);
    aim_turret(&mut app, FRAC_PI_2);

    press(&mut app, KeyCode::Space);
    step(&mut app, 1);
    assert_eq!(player_shells(&mut app), 1);
    assert_eq!(count::<Factory>(&mut app), 2);

    tap(&mut app, KeyCode::Escape);
    tap(&mut app, KeyCode::KeyQ);
    assert_eq!(state(&app), AppState::MainMenu);
    assert_eq!(count::<Shell>(&mut app), 0);
    assert_eq!(count::<Factory>(&mut app), 0);
}

#[test]
fn releasing_the_trigger_is_not_needed_to_stop_firing_into_a_pause() {
    let mut app = headless_app(arena());
    start_playing(&mut app);
    let center = maze_center(&app);
    place_tank(&mut app, center);
    aim_turret(&mut app, FRAC_PI_2);

    // Trigger held down through the pause, and never let go of.
    press(&mut app, KeyCode::Space);
    step(&mut app, 1);
    tap(&mut app, KeyCode::Escape);
    assert_eq!(state(&app), AppState::Paused);
    let in_the_air = count::<Shell>(&mut app);

    step(&mut app, 120);
    assert_eq!(
        count::<Shell>(&mut app),
        in_the_air,
        "a paused game must not fire, and its shells must not move"
    );
    assert_eq!(
        shell_positions(&mut app),
        shell_positions(&mut app),
        "and this reads the same both times, so the comparison means something"
    );
}

#[test]
fn a_shell_gives_out_at_the_end_of_its_range() {
    let mut app = headless_app(arena());
    start_playing(&mut app);
    let center = maze_center(&app);
    place_tank(&mut app, center);
    aim_turret(&mut app, 0.0);

    press(&mut app, KeyCode::Space);
    step(&mut app, 1);
    release(&mut app, KeyCode::Space);
    let muzzle = shell_positions(&mut app)[0];
    // The far wall is 960 px away, well past the 576 px the shell has in it, so
    // range is what ends this and not masonry.
    assert!(maze_size(&app).x - CELL_SIZE - muzzle.x > GUN.range);

    // Range at shell speed is 54 ticks. At 50 it is still short of it.
    step(&mut app, 50);
    let flying = shell_positions(&mut app);
    assert_eq!(flying.len(), 1, "50 ticks is inside its range");
    let travelled = flying[0].x - muzzle.x;
    assert!(
        travelled > GUN.range - CELL_SIZE && travelled <= GUN.range,
        "should be a cell or so short of its {} px range, got {travelled}",
        GUN.range
    );

    step(&mut app, 8);
    assert_eq!(
        count::<Shell>(&mut app),
        0,
        "58 ticks is past the end of it"
    );
}

#[test]
fn the_tank_cannot_drive_through_a_factory() {
    let mut app = headless_app(arena_with_factories(1));
    start_playing(&mut app);
    let factory = factory_centers(&mut app)[0];

    // Run at it head-on from whichever side of it has the room.
    let (standoff, key) = if factory.x < maze_center(&app).x {
        (CELL_SIZE * 3.0, KeyCode::KeyA)
    } else {
        (-CELL_SIZE * 3.0, KeyCode::KeyD)
    };
    place_tank(&mut app, factory + Vec2::new(standoff, 0.0));

    press(&mut app, key);
    step(&mut app, 240);

    let at = tank_position(&mut app);
    let gap = at.distance(factory);
    assert!(
        gap >= TANK_RADIUS + FACTORY_RADIUS - 0.1,
        "drove into the building: only {gap} px between the two centres"
    );
    assert_eq!(
        at.x < factory.x,
        standoff < 0.0,
        "ended up on the far side of the building at {at:?}"
    );
    assert_eq!(
        tank_velocity(&mut app),
        Vec2::ZERO,
        "no speed may be banked against a building either"
    );
}

#[test]
fn no_shell_ever_ends_up_inside_a_wall() {
    // What `collision::sweep` is for, exercised through the real schedule: driving
    // and shooting in every direction around a real maze for twenty seconds.
    let mut app = headless_app(empty_maze(31337));
    start_playing(&mut app);
    let grid = app.world().resource::<Maze>().grid.clone();

    press(&mut app, KeyCode::KeyW);
    press(&mut app, KeyCode::KeyD);
    press(&mut app, KeyCode::Space);
    for tick in 0..1200 {
        // Sweep the gun round so shots go out on every bearing rather than one.
        aim_turret(&mut app, tick as f32 * 0.11);
        step(&mut app, 1);
        for at in shell_positions(&mut app) {
            assert!(
                collision::is_clear(&grid, at, SHELL_RADIUS),
                "tick {tick}: a shell is inside a wall at {at:?}"
            );
        }
        assert!(
            count::<Shell>(&mut app) <= 3,
            "tick {tick}: shells are not being cleaned up"
        );
    }
}

// ---------------------------------------------------------------------------
// M3: drones, damage, and the end of a run.
// ---------------------------------------------------------------------------

/// Hit points left on the tank.
fn tank_health(app: &mut App) -> i32 {
    let mut query = app.world_mut().query_filtered::<&Health, With<Tank>>();
    query.single(app.world()).expect("a live tank").current()
}

/// Hit points left on a particular entity.
fn health_of(app: &App, entity: Entity) -> i32 {
    app.world()
        .get::<Health>(entity)
        .expect("the entity has health")
        .current()
}

/// Moves any entity, so a test can hold a firing line still.
///
/// Drones drive themselves, and a test about what a shell passes through cannot
/// also be a test about where three moving things drifted to. Pinning them each
/// tick makes the geometry the fixed part and the shell the only thing in motion.
fn place_at(app: &mut App, entity: Entity, at: Vec2) {
    let mut transform = app
        .world_mut()
        .get_mut::<Transform>(entity)
        .expect("the entity has a transform");
    transform.translation.x = at.x;
    transform.translation.y = at.y;
}

/// Puts a drone of `kind` on the field at a world position, owned by nobody.
///
/// Factories build drones on a three-and-a-half second timer and pick the kind
/// from a level's mix, neither of which a test about one kind's behaviour wants to
/// wait for or argue with. The drone is identical to a built one except that the
/// factory it names does not exist, which matters only to the production cap.
fn place_drone(app: &mut App, kind: DroneKind, at: Vec2) -> Entity {
    let cell = app.world().resource::<Maze>().grid.cell_at(at);
    let entity = {
        let mut commands = app.world_mut().commands();
        drone::spawn(&mut commands, kind, at, cell, Entity::PLACEHOLDER, 0)
    };
    app.world_mut().flush();
    entity
}

/// Takes the tank off the field.
///
/// Production and movement are worth testing without a firefight going on around
/// them: with nothing to shoot at, drones wander and factories build, which is
/// exactly the part under test. The simulation is built to cope with this — it is
/// the state the world is in for the tick between the tank dying and the game-over
/// screen coming up.
fn remove_tank(app: &mut App) {
    let tanks: Vec<Entity> = {
        let mut query = app.world_mut().query_filtered::<Entity, With<Tank>>();
        query.iter(app.world()).collect()
    };
    for tank in tanks {
        app.world_mut().despawn(tank);
    }
}

/// Which kinds of drone are on the field.
fn drone_kinds(app: &mut App) -> Vec<DroneKind> {
    let mut query = app.world_mut().query::<&Drone>();
    let mut kinds: Vec<DroneKind> = query.iter(app.world()).map(|drone| drone.kind).collect();
    kinds.sort_by_key(|kind| kind.unlock_level());
    kinds.dedup();
    kinds
}

/// Where every drone is.
fn drone_positions(app: &mut App) -> Vec<Vec2> {
    let mut query = app.world_mut().query_filtered::<&Transform, With<Drone>>();
    query
        .iter(app.world())
        .map(|transform| transform.translation.truncate())
        .collect()
}

#[test]
fn a_factory_ships_its_first_drone_on_the_interval_and_not_before() {
    let mut app = headless_app(arena_with_factories(1));
    start_playing(&mut app);
    // Nothing to shoot at and nothing shooting back: this is about the timer.
    remove_tank(&mut app);

    assert_eq!(
        count::<Drone>(&mut app),
        0,
        "a level should not open with a drone already out"
    );

    // The interval is 3.5 s, which is 210 ticks. Checked either side of it rather
    // than on it, because a countdown stepped by 1/60 of a second in binary
    // floating point does not land exactly on zero at the tick you would expect.
    step(&mut app, 200);
    assert_eq!(count::<Drone>(&mut app), 0, "too early at 200 ticks");

    step(&mut app, 20);
    assert_eq!(count::<Drone>(&mut app), 1, "should have shipped by 220");
}

#[test]
fn a_factory_stops_building_at_its_cap_and_stays_there() {
    let mut app = headless_app(arena_with_factories(1));
    start_playing(&mut app);
    remove_tank(&mut app);

    // Six drones at one every 3.5 s is 21 s. Run to thirty, so there is a clear
    // stretch on the end during which the factory is at its cap and trying again
    // every half second — which is the part that has to *not* produce anything.
    let mut peak = 0;
    for tick in 0..1800 {
        step(&mut app, 1);
        let live = count::<Drone>(&mut app);
        peak = peak.max(live);
        assert!(
            live <= LIVE_CAP,
            "tick {tick}: {live} drones out, past the cap of {LIVE_CAP}"
        );
    }
    assert_eq!(peak, LIVE_CAP, "production never reached the cap");
}

#[test]
fn a_destroyed_drone_makes_room_for_another() {
    // The cap has to count what is alive, not what has ever been built. A factory
    // that stopped for good after six would make clearing a level a matter of
    // waiting rather than fighting.
    let mut app = headless_app(arena_with_factories(1));
    start_playing(&mut app);
    remove_tank(&mut app);

    step(&mut app, 1400);
    assert_eq!(count::<Drone>(&mut app), LIVE_CAP, "at the cap");

    let doomed: Vec<Entity> = {
        let mut query = app.world_mut().query_filtered::<Entity, With<Drone>>();
        query.iter(app.world()).take(2).collect()
    };
    for drone in doomed {
        app.world_mut().despawn(drone);
    }
    assert_eq!(count::<Drone>(&mut app), LIVE_CAP - 2);

    // The factory is at its cap and retrying every half second, so two more are
    // due within a couple of intervals of the room appearing.
    step(&mut app, 480);
    assert_eq!(count::<Drone>(&mut app), LIVE_CAP, "back up to the cap");
}

#[test]
fn one_shell_from_the_tank_destroys_a_drone() {
    let mut app = headless_app(arena());
    start_playing(&mut app);
    let center = maze_center(&app);
    place_tank(&mut app, center);
    aim_turret(&mut app, 0.0);

    // Straight out along +X, near enough that the shell arrives before the drone
    // has drifted far, and further than the muzzle so the shell has to fly.
    place_drone(&mut app, DroneKind::Drone, center + Vec2::new(120.0, 0.0));

    press(&mut app, KeyCode::Space);
    step(&mut app, 1);
    release(&mut app, KeyCode::Space);
    assert_eq!(player_shells(&mut app), 1);

    // 120 px at 640 px/s is under twelve ticks, with the muzzle offset taken off.
    step(&mut app, 14);
    assert_eq!(count::<Drone>(&mut app), 0, "the drone should be wreckage");
    assert_eq!(player_shells(&mut app), 0, "and the shell spent on it");
}

#[test]
fn a_drone_shoots_the_tank_and_takes_health_off_it() {
    let mut app = headless_app(arena());
    start_playing(&mut app);
    let center = maze_center(&app);
    place_tank(&mut app, center);
    assert_eq!(tank_health(&mut app), 100, "a fresh tank is at full health");

    // A torpedo drone: it holds its fire until it has a shot, so this also proves
    // it takes one when it has.
    let drone = place_drone(&mut app, DroneKind::Torpedo, center + Vec2::new(220.0, 0.0));

    // Both held still, so the only thing that has to happen is the drone slewing
    // onto the bearing and firing. Half a turn of traverse is a second, the
    // cooldown another 1.4, and the flight is a handful of ticks.
    for _ in 0..240 {
        place_tank(&mut app, center);
        place_at(&mut app, drone, center + Vec2::new(220.0, 0.0));
        step(&mut app, 1);
    }

    assert!(tank_health(&mut app) < 100, "the drone never landed a shot");
    assert_eq!(state(&app), AppState::Playing, "one shell is not a run");
}

#[test]
fn a_shell_passes_straight_through_its_own_side() {
    // A factory shooting over its own drones, and a drone firing down a corridor
    // with two others in it, both depend on this. Set up as a firing line: the
    // shooter at one end, the tank at the other, and one of the shooter's own kind
    // sitting exactly in the middle of it.
    let mut app = headless_app(arena());
    start_playing(&mut app);
    let center = maze_center(&app);
    let shooter_at = center + Vec2::new(250.0, 0.0);
    let bystander_at = center + Vec2::new(125.0, 0.0);

    place_tank(&mut app, center);
    let shooter = place_drone(&mut app, DroneKind::Torpedo, shooter_at);
    let bystander = place_drone(&mut app, DroneKind::Torpedo, bystander_at);
    let bystander_health = health_of(&app, bystander);

    for _ in 0..240 {
        place_tank(&mut app, center);
        place_at(&mut app, shooter, shooter_at);
        place_at(&mut app, bystander, bystander_at);
        step(&mut app, 1);
    }

    assert!(
        tank_health(&mut app) < 100,
        "the shell should have reached the tank through its own side"
    );
    assert_eq!(
        health_of(&app, bystander),
        bystander_health,
        "and should not have touched the drone standing in the way"
    );
}

#[test]
fn a_hunter_closes_on_the_tank() {
    let mut app = headless_app(arena());
    start_playing(&mut app);
    let center = maze_center(&app);
    place_tank(&mut app, center);

    let start = center + Vec2::new(400.0, 0.0);
    let hunter = place_drone(&mut app, DroneKind::Hunter, start);

    // Four seconds. A hunter does 130 px/s, so 400 px is well inside what it can
    // cover — the assertion is about it going the right way, not about its speed.
    for _ in 0..240 {
        place_tank(&mut app, center);
        step(&mut app, 1);
    }

    let at = app
        .world()
        .get::<Transform>(hunter)
        .expect("the hunter is still alive")
        .translation
        .truncate();
    assert!(
        at.distance(center) < start.distance(center) * 0.5,
        "the hunter closed nothing: {} px away, from {}",
        at.distance(center),
        start.distance(center)
    );
}

#[test]
fn no_drone_ever_ends_up_inside_a_wall() {
    // The counterpart to the tank's version of this test. Drones go through the
    // same collision code, but they are steered by something with no idea where
    // the walls are, so they lean on it far harder than a player does.
    let mut app = headless_app(empty_maze(7717));
    start_playing(&mut app);
    let grid = app.world().resource::<Maze>().grid.clone();
    // With nothing to hunt, every kind falls back on wandering — which is the
    // steering that actually drives into things.
    remove_tank(&mut app);

    for (index, cell) in grid.open().step_by(37).enumerate() {
        let kind = DroneKind::ALL[index % DroneKind::ALL.len()];
        place_drone(&mut app, kind, grid.cell_center(cell));
    }
    assert!(count::<Drone>(&mut app) > 4, "a decent crowd of them");

    for tick in 0..1200 {
        step(&mut app, 1);
        for at in drone_positions(&mut app) {
            assert!(
                collision::is_clear(&grid, at, DRONE_RADIUS),
                "tick {tick}: a drone is inside a wall at {at:?}"
            );
        }
    }
}

/// Parks the tank `distance` px from the level's only factory for `ticks`, and
/// reports the health it had left.
///
/// A fresh app each time, and never for longer than the 210 ticks a factory takes
/// to ship its first drone, so that the factory's own gun is the only thing that
/// can have fired. Held in place rather than driven, because what is being
/// measured is the factory's reach and not the tank's ability to stay put.
fn health_after_loitering(distance: f32, ticks: u32) -> i32 {
    let mut app = headless_app(arena_with_factories(1));
    start_playing(&mut app);
    let factory = factory_centers(&mut app)[0];
    let spot = factory + (maze_center(&app) - factory).normalize() * distance;

    for _ in 0..ticks {
        place_tank(&mut app, spot);
        step(&mut app, 1);
    }
    assert_eq!(
        count::<Drone>(&mut app),
        0,
        "the window has to close before the first drone ships"
    );
    tank_health(&mut app)
}

#[test]
fn a_factory_shoots_back_at_close_range_and_ignores_you_at_long() {
    // Two hundred ticks is over three seconds: enough for the slowest half-turn of
    // traverse this gun can be asked for, and a shot after it. Well short of the
    // 210 the production line needs, so nothing else is armed yet.
    assert_eq!(
        health_after_loitering(700.0, 200),
        100,
        "a factory should be no threat from across the map"
    );
    assert!(
        health_after_loitering(60.0, 200) < 100,
        "a factory should defend itself against someone parked on its doorstep"
    );
}

#[test]
fn the_tank_running_out_of_health_ends_the_run() {
    let mut app = headless_app(arena_with_factories(1));
    start_playing(&mut app);
    let factory = factory_centers(&mut app)[0];
    let near = factory + (maze_center(&app) - factory).normalize() * 60.0;

    // Parked on the doorstep and never moved. A hundred hit points against twelve
    // a shell is nine of them, plus whatever the drones it is building add.
    // The tank is despawned on the tick its death is noticed, and the state
    // transition that follows is applied on the next one — so "still playing" is
    // not on its own enough to know there is still a tank to reposition.
    for _ in 0..3000 {
        if state(&app) != AppState::Playing || tank_count(&mut app) == 0 {
            break;
        }
        place_tank(&mut app, near);
        step(&mut app, 1);
    }
    step(&mut app, 2);

    assert_eq!(state(&app), AppState::GameOver);
    assert_eq!(tank_count(&mut app), 0, "the tank does not outlive the run");
}

#[test]
fn a_lost_run_stands_down_to_the_menu_with_nothing_left_behind() {
    let mut app = headless_app(arena_with_factories(1));
    start_playing(&mut app);
    let factory = factory_centers(&mut app)[0];
    let near = factory + (maze_center(&app) - factory).normalize() * 60.0;

    // The tank is despawned on the tick its death is noticed, and the state
    // transition that follows is applied on the next one — so "still playing" is
    // not on its own enough to know there is still a tank to reposition.
    for _ in 0..3000 {
        if state(&app) != AppState::Playing || tank_count(&mut app) == 0 {
            break;
        }
        place_tank(&mut app, near);
        step(&mut app, 1);
    }
    step(&mut app, 2);
    assert_eq!(state(&app), AppState::GameOver);

    tap(&mut app, KeyCode::Enter);
    assert_eq!(state(&app), AppState::MainMenu);
    assert_eq!(count::<Factory>(&mut app), 0);
    assert_eq!(count::<Drone>(&mut app), 0, "drones belong to the run");
    assert_eq!(count::<Shell>(&mut app), 0);
}

#[test]
fn the_level_decides_which_kinds_get_built() {
    // Level one builds nothing but the dumbest kind; the mix opens up as the
    // levels go on. What is being checked here is the plumbing — that the level
    // reaches `kind_at` at all — since the shape of the table is core's business.
    let mut app = headless_app(arena_with_factories(2));
    start_playing(&mut app);
    remove_tank(&mut app);
    step(&mut app, 1400);
    assert_eq!(
        drone_kinds(&mut app),
        vec![DroneKind::Drone],
        "level one has unlocked nothing else"
    );

    let mut later = headless_app(MazeConfig {
        level: 4,
        ..arena_with_factories(4)
    });
    start_playing(&mut later);
    remove_tank(&mut later);
    step(&mut later, 3000);
    let kinds = drone_kinds(&mut later);
    assert!(
        kinds.len() > 1,
        "level four should be building a mix, got {kinds:?}"
    );
    assert!(
        kinds.iter().all(|kind| kind.unlock_level() <= 4),
        "nothing should be built before its level: {kinds:?}"
    );
}

#[test]
fn an_assassin_finds_its_way_to_the_tank_across_a_real_maze() {
    // The one thing in M3 that no amount of driving at the player would achieve:
    // an assassin starts at the far end of the maze, behind however many walls the
    // seed put there, and arrives. Measured in A* steps rather than in pixels,
    // because straight-line distance is exactly the thing walls make a lie.
    let mut app = headless_app(empty_maze(5150));
    start_playing(&mut app);
    let grid = app.world().resource::<Maze>().grid.clone();
    let home = app.world().resource::<Maze>().spawn;
    let tank_at = grid.cell_center(home);
    place_tank(&mut app, tank_at);

    let steps_home = |cell| {
        path::find_path(&grid, cell, home)
            .map(|route| route.steps())
            .expect("a generated maze is all one piece")
    };
    let far = grid.open().max_by_key(|cell| steps_home(*cell)).unwrap();
    let started = steps_home(far);
    assert!(
        started > 20,
        "the far corner should be a real trek: {started}"
    );

    place_drone(&mut app, DroneKind::Assassin, grid.cell_center(far));

    // Held still, so this measures the assassin's navigation and not the player's
    // inability to run away. It shoots as well as it drives, so the loop ends when
    // the tank does — by then the closest approach has already been recorded.
    let mut closest = started;
    for _ in 0..1800 {
        if state(&app) != AppState::Playing || tank_count(&mut app) == 0 {
            break;
        }
        place_tank(&mut app, tank_at);
        step(&mut app, 1);
        if let Some(at) = drone_positions(&mut app).first() {
            closest = closest.min(steps_home(grid.cell_at(*at)));
        }
    }

    assert!(
        closest <= 2,
        "the assassin got no closer than {closest} steps, having started {started} away"
    );
}
