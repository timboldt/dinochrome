//! Full-screen text overlays: the main menu and the pause scrim.

use bevy::prelude::*;

use crate::palette;

/// Marks entities belonging to the main menu overlay.
#[derive(Component)]
pub struct MainMenuUi;

/// Marks entities belonging to the pause overlay.
#[derive(Component)]
pub struct PauseUi;

/// Marks entities belonging to the level-cleared overlay.
#[derive(Component)]
pub struct LevelCompleteUi;

/// Marks entities belonging to the game-over overlay.
#[derive(Component)]
pub struct GameOverUi;

/// A full-screen, centred, vertically stacked overlay root.
fn overlay_root(background: Color) -> impl Bundle {
    (
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            row_gap: px(16),
            ..default()
        },
        BackgroundColor(background),
    )
}

fn heading(text: &str, color: Color) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font_size: FontSize::Px(64.0),
            ..default()
        },
        TextColor(color),
    )
}

fn line(text: &str, color: Color) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font_size: FontSize::Px(22.0),
            ..default()
        },
        TextColor(color),
    )
}

pub fn spawn_main_menu(mut commands: Commands) {
    commands.spawn((
        MainMenuUi,
        overlay_root(palette::VOID),
        children![
            heading("DINOCHROME", palette::AMBER),
            line("Dinochrome Brigade — armour command", palette::PHOSPHOR_DIM),
            line("Press ENTER or SPACE to deploy", palette::PHOSPHOR),
            line(
                "Destroy every drone factory in the sector",
                palette::PHOSPHOR
            ),
            line(
                "WASD to drive · ARROWS to aim · SPACE to fire · ESC to pause",
                palette::PHOSPHOR_DIM,
            ),
        ],
    ));
}

pub fn spawn_pause_overlay(mut commands: Commands) {
    commands.spawn((
        PauseUi,
        overlay_root(palette::SCRIM),
        children![
            heading("HOLDING", palette::AMBER),
            line("ESC to resume · Q to abandon the sortie", palette::PHOSPHOR),
        ],
    ));
}

/// The dispatch that comes in when the last factory goes down.
///
/// Terse on purpose, and M4 makes it a screen of its own with the sector's numbers
/// on it. For now it is the acknowledgement that the win condition fired.
pub fn spawn_level_complete_overlay(mut commands: Commands) {
    commands.spawn((
        LevelCompleteUi,
        overlay_root(palette::SCRIM),
        children![
            heading("SECTOR CLEAR", palette::AMBER),
            line(
                "Brigade to unit: last factory confirmed destroyed. Well done.",
                palette::PHOSPHOR,
            ),
            line("ENTER or SPACE to stand down", palette::PHOSPHOR_DIM),
        ],
    ));
}

/// The dispatch that comes in when the tank does not.
///
/// Amber rather than the hostile red, because this is the Brigade talking, not the
/// thing that killed you.
pub fn spawn_game_over_overlay(mut commands: Commands) {
    commands.spawn((
        GameOverUi,
        overlay_root(palette::SCRIM),
        children![
            heading("UNIT LOST", palette::AMBER),
            line(
                "Brigade to sector: contact lost. The factories are still standing.",
                palette::PHOSPHOR,
            ),
            line("ENTER or SPACE to return to command", palette::PHOSPHOR_DIM),
        ],
    ));
}
