//! Bevy front end for dinochrome.
//!
//! The plugins are split so that the parts which decide *what happens* can run
//! without a renderer:
//!
//! - [`SimPlugin`] — state machine, entities, input, fixed-timestep movement.
//!   Runs headlessly on `MinimalPlugins`; this is what the smoke test drives.
//! - [`PresentationPlugin`] — camera, sprites, UI overlays. Needs a renderer.
//! - [`DinochromePlugin`] — both of the above; what `main` adds.

pub mod menu;
pub mod palette;
pub mod player;
pub mod state;

use bevy::app::{RunFixedMainLoop, RunFixedMainLoopSystems};
use bevy::prelude::*;
use dinochrome_core::FIXED_HZ;

pub use state::AppState;

/// Everything that decides what happens in the game.
pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(FIXED_HZ))
            .init_state::<AppState>()
            // The tank outlives the pause state, so its lifetime is tied to
            // leaving and re-entering the menu rather than to `Playing`. M4
            // hands this over to level progression.
            .add_systems(OnExit(AppState::MainMenu), player::spawn_tank)
            .add_systems(OnEnter(AppState::MainMenu), player::despawn_tank)
            .add_systems(OnEnter(AppState::Paused), player::clear_drive_input)
            .add_systems(
                Update,
                (
                    state::start_game.run_if(in_state(AppState::MainMenu)),
                    state::toggle_pause
                        .run_if(in_state(AppState::Playing).or_else(in_state(AppState::Paused))),
                    state::quit_to_menu.run_if(in_state(AppState::Paused)),
                ),
            )
            // `RunFixedMainLoop` runs *before* `Update`, so sampling input in
            // `Update` would feed the simulation input that is already a frame
            // stale. `BeforeFixedMainLoop` is the set Bevy provides for exactly
            // this: read the keyboard, then immediately tick the simulation with
            // what it said.
            .add_systems(
                RunFixedMainLoop,
                player::sample_drive_input
                    .in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop)
                    .run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                FixedUpdate,
                player::move_tanks.run_if(in_state(AppState::Playing)),
            );
    }
}

/// Everything that decides what the game looks like.
pub struct PresentationPlugin;

impl Plugin for PresentationPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(palette::VOID))
            .add_systems(Startup, spawn_camera)
            .add_systems(OnEnter(AppState::MainMenu), menu::spawn_main_menu)
            .add_systems(
                OnExit(AppState::MainMenu),
                menu::despawn_all::<menu::MainMenuUi>,
            )
            .add_systems(OnEnter(AppState::Paused), menu::spawn_pause_overlay)
            .add_systems(OnExit(AppState::Paused), menu::despawn_all::<menu::PauseUi>)
            .add_systems(Update, player::attach_tank_sprite);
    }
}

/// The whole game.
pub struct DinochromePlugin;

impl Plugin for DinochromePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((SimPlugin, PresentationPlugin));
    }
}

/// M1 attaches this camera to the tank; for now it sits at the origin.
fn spawn_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}
