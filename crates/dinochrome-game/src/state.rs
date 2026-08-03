//! Top-level app state machine.

use bevy::prelude::*;

/// The screens the app can be in.
#[derive(States, Debug, Clone, Copy, Default, Eq, PartialEq, Hash)]
pub enum AppState {
    #[default]
    MainMenu,
    Playing,
    Paused,
    /// Every factory in the level is destroyed.
    LevelComplete,
    /// The tank is destroyed.
    GameOver,
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
        AppState::MainMenu | AppState::LevelComplete | AppState::GameOver => {}
    }
}

/// Abandons the current run and returns to the menu.
pub fn quit_to_menu(keys: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<AppState>>) {
    if keys.just_pressed(KeyCode::KeyQ) {
        next.set(AppState::MainMenu);
    }
}

/// Leaves the level-cleared screen.
///
/// Back to the menu for now. M4 turns this into progression — the next level, one
/// harder — which is why it is a transition of its own rather than a second use of
/// [`quit_to_menu`].
pub fn leave_level_complete(
    keys: Res<ButtonInput<KeyCode>>,
    mut next: ResMut<NextState<AppState>>,
) {
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
        next.set(AppState::MainMenu);
    }
}

/// Leaves the game-over screen.
///
/// Its own transition rather than a third use of [`quit_to_menu`], for the same
/// reason [`leave_level_complete`] is: what follows a loss and what follows a win
/// stop being the same thing the moment M4 adds progression to one of them.
pub fn leave_game_over(keys: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<AppState>>) {
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space) {
        next.set(AppState::MainMenu);
    }
}
