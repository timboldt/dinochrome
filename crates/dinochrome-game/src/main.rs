//! `dinochrome` binary entry point.

use bevy::prelude::*;
use dinochrome_game::DinochromePlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "dinochrome".into(),
                resolution: (1280, 720).into(),
                // On wasm, bind to the canvas `index.html` provides and let
                // it track its wrapper's size. The wrapper is a fixed
                // viewport-sized box, so there is no resize feedback loop.
                canvas: Some("#dinochrome-canvas".into()),
                fit_canvas_to_parent: true,
                // Keep WASD, the arrow keys and space from scrolling the page
                // out from under the game.
                prevent_default_event_handling: true,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(DinochromePlugin)
        .run();
}
