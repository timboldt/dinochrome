//! Engine-free simulation logic for dinochrome.
//!
//! Nothing in this crate may depend on Bevy. Every rule that decides what
//! happens in the game — maze generation, movement, collision, line of sight,
//! pathfinding — lives here so it can be unit-tested headlessly, and so the
//! Bevy layer is reduced to reading input, calling into this crate, and drawing
//! the result.
//!
//! `glam` is pinned to the same major version Bevy uses internally, so [`Vec2`]
//! crosses the boundary without conversion.
//!
//! [`Vec2`]: glam::Vec2

pub mod collision;
pub mod grid;
pub mod hull;
pub mod maze;

pub use grid::{CELL_SIZE, Cell, Grid};
pub use maze::{Maze, MazeParams};

/// Simulation tick rate, in hertz.
///
/// Gameplay must be identical at any render frame rate, so every simulation
/// system runs on a fixed timestep and none of them may read a variable frame
/// delta. See [`FIXED_DT`] for the matching per-tick duration.
pub const FIXED_HZ: f64 = 60.0;

/// Duration of one simulation tick, in seconds.
pub const FIXED_DT: f32 = 1.0 / FIXED_HZ as f32;
