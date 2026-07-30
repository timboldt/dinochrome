//! The level's maze: generating it, and turning it into wall sprites.
//!
//! Generation is a simulation concern — collision, and later line of sight and
//! pathfinding, all read the grid — so it lives in [`SimPlugin`] and runs
//! headlessly. Only [`render_walls`] needs a renderer.
//!
//! [`SimPlugin`]: crate::SimPlugin

use bevy::prelude::*;
use dinochrome_core::grid::CELL_SIZE;
use dinochrome_core::maze::{self, MazeParams};

use crate::palette;

/// Draw order for wall sprites: behind everything that moves.
const Z_WALL: f32 = -1.0;

/// The maze the current run is being played in.
///
/// Created on leaving the menu, before the tank is spawned, so anything that
/// runs while [`AppState::Playing`] can count on it existing.
///
/// [`AppState::Playing`]: crate::AppState::Playing
#[derive(Resource, Debug, Deref)]
pub struct Maze(pub maze::Maze);

/// What to generate for the next level.
///
/// M4 turns this into the level-progression knob — bigger and denser each level.
/// For now it is here so that tests can ask for a maze they can predict.
#[derive(Resource, Debug, Clone, Default)]
pub struct MazeConfig {
    /// Size and wall density.
    pub params: MazeParams,
    /// A fixed seed, or `None` to take one from the clock at level start.
    pub seed: Option<u64>,
}

/// Marks a wall sprite, so the whole drawn maze can be cleared in one pass.
#[derive(Component)]
pub struct MazeWall;

/// Generates the maze for a new run.
pub fn generate(mut commands: Commands, config: Res<MazeConfig>, real: Res<Time<Real>>) {
    // Native and wasm share no entropy source that does not drag in a
    // JS-backed `getrandom` — but they do share a clock, and how long the player
    // left the menu up before pressing start is an unpredictable nanosecond
    // count. Good enough to pick a maze; nothing here is security-sensitive.
    let seed = config
        .seed
        .unwrap_or_else(|| real.elapsed().as_nanos() as u64);

    let maze = maze::generate(config.params, seed);
    // Printed so that a maze someone got stuck in can be regenerated exactly
    // from a bug report.
    info!(
        "maze {}x{}, density {:.3}, seed {}",
        maze.grid.width(),
        maze.grid.height(),
        maze.wall_density(),
        seed,
    );
    commands.insert_resource(Maze(maze));
}

/// Draws one sprite per wall cell.
///
/// A level-one maze is a few hundred sprites and the largest planned maze is
/// under two thousand, which Bevy's 2D batcher handles without noticing. Merging
/// runs of wall into single stretched quads would cut that further, but there is
/// no measured reason to.
pub fn render_walls(mut commands: Commands, maze: Res<Maze>) {
    let block = Sprite::from_color(palette::WALL, Vec2::splat(CELL_SIZE));
    for cell in maze.grid.walls() {
        let at = maze.grid.cell_center(cell);
        commands.spawn((
            MazeWall,
            block.clone(),
            Transform::from_xyz(at.x, at.y, Z_WALL),
        ));
    }
}
