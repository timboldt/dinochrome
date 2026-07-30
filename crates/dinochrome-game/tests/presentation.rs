//! Headless test of the presentation wiring.
//!
//! `tests/smoke.rs` drives [`SimPlugin`] alone, which leaves the parts of the app
//! that only exist to be looked at completely unexercised — and those are exactly
//! where the mistakes that a compiler cannot catch live: two systems in different
//! plugins racing in the same schedule, a `&mut Transform` query that conflicts
//! with another one, a system reading a resource that its ordering does not
//! actually guarantee exists yet.
//!
//! None of that needs a GPU to go wrong. Spawning a `Sprite` or a `Node` is just
//! inserting components, and Bevy validates a system's world access when the
//! schedule containing it first runs, whether or not anything is drawn. So this
//! builds the whole [`DinochromePlugin`] on `MinimalPlugins` and walks it through
//! a full run. Nothing renders; everything is checked.
//!
//! [`SimPlugin`]: dinochrome_game::SimPlugin

use bevy::asset::AssetPlugin;
use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::{ButtonState, InputPlugin};
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use dinochrome_core::maze::MazeParams;
use dinochrome_game::maze::{Maze, MazeConfig, MazeWall};
use dinochrome_game::menu::{MainMenuUi, PauseUi};
use dinochrome_game::player::Tank;
use dinochrome_game::{AppState, DinochromePlugin};

/// The whole game, minus a window and a renderer.
fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        InputPlugin,
        // `Sprite` and the UI text nodes hold `Handle`s, so the asset server has
        // to exist even though nothing is ever loaded from it.
        AssetPlugin::default(),
        DinochromePlugin,
    ))
    .insert_resource(MazeConfig {
        params: MazeParams::LEVEL_ONE,
        seed: Some(20260729),
    })
    .insert_resource(TimeUpdateStrategy::FixedTimesteps(1));
    app.update();
    app
}

fn step(app: &mut App, ticks: u32) {
    for _ in 0..ticks {
        app.update();
    }
}

fn tap(app: &mut App, key_code: KeyCode) {
    for state in [ButtonState::Pressed, ButtonState::Released] {
        app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
        step(app, 1);
    }
}

fn state(app: &App) -> AppState {
    *app.world().resource::<State<AppState>>().get()
}

fn count<T: Component>(app: &mut App) -> usize {
    app.world_mut()
        .query_filtered::<Entity, With<T>>()
        .iter(app.world())
        .count()
}

#[test]
fn the_menu_is_up_and_the_camera_exists_before_anything_else_does() {
    let mut app = app();
    assert_eq!(state(&app), AppState::MainMenu);
    assert_eq!(count::<Camera2d>(&mut app), 1, "exactly one camera");
    assert_eq!(count::<MainMenuUi>(&mut app), 1);
    assert_eq!(count::<MazeWall>(&mut app), 0, "no maze before a run");
}

#[test]
fn starting_a_run_draws_one_sprite_per_wall_cell() {
    let mut app = app();
    tap(&mut app, KeyCode::Enter);
    assert_eq!(state(&app), AppState::Playing);

    let expected = app.world().resource::<Maze>().grid.walls().count();
    assert!(expected > 0, "a level-one maze has walls");
    assert_eq!(
        count::<MazeWall>(&mut app),
        expected,
        "every wall cell should have been drawn exactly once"
    );
    assert_eq!(count::<MainMenuUi>(&mut app), 0, "the menu should be gone");
}

#[test]
fn the_tank_gets_its_sprite_attached_exactly_once() {
    let mut app = app();
    tap(&mut app, KeyCode::Enter);
    step(&mut app, 30);

    let mut query = app
        .world_mut()
        .query_filtered::<&Sprite, (With<Tank>, With<Sprite>)>();
    assert_eq!(
        query.iter(app.world()).count(),
        1,
        "the tank should end up with one sprite and keep it"
    );
}

#[test]
fn the_pause_overlay_comes_and_goes_without_disturbing_the_maze() {
    let mut app = app();
    tap(&mut app, KeyCode::Enter);
    let walls = count::<MazeWall>(&mut app);

    tap(&mut app, KeyCode::Escape);
    assert_eq!(state(&app), AppState::Paused);
    assert_eq!(count::<PauseUi>(&mut app), 1);
    assert_eq!(count::<MazeWall>(&mut app), walls, "the maze stays drawn");

    tap(&mut app, KeyCode::Escape);
    assert_eq!(state(&app), AppState::Playing);
    assert_eq!(count::<PauseUi>(&mut app), 0, "the overlay should be gone");
    assert_eq!(count::<MazeWall>(&mut app), walls);
}

#[test]
fn abandoning_a_run_clears_the_drawn_maze_and_the_next_run_redraws_it() {
    let mut app = app();

    tap(&mut app, KeyCode::Enter);
    let walls = count::<MazeWall>(&mut app);
    assert!(walls > 0);

    tap(&mut app, KeyCode::Escape);
    tap(&mut app, KeyCode::KeyQ);
    assert_eq!(state(&app), AppState::MainMenu);
    assert_eq!(
        count::<MazeWall>(&mut app),
        0,
        "wall sprites must not survive the run that created them"
    );
    assert_eq!(count::<Tank>(&mut app), 0);

    // The seed is fixed, so the second run is the same maze — and must not end
    // up drawn twice over.
    tap(&mut app, KeyCode::Enter);
    assert_eq!(count::<MazeWall>(&mut app), walls);
}

#[test]
fn a_full_run_survives_being_stepped_with_every_system_live() {
    // The catch-all: every schedule in the game actually runs, so every system's
    // world access and ordering gets validated.
    let mut app = app();
    tap(&mut app, KeyCode::Enter);

    app.world_mut().write_message(KeyboardInput {
        key_code: KeyCode::KeyD,
        logical_key: Key::Unidentified(NativeKey::Unidentified),
        state: ButtonState::Pressed,
        text: None,
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
    step(&mut app, 300);
    assert_eq!(state(&app), AppState::Playing);
}
