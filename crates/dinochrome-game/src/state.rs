//! Top-level app state machine.

use bevy::prelude::*;

/// The screens the app can be in.
///
/// `LevelComplete` and `GameOver` from the design sketch are deliberately absent
/// until the milestones that can actually reach them (M2 and M3); an unreachable
/// state is just a lie in the type system.
#[derive(States, Debug, Clone, Copy, Default, Eq, PartialEq, Hash)]
pub enum AppState {
    #[default]
    MainMenu,
    Playing,
    Paused,
}

/// Leaves the menu once the player asks to start.
///
/// This is also the "user gesture" the browser requires before an `AudioContext`
/// may start, so from M5 onward audio initialization hangs off this transition
/// rather than off app startup.
pub fn start_game(keys: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<AppState>>) {
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
        next.set(AppState::Playing);
    }
}

/// Toggles between playing and paused on Escape.
pub fn toggle_pause(
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<State<AppState>>,
    mut next: ResMut<NextState<AppState>>,
) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    match state.get() {
        AppState::Playing => next.set(AppState::Paused),
        AppState::Paused => next.set(AppState::Playing),
        AppState::MainMenu => {}
    }
}

/// Abandons the current run and returns to the menu.
pub fn quit_to_menu(keys: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<AppState>>) {
    if keys.just_pressed(KeyCode::KeyQ) {
        next.set(AppState::MainMenu);
    }
}
