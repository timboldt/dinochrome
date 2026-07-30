//! Full-screen text overlays: the main menu and the pause scrim.

use bevy::prelude::*;

use crate::palette;

/// Marks entities belonging to the main menu overlay.
#[derive(Component)]
pub struct MainMenuUi;

/// Marks entities belonging to the pause overlay.
#[derive(Component)]
pub struct PauseUi;

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
            line("WASD to drive · ESC to pause", palette::PHOSPHOR_DIM),
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
