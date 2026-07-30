//! The chase camera.
//!
//! The camera trails the tank rather than being welded to it, which reads as
//! momentum, and it is clamped so the viewport never slides off the maze into the
//! void. Both are presentation: nothing here may affect the simulation, so this
//! is the one place in the game that is allowed to read a variable frame delta.

use bevy::prelude::*;

use crate::maze::Maze;
use crate::player::Tank;

/// How fast the camera closes the gap to the tank, in "e-folds" per second.
///
/// Higher is snappier. Around 12 the camera is a step behind during a turn and
/// caught up within a fifth of a second, which is enough lag to feel like weight
/// and not enough to lose track of what is ahead.
const FOLLOW_RATE: f32 = 12.0;

/// The camera's transform, plus the framing information on the camera itself.
///
/// Aliased because it appears in three signatures. The `Without<Tank>` is what
/// proves to Bevy that this `&mut Transform` and the tank's `&Transform` can
/// never be the same entity; without it, the two queries conflict.
type CameraView<'w, 's> =
    Query<'w, 's, (&'static mut Transform, &'static Camera), (With<Camera2d>, Without<Tank>)>;

/// Creates the one camera.
pub fn spawn(mut commands: Commands) {
    commands.spawn(Camera2d);
}

/// Puts the camera on the tank immediately, with no easing.
///
/// Runs when a level starts. Without it the first second of every run would be a
/// long glide in from wherever the camera happened to be.
pub fn snap_to_tank(maze: Res<Maze>, tank: Query<&Transform, With<Tank>>, camera: CameraView) {
    aim(&maze, tank, camera, 1.0);
}

/// Eases the camera toward the tank.
pub fn follow_tank(
    time: Res<Time>,
    maze: Res<Maze>,
    tank: Query<&Transform, With<Tank>>,
    camera: CameraView,
) {
    // Closing a fixed *fraction* per frame would make the camera's stiffness
    // depend on the frame rate. Closing a fixed fraction per second does not.
    let fraction = 1.0 - (-FOLLOW_RATE * time.delta_secs()).exp();
    aim(&maze, tank, camera, fraction);
}

/// Moves the camera `fraction` of the way from where it is to where it belongs.
///
/// Every early return here is a legitimate state, not a failure: the headless
/// tests have no camera at all, and a camera has no viewport until the render
/// world has sized it.
fn aim(maze: &Maze, tank: Query<&Transform, With<Tank>>, mut camera: CameraView, fraction: f32) {
    let Ok(tank) = tank.single() else {
        return;
    };
    let Ok((mut transform, camera)) = camera.single_mut() else {
        return;
    };
    // No viewport yet means the render world has not sized this camera. Framing
    // it against a zero viewport would centre on the tank exactly, which is
    // wrong for one frame and fixes itself on the next.
    let Some(viewport) = camera.logical_viewport_size() else {
        return;
    };

    let target = frame(
        tank.translation.truncate(),
        viewport,
        maze.grid.world_size(),
    );
    let next = transform.translation.truncate().lerp(target, fraction);
    transform.translation.x = next.x;
    transform.translation.y = next.y;
}

/// Where the camera should be centred to keep `subject` in view without showing
/// anything outside the maze.
///
/// The maze occupies `Vec2::ZERO` to `world`. If it is smaller than the viewport
/// on an axis there is no way to fill the screen with it, so it is centred on
/// that axis instead — the void shows, but symmetrically, rather than the maze
/// being jammed against one edge.
fn frame(subject: Vec2, viewport: Vec2, world: Vec2) -> Vec2 {
    let margin = viewport * 0.5;
    let low = margin;
    let high = world - margin;
    Vec2::new(
        if low.x <= high.x {
            subject.x.clamp(low.x, high.x)
        } else {
            world.x * 0.5
        },
        if low.y <= high.y {
            subject.y.clamp(low.y, high.y)
        } else {
            world.y * 0.5
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A level-one maze against a 720p window.
    const WORLD: Vec2 = Vec2::new(2112.0, 1600.0);
    const VIEW: Vec2 = Vec2::new(1280.0, 720.0);

    #[test]
    fn the_camera_tracks_the_subject_in_the_middle_of_the_maze() {
        let subject = Vec2::new(1000.0, 800.0);
        assert_eq!(frame(subject, VIEW, WORLD), subject);
    }

    #[test]
    fn the_viewport_never_leaves_the_maze() {
        for subject in [
            Vec2::ZERO,
            WORLD,
            Vec2::new(-5000.0, 9000.0),
            Vec2::new(50.0, 1550.0),
        ] {
            let at = frame(subject, VIEW, WORLD);
            let low = at - VIEW * 0.5;
            let high = at + VIEW * 0.5;
            assert!(
                low.x >= -0.01 && low.y >= -0.01 && high.x <= WORLD.x + 0.01 && high.y <= WORLD.y,
                "subject {subject:?} framed at {at:?} shows the void"
            );
        }
    }

    #[test]
    fn a_maze_narrower_than_the_viewport_is_centred_on_that_axis() {
        // Half as wide as the window, but twice as tall.
        let world = Vec2::new(640.0, 1600.0);
        let at = frame(Vec2::new(0.0, 800.0), VIEW, world);
        assert_eq!(at.x, 320.0, "the narrow axis should be centred");
        assert_eq!(at.y, 800.0, "the tall axis should still track");
    }

    #[test]
    fn a_maze_exactly_the_size_of_the_viewport_is_pinned() {
        let at = frame(Vec2::new(9999.0, -9999.0), VIEW, VIEW);
        assert_eq!(at, VIEW * 0.5);
    }
}
