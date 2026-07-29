//! Headless integration smoke test.
//!
//! Builds the simulation half of the app on `MinimalPlugins` — no window, no
//! renderer, no GPU — and steps it through the state machine. This is the test
//! that catches system-ordering and schedule mistakes in CI, where there is no
//! display to open.

use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::{ButtonState, InputPlugin};
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use dinochrome_game::player::{DriveCommand, Tank, Velocity};
use dinochrome_game::{AppState, SimPlugin};

/// A simulation-only app: everything `SimPlugin` needs and nothing else.
fn headless_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, InputPlugin, SimPlugin));
    // One update to let plugin setup and the initial state transition settle.
    app.update();
    app
}

/// Steps the app, forcing `ticks` fixed updates to run regardless of wall-clock
/// time, so the test does not depend on how fast the machine is.
fn step(app: &mut App, ticks: u32) {
    for _ in 0..ticks {
        app.world_mut().resource_mut::<Time<Fixed>>().advance_by(
            std::time::Duration::from_secs_f64(1.0 / dinochrome_core::FIXED_HZ),
        );
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

fn tank_count(app: &mut App) -> usize {
    app.world_mut()
        .query_filtered::<Entity, With<Tank>>()
        .iter(app.world())
        .count()
}

#[test]
fn starts_in_the_main_menu_with_no_tank() {
    let mut app = headless_app();
    assert_eq!(state(&app), AppState::MainMenu);
    assert_eq!(tank_count(&mut app), 0);
}

#[test]
fn runs_a_full_menu_play_pause_quit_cycle_without_panicking() {
    let mut app = headless_app();

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
fn holding_d_drives_the_tank_right() {
    let mut app = headless_app();
    start_playing(&mut app);

    let start = tank_position(&mut app);
    press(&mut app, KeyCode::KeyD);
    step(&mut app, 60);
    let moved = tank_position(&mut app);

    assert!(
        moved.x > start.x,
        "expected rightward travel, got {moved:?}"
    );
    assert!((moved.y - start.y).abs() < 1e-4, "no drift on Y: {moved:?}");
}

#[test]
fn the_tank_does_not_move_while_paused() {
    let mut app = headless_app();
    start_playing(&mut app);

    press(&mut app, KeyCode::KeyD);
    step(&mut app, 60);
    tap(&mut app, KeyCode::Escape);
    assert_eq!(state(&app), AppState::Paused);

    // D is still held down, but paused: nothing may advance.
    let paused_at = tank_position(&mut app);
    step(&mut app, 120);
    assert_eq!(tank_position(&mut app), paused_at);
}

#[test]
fn resuming_from_pause_does_not_lurch_on_stale_input() {
    let mut app = headless_app();
    start_playing(&mut app);

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
    let mut app = headless_app();
    start_playing(&mut app);

    press(&mut app, KeyCode::KeyW);
    step(&mut app, 60);
    release(&mut app, KeyCode::KeyW);
    step(&mut app, 120);

    let mut query = app.world_mut().query_filtered::<&Velocity, With<Tank>>();
    let velocity = query.single(app.world()).expect("tank should exist");
    assert_eq!(velocity.0, Vec2::ZERO);
}
