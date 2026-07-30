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

use std::f32::consts::FRAC_PI_2;

use bevy::asset::AssetPlugin;
use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::{ButtonState, InputPlugin};
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use dinochrome_core::maze::MazeParams;
use dinochrome_game::factory::{Factory, FactoryCore};
use dinochrome_game::maze::{Maze, MazeConfig, MazeWall};
use dinochrome_game::menu::{LevelCompleteUi, MainMenuUi, PauseUi};
use dinochrome_game::player::Tank;
use dinochrome_game::turret::{Barrel, Turret};
use dinochrome_game::weapon::{Health, Shell};
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

fn tap(app: &mut App, key_code: KeyCode) {
    for state in [ButtonState::Pressed, ButtonState::Released] {
        key(app, key_code, state);
        step(app, 1);
    }
}

fn press(app: &mut App, key_code: KeyCode) {
    key(app, key_code, ButtonState::Pressed);
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
fn every_factory_is_drawn_as_a_body_with_a_core_in_it() {
    let mut app = app();
    tap(&mut app, KeyCode::Enter);
    step(&mut app, 2);

    let expected = app.world().resource::<Maze>().factories.len();
    assert!(expected > 0, "a level-one maze has factories in it");
    assert_eq!(count::<Factory>(&mut app), expected);
    assert_eq!(count::<FactoryCore>(&mut app), expected, "one core each");

    // The body sprite goes on the factory entity itself, so a factory without one
    // would be an invisible thing to shoot at.
    let mut query = app
        .world_mut()
        .query_filtered::<&Sprite, (With<Factory>, With<Sprite>)>();
    assert_eq!(query.iter(app.world()).count(), expected);
}

#[test]
fn a_factory_darkens_as_it_takes_damage() {
    let mut app = app();
    tap(&mut app, KeyCode::Enter);
    step(&mut app, 2);

    let full_health = factory_colour(&mut app);
    damage_a_factory(&mut app, 60);
    step(&mut app, 1);
    let hurt = factory_colour(&mut app);

    assert_ne!(
        hurt, full_health,
        "damage is the only feedback M2 has that a shell connected"
    );
    assert!(
        luminance(hurt) < luminance(full_health),
        "and it should be getting darker, not brighter: {full_health:?} to {hurt:?}"
    );
}

#[test]
fn the_tank_gets_one_barrel_and_it_points_where_the_turret_does() {
    let mut app = app();
    tap(&mut app, KeyCode::Enter);
    step(&mut app, 2);
    assert_eq!(count::<Barrel>(&mut app), 1, "exactly one barrel");

    // A barrel that is not a child of the tank would not travel with it.
    let tank = {
        let mut query = app.world_mut().query_filtered::<Entity, With<Tank>>();
        query.single(app.world()).expect("one tank")
    };
    let mut query = app.world_mut().query_filtered::<&ChildOf, With<Barrel>>();
    let parent = query.single(app.world()).expect("one barrel").parent();
    assert_eq!(parent, tank);

    aim_turret(&mut app, FRAC_PI_2);
    step(&mut app, 1);
    let mut query = app.world_mut().query_filtered::<&Transform, With<Barrel>>();
    let rotation = query.single(app.world()).expect("one barrel").rotation;
    // The barrel is drawn along its own +X, so rotating +X by it is the bearing it
    // is showing.
    let showing = (rotation * Vec3::X).truncate().to_angle();
    assert!(
        (showing - FRAC_PI_2).abs() < 1e-4,
        "the barrel is showing {showing}, not the {FRAC_PI_2} the turret is at"
    );
}

#[test]
fn a_shell_is_drawn_on_the_frame_it_is_fired() {
    let mut app = app();
    tap(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Space);
    // One tick only. In a real maze the tank is usually pointing at a wall a
    // couple of cells away, so a shell's whole life can be over inside two.
    step(&mut app, 1);

    assert_eq!(count::<Shell>(&mut app), 1, "one shell, just fired");
    let mut query = app
        .world_mut()
        .query_filtered::<&Sprite, (With<Shell>, With<Sprite>)>();
    assert_eq!(
        query.iter(app.world()).count(),
        1,
        "a shell has to be drawn on the frame it appears, not the one after"
    );

    // Trigger held for five more seconds. A shell lives 0.9 s and one is fired
    // every 0.45 s, so a couple in the air at once is the ceiling — more than
    // that means dead shells and their sprites are piling up.
    step(&mut app, 300);
    let in_the_air = count::<Shell>(&mut app);
    assert!(in_the_air <= 3, "{in_the_air} shells still in the air");
}

#[test]
fn the_cleared_sector_overlay_comes_and_goes_over_a_field_that_stays_drawn() {
    // `tests/smoke.rs` proves the win condition actually fires; what is being
    // checked here is what happens on the way into and out of the state, so the
    // state is entered directly rather than by fighting through a whole level.
    let mut app = app();
    tap(&mut app, KeyCode::Enter);
    let walls = count::<MazeWall>(&mut app);

    app.world_mut()
        .resource_mut::<NextState<AppState>>()
        .set(AppState::LevelComplete);
    step(&mut app, 2);
    assert_eq!(state(&app), AppState::LevelComplete);
    assert_eq!(count::<LevelCompleteUi>(&mut app), 1);
    assert_eq!(
        count::<MazeWall>(&mut app),
        walls,
        "the sector is still there to look at"
    );

    tap(&mut app, KeyCode::Enter);
    assert_eq!(state(&app), AppState::MainMenu);
    assert_eq!(count::<LevelCompleteUi>(&mut app), 0);
    assert_eq!(count::<MainMenuUi>(&mut app), 1);
}

#[test]
fn a_full_run_survives_being_stepped_with_every_system_live() {
    // The catch-all: every schedule in the game actually runs, so every system's
    // world access and ordering gets validated. Everything is held down at once so
    // that driving, aiming and firing are all live while it does.
    let mut app = app();
    tap(&mut app, KeyCode::Enter);

    for held in [
        KeyCode::KeyD,
        KeyCode::KeyW,
        KeyCode::ArrowUp,
        KeyCode::ArrowRight,
        KeyCode::Space,
    ] {
        press(&mut app, held);
    }
    step(&mut app, 300);
    assert_eq!(state(&app), AppState::Playing);
}

/// The body colour of whichever factory the query happens to yield first.
///
/// Which one it is does not matter: the test only compares it against itself
/// before and after, and [`damage_a_factory`] picks the same one.
fn factory_colour(app: &mut App) -> Color {
    let mut query = app.world_mut().query_filtered::<&Sprite, With<Factory>>();
    query.iter(app.world()).next().expect("a factory").color
}

/// Knocks health off the first factory, standing in for a shell arriving.
fn damage_a_factory(app: &mut App, amount: i32) {
    let mut query = app
        .world_mut()
        .query_filtered::<&mut Health, With<Factory>>();
    let mut health = query
        .iter_mut(app.world_mut())
        .next()
        .expect("a factory to shoot");
    health.damage(amount);
}

fn aim_turret(app: &mut App, angle: f32) {
    let mut query = app.world_mut().query_filtered::<&mut Turret, With<Tank>>();
    let mut turret = query.single_mut(app.world_mut()).expect("one tank");
    turret.0 = angle;
}

/// Rough brightness of a colour, for "did this get darker" without caring how.
fn luminance(colour: Color) -> f32 {
    let rgb = colour.to_linear();
    rgb.red + rgb.green + rgb.blue
}
