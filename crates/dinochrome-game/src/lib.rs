//! Bevy front end for dinochrome.
//!
//! The plugins are split so that the parts which decide *what happens* can run
//! without a renderer:
//!
//! - [`SimPlugin`] — state machine, maze, entities, input, fixed-timestep
//!   movement. Runs headlessly on `MinimalPlugins`; this is what the smoke test
//!   drives.
//! - [`PresentationPlugin`] — camera, sprites, UI overlays. Needs a renderer.
//! - [`DinochromePlugin`] — both of the above; what `main` adds.

pub mod camera;
pub mod factory;
pub mod maze;
pub mod menu;
pub mod palette;
pub mod player;
pub mod state;
pub mod turret;
pub mod weapon;

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
            .init_resource::<maze::MazeConfig>()
            // The maze, the tank and the factories outlive the pause state, so
            // their lifetime is tied to leaving and re-entering the menu rather
            // than to `Playing`. M4 hands this over to level progression.
            //
            // Chained because both the tank and the factories are placed on cells
            // the maze picked: neither can be created before the maze they stand
            // in.
            .add_systems(
                OnExit(AppState::MainMenu),
                (maze::generate, player::spawn_tank, factory::spawn_factories).chain(),
            )
            .add_systems(
                OnEnter(AppState::MainMenu),
                (
                    player::despawn_tank,
                    despawn_all::<factory::Factory>,
                    despawn_all::<weapon::Shell>,
                ),
            )
            // Leaving `Playing` for any reason — paused, cleared, abandoned — has
            // to drop the held controls. Left set, they would be acted on the
            // instant play resumed, so a tank paused mid-throttle would lurch and
            // a held trigger would fire a shell into the pause screen.
            .add_systems(
                OnExit(AppState::Playing),
                (
                    player::clear_drive_input,
                    turret::clear_aim_input,
                    weapon::clear_fire_input,
                ),
            )
            .add_systems(
                Update,
                (
                    state::start_game.run_if(in_state(AppState::MainMenu)),
                    state::toggle_pause
                        .run_if(in_state(AppState::Playing).or_else(in_state(AppState::Paused))),
                    state::quit_to_menu.run_if(in_state(AppState::Paused)),
                    state::leave_level_complete.run_if(in_state(AppState::LevelComplete)),
                ),
            )
            // `RunFixedMainLoop` runs *before* `Update`, so sampling input in
            // `Update` would feed the simulation input that is already a frame
            // stale. `BeforeFixedMainLoop` is the set Bevy provides for exactly
            // this: read the keyboard, then immediately tick the simulation with
            // what it said.
            .add_systems(
                RunFixedMainLoop,
                (
                    player::sample_drive_input,
                    turret::sample_aim_input,
                    weapon::sample_fire_input,
                )
                    .in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop)
                    .run_if(in_state(AppState::Playing)),
            )
            // One tick, in a fixed order, so the simulation is reproducible rather
            // than merely correct. Shells move before the gun fires, so a shell
            // spends its first tick sitting at the muzzle where the player can see
            // it leave; and the dead are reaped after the shells have landed, so a
            // factory destroyed this tick is gone this tick.
            .add_systems(
                FixedUpdate,
                (
                    player::move_tanks,
                    turret::slew_turrets,
                    weapon::move_shells,
                    weapon::fire_weapons,
                    factory::destroy_dead_factories,
                )
                    .chain()
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

/// Everything that decides what the game looks like.
pub struct PresentationPlugin;

impl Plugin for PresentationPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(palette::VOID))
            .add_systems(Startup, camera::spawn)
            .add_systems(OnEnter(AppState::MainMenu), menu::spawn_main_menu)
            .add_systems(
                OnExit(AppState::MainMenu),
                (
                    despawn_all::<menu::MainMenuUi>,
                    // Both of these read what the simulation just created, so
                    // both have to be ordered against it explicitly — a
                    // different plugin, but the same schedule.
                    maze::render_walls.after(maze::generate),
                    camera::snap_to_tank.after(player::spawn_tank),
                ),
            )
            .add_systems(OnEnter(AppState::MainMenu), despawn_all::<maze::MazeWall>)
            .add_systems(OnEnter(AppState::Paused), menu::spawn_pause_overlay)
            .add_systems(OnExit(AppState::Paused), despawn_all::<menu::PauseUi>)
            .add_systems(
                OnEnter(AppState::LevelComplete),
                menu::spawn_level_complete_overlay,
            )
            .add_systems(
                OnExit(AppState::LevelComplete),
                despawn_all::<menu::LevelCompleteUi>,
            )
            .add_systems(
                Update,
                (
                    player::attach_tank_sprite,
                    weapon::attach_shell_sprites,
                    factory::attach_factory_sprites,
                    factory::show_factory_damage,
                    turret::sync_barrels,
                    // Left running while paused so that a glide already in
                    // flight settles instead of freezing half-way.
                    camera::follow_tank
                        .run_if(in_state(AppState::Playing).or_else(in_state(AppState::Paused))),
                ),
            );
    }
}

/// The whole game.
pub struct DinochromePlugin;

impl Plugin for DinochromePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((SimPlugin, PresentationPlugin));
    }
}

/// Despawns every entity carrying the marker `T`, along with its children.
pub fn despawn_all<T: Component>(mut commands: Commands, entities: Query<Entity, With<T>>) {
    for entity in &entities {
        commands.entity(entity).despawn();
    }
}
