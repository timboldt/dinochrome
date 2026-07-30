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

use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::{ButtonState, InputPlugin};
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use dinochrome_core::maze::MazeParams;
use dinochrome_core::{CELL_SIZE, collision};
use dinochrome_game::maze::{Maze, MazeConfig};
use dinochrome_game::player::{DriveCommand, Tank, Velocity};
use dinochrome_game::{AppState, SimPlugin};

/// The tank's collider radius, as `player` sets it.
const TANK_RADIUS: f32 = 20.0;

/// A maze whose whole interior is open: a room with a wall around it.
///
/// Movement tests need somewhere to drive without the seed deciding how far they
/// get.
fn arena() -> MazeConfig {
    MazeConfig {
        params: MazeParams {
            density: 0.0,
            ..MazeParams::LEVEL_ONE
        },
        seed: Some(1),
    }
}

/// An ordinary level-one maze, from a fixed seed.
fn real_maze(seed: u64) -> MazeConfig {
    MazeConfig {
        params: MazeParams::LEVEL_ONE,
        seed: Some(seed),
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

/// Where the tank comes to rest flush in the top-right corner of an open arena.
///
/// The border ring is one cell thick, so the last open cell ends one cell in.
fn arena_far_corner(app: &App) -> Vec2 {
    maze_size(app) - Vec2::splat(CELL_SIZE + TANK_RADIUS)
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
    let floor = CELL_SIZE + TANK_RADIUS;
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
    let mut app = headless_app(real_maze(31337));
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
