//! A phosphor palette — green and amber on near-black — as a nod to the Apple II
//! the original ran on.

use bevy::prelude::Color;

/// Window/background clear colour.
pub const VOID: Color = Color::srgb(0.03, 0.04, 0.03);
/// Primary phosphor green, used for the tank and body text.
pub const PHOSPHOR: Color = Color::srgb(0.35, 0.95, 0.45);
/// Amber, used for headings and anything that wants attention.
pub const AMBER: Color = Color::srgb(1.0, 0.72, 0.20);
/// Dimmed green for secondary text.
pub const PHOSPHOR_DIM: Color = Color::srgb(0.20, 0.55, 0.28);
/// Wash drawn over the play field while paused.
pub const SCRIM: Color = Color::srgba(0.03, 0.04, 0.03, 0.78);

/// Maze walls. Darker than [`PHOSPHOR_DIM`] on purpose: walls cover something
/// like half the screen, and at text brightness that is all you would see.
pub const WALL: Color = Color::srgb(0.09, 0.24, 0.13);

/// Anything belonging to the enemy. The one thing on screen that is not the
/// green/amber phosphor, because "not yours" should need no second look.
pub const HOSTILE: Color = Color::srgb(0.93, 0.31, 0.24);

/// The player's tank.
pub const TANK: Color = PHOSPHOR;

/// The tank's gun barrel. Brighter than the hull, because where you are pointing
/// is the thing you need to read at a glance.
pub const TURRET: Color = Color::srgb(0.72, 1.0, 0.78);

/// A shell in flight.
pub const SHELL: Color = AMBER;

/// A drone factory's shell.
pub const FACTORY: Color = HOSTILE;

/// A drone factory's core — what you are actually shooting at.
pub const FACTORY_CORE: Color = AMBER;

/// A factory at zero health, mixed toward as it takes damage.
///
/// Not [`VOID`]: a factory that faded into the background exactly as it became
/// worth one more shell would be the wrong thing to hide.
pub const FACTORY_WRECKED: Color = Color::srgb(0.28, 0.12, 0.10);
