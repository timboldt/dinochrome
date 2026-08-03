//! The four kinds of drone: what they are made of, and how each one thinks.
//!
//! This is the difficulty curve made explicit. A factory on level one builds
//! nothing but the dumbest kind; each level unlocks the next one up and shifts the
//! mix toward it, so the maze does not merely get bigger as the game goes on — it
//! gets meaner in a way the player can name after the first time one of them
//! comes round a corner already pointing the right way.
//!
//! # What is here and what is not
//!
//! A drone's decisions are two questions, and both are answered here as pure
//! functions of the grid: *which cell do I head for next* ([`Pursuit`]) and *may I
//! fire* ([`Trigger`]). Turning a chosen cell into a heading, and a heading into a
//! position, is the same [`crate::hull`] and [`crate::collision`] code the player's
//! tank goes through — a drone is not a special case of movement, it just has
//! something other than a keyboard deciding where it wants to go.
//!
//! All four are slower than the player's tank. Being able to break contact and
//! come back on your own terms is most of what makes the maze a place to fight in
//! rather than a place to be cornered in, so nothing chases you down by simply
//! being quicker; an assassin gets you because it knows the way, not because it
//! outruns you.

use glam::IVec2;

use crate::grid::Grid;
use crate::hull::HullParams;
use crate::weapon::WeaponParams;

/// How many drone kinds there are. The spawn table is indexed by this.
pub const KIND_COUNT: usize = 4;

/// The kinds of drone a factory can build, dumbest first.
///
/// Ordering is the difficulty ordering, and [`DroneKind::ALL`] is in it, so a
/// spawn table can be written as a plain array without a lookup.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DroneKind {
    /// Wanders at random and fires on a timer, at whatever happens to be in front
    /// of it. Dangerous in the way a loose wire is dangerous.
    Drone,
    /// Wanders too, but quicker, and holds its fire until it actually has a shot.
    Torpedo,
    /// Drives straight at you, taking whichever opening leads that way. Loses you
    /// to any wall it cannot see round, which is the whole of its weakness.
    Hunter,
    /// Pathfinds. There is no corner it does not know the way around; the only
    /// thing that beats it is killing it.
    Assassin,
}

impl DroneKind {
    /// Every kind, in difficulty order.
    pub const ALL: [Self; KIND_COUNT] = [Self::Drone, Self::Torpedo, Self::Hunter, Self::Assassin];

    /// The level at which factories start building this kind.
    pub const fn unlock_level(self) -> u32 {
        match self {
            Self::Drone => 1,
            Self::Torpedo => 2,
            Self::Hunter => 3,
            Self::Assassin => 4,
        }
    }

    /// Everything that makes this kind what it is.
    pub const fn params(self) -> DroneParams {
        match self {
            Self::Drone => DroneParams {
                hull: HullParams {
                    max_speed: 70.0,
                    accel: 240.0,
                    brake: 320.0,
                },
                weapon: WeaponParams {
                    shell_speed: 380.0,
                    cooldown: 2.2,
                    range: 320.0,
                    damage: 8,
                },
                health: 20,
                sight: 0.0,
                repath: 0.0,
                pursuit: Pursuit::Wander,
                trigger: Trigger::Blind,
            },
            Self::Torpedo => DroneParams {
                hull: HullParams {
                    max_speed: 110.0,
                    accel: 380.0,
                    brake: 480.0,
                },
                weapon: WeaponParams {
                    shell_speed: 520.0,
                    cooldown: 1.4,
                    range: 448.0,
                    damage: 10,
                },
                health: 20,
                // Exactly its shell's range: it never takes a shot that gives out
                // in mid-air, and never declines one it could have landed.
                sight: 448.0,
                repath: 0.0,
                pursuit: Pursuit::Wander,
                trigger: Trigger::OnSight,
            },
            Self::Hunter => DroneParams {
                hull: HullParams {
                    max_speed: 130.0,
                    accel: 440.0,
                    brake: 560.0,
                },
                weapon: WeaponParams {
                    shell_speed: 440.0,
                    cooldown: 1.8,
                    range: 320.0,
                    damage: 10,
                },
                health: 40,
                sight: 320.0,
                repath: 0.0,
                pursuit: Pursuit::Greedy,
                trigger: Trigger::OnSight,
            },
            Self::Assassin => DroneParams {
                hull: HullParams {
                    max_speed: 155.0,
                    accel: 520.0,
                    brake: 640.0,
                },
                weapon: WeaponParams {
                    shell_speed: 520.0,
                    cooldown: 1.2,
                    range: 384.0,
                    damage: 12,
                },
                health: 40,
                sight: 384.0,
                // Twice a second. Fast enough that dodging round one cell does not
                // shake it; slow enough that a dozen of them are a rounding error
                // in the frame budget. Callers stagger the phase — see
                // [`repath_phase`].
                repath: 0.5,
                pursuit: Pursuit::Path,
                trigger: Trigger::OnSight,
            },
        }
    }
}

/// How a drone decides which cell to head for next.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pursuit {
    /// Any open neighbour, preferring not to double back.
    Wander,
    /// Whichever open neighbour gets nearer the player, falling back to wandering
    /// where none of them does.
    Greedy,
    /// The first step of an A* route to the player.
    Path,
}

/// When a drone is allowed to pull the trigger.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Trigger {
    /// Whenever the gun is loaded, wherever it happens to be pointing.
    Blind,
    /// Only with the player in range and in plain sight.
    OnSight,
}

/// Everything that distinguishes one kind of drone from another.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DroneParams {
    /// How it moves.
    pub hull: HullParams,
    /// What it fires.
    pub weapon: WeaponParams,
    /// Hit points. The tank's shell does twenty, so this is a shell count.
    pub health: i32,
    /// How far it can see to shoot, in world units. Zero for a kind that does not
    /// look before firing.
    pub sight: f32,
    /// Seconds between route recomputes. Zero for a kind that does not path.
    pub repath: f32,
    /// How it chooses where to go.
    pub pursuit: Pursuit,
    /// How it chooses when to shoot.
    pub trigger: Trigger,
}

/// Relative likelihood of each kind at `level`, in [`DroneKind::ALL`] order.
///
/// A kind is unavailable until its level and then ramps in over the next few, to a
/// ceiling every kind eventually reaches. Two things fall out of that ceiling and
/// both are deliberate: the mix converges on uniform rather than on all-assassins,
/// so a late level is *varied* rather than uniformly lethal; and the cheap drones
/// never disappear, so there is always something on the screen to shoot at between
/// the things that are actually trying to kill you.
pub fn spawn_weights(level: u32) -> [f32; KIND_COUNT] {
    /// Weight a kind reaches three levels after it is unlocked, and never passes.
    const CEILING: u32 = 3;

    DroneKind::ALL.map(|kind| {
        let unlock = kind.unlock_level();
        if level < unlock {
            0.0
        } else {
            (level - unlock + 1).min(CEILING) as f32
        }
    })
}

/// Picks a kind for `level` from a roll in `0.0..1.0`.
///
/// Out-of-range rolls are clamped rather than rejected: this is called with the
/// output of [`crate::rng::SimRng::unit`], and a roll that lands exactly on the
/// end of the range should build the last kind rather than panic.
pub fn kind_at(level: u32, roll: f32) -> DroneKind {
    let weights = spawn_weights(level);
    let total: f32 = weights.iter().sum();
    // Below the first unlock there is nothing to build but the first kind. That is
    // level zero, which no level ever is, but the alternative is an `Option` every
    // caller has to unwrap for a case that cannot happen.
    if total <= 0.0 {
        return DroneKind::Drone;
    }

    let mut ticket = roll.clamp(0.0, 1.0) * total;
    for (index, weight) in weights.iter().enumerate() {
        // Tested before the subtraction, and against the weight rather than
        // against zero, so a kind with no weight can never win. Subtracting first
        // and asking afterwards lets a roll of exactly 1.0 fall past every kind
        // with a share of the table and land on one with none of it.
        if ticket < *weight {
            return DroneKind::ALL[index];
        }
        ticket -= weight;
    }
    // Only reachable when the roll lands exactly on the total, which the clamp
    // above allows: that is the far end of the last kind that has a share.
    let last = weights
        .iter()
        .rposition(|weight| *weight > 0.0)
        .expect("a positive total has a positive weight in it");
    DroneKind::ALL[last]
}

/// Spreads a fleet's route recomputes across the interval instead of bunching them.
///
/// Twenty assassins that all repath on the same tick cost twenty searches in one
/// frame and nothing in the next twenty-nine. Offsetting each one by a slice of
/// the interval turns the same total work into an even trickle, which is the
/// difference between a hitch the player can feel and one nobody ever measures.
///
/// `serial` is any per-drone number that differs between drones; the spawn order
/// is what callers have.
pub fn repath_phase(interval: f32, serial: u32) -> f32 {
    /// Number of slices the interval is cut into. Coprime with nothing in
    /// particular — it only has to be more than the drones alive at once, which a
    /// factory cap keeps well under this.
    const SLICES: u32 = 64;

    interval * (serial % SLICES) as f32 / SLICES as f32
}

/// Picks the next cell for a wandering drone.
///
/// Doubling back is a last resort rather than a one-in-four chance, which is what
/// keeps a wanderer exploring the maze instead of jittering in a corridor. In a
/// dead end there is nothing else to do, so it is allowed there.
///
/// `None` only if the drone is somewhere with no open neighbour at all — walled
/// in, or standing outside the maze.
pub fn wander_step(grid: &Grid, at: IVec2, came_from: Option<IVec2>, roll: f32) -> Option<IVec2> {
    let onward = |exclude: Option<IVec2>| {
        let options: Vec<IVec2> = grid
            .open_neighbours(at)
            .filter(|next| Some(*next) != exclude)
            .collect();
        pick(&options, roll)
    };
    onward(came_from).or_else(|| onward(None))
}

/// Picks the neighbour that gets a hunter nearest `toward`.
///
/// Greed is the whole design: it takes the opening that points at you and has no
/// memory of anything it has tried. That makes it beatable by any wall it has to
/// go the wrong way round, which is what a hunter is *for* — the thing that does
/// not fall for it is the assassin, and the difference between the two should be
/// something the player can see happening.
///
/// Where no neighbour is an improvement — in the pocket of a dead end, say — it
/// wanders instead. Standing still would be worse than useless: a stuck hunter is
/// free score.
pub fn greedy_step(
    grid: &Grid,
    at: IVec2,
    toward: IVec2,
    came_from: Option<IVec2>,
    roll: f32,
) -> Option<IVec2> {
    let here = (at - toward).length_squared();
    grid.open_neighbours(at)
        .filter(|next| (*next - toward).length_squared() < here)
        // `min_by_key` keeps the first of equal keys, and `open_neighbours` is in
        // a fixed order, so a tie resolves the same way every run.
        .min_by_key(|next| (*next - toward).length_squared())
        .or_else(|| wander_step(grid, at, came_from, roll))
}

/// Picks one of `options` using a roll in `0.0..1.0`.
fn pick(options: &[IVec2], roll: f32) -> Option<IVec2> {
    if options.is_empty() {
        return None;
    }
    let index = (roll.clamp(0.0, 1.0) * options.len() as f32) as usize;
    options.get(index.min(options.len() - 1)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::ORTHOGONAL;
    use crate::hull::HullParams;
    use crate::maze::{self, MazeParams};
    use crate::rng::SimRng;

    /// Rolls spread across the unit interval, including both ends.
    const ROLLS: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 0.999];

    /// A blind vertical pocket, open only at the bottom, with open floor above it.
    ///
    /// The trap that tells a hunter from an assassin. From `(2, 1)` the target at
    /// `(2, 4)` is three cells straight up and nine steps away in reality: the
    /// pocket points right at it and does not lead there.
    fn pocket() -> Grid {
        Grid::from_rows(&[
            ".....", // y = 5
            ".....", // y = 4  ← the target's row
            ".###.", // y = 3  ← the lid of the pocket
            ".#.#.", // y = 2
            ".#.#.", // y = 1  ← where the hunter starts
            ".....", // y = 0  ← the pocket's only mouth
        ])
    }

    #[test]
    fn nothing_outruns_the_players_tank() {
        // The one balance invariant the whole design leans on: you can always
        // break contact. A drone faster than the tank would make retreating
        // impossible and the maze a trap rather than a battlefield.
        for kind in DroneKind::ALL {
            let speed = kind.params().hull.max_speed;
            assert!(
                speed < HullParams::TANK.max_speed,
                "{kind:?} does {speed}, the tank does {}",
                HullParams::TANK.max_speed
            );
        }
    }

    #[test]
    fn each_kind_is_faster_than_the_one_below_it() {
        for pair in DroneKind::ALL.windows(2) {
            let (slower, quicker) = (pair[0].params(), pair[1].params());
            assert!(
                quicker.hull.max_speed > slower.hull.max_speed,
                "{:?} is not quicker than {:?}",
                pair[1],
                pair[0]
            );
        }
    }

    #[test]
    fn a_drone_that_looks_before_firing_can_see_at_least_as_far_as_it_shoots() {
        // Otherwise it would decline shots it could have landed, or take shots
        // that give out in mid-air.
        for kind in DroneKind::ALL {
            let params = kind.params();
            if params.trigger == Trigger::OnSight {
                assert!(
                    params.sight <= params.weapon.range,
                    "{kind:?} sees {} but only shoots {}",
                    params.sight,
                    params.weapon.range
                );
            }
        }
    }

    #[test]
    fn only_the_assassin_pays_for_a_route() {
        for kind in DroneKind::ALL {
            let params = kind.params();
            assert_eq!(
                params.pursuit == Pursuit::Path,
                params.repath > 0.0,
                "{kind:?} disagrees with itself about pathfinding"
            );
        }
    }

    #[test]
    fn level_one_builds_nothing_but_the_dumbest_kind() {
        assert_eq!(spawn_weights(1), [1.0, 0.0, 0.0, 0.0]);
        for roll in ROLLS {
            assert_eq!(kind_at(1, roll), DroneKind::Drone, "roll {roll}");
        }
    }

    #[test]
    fn a_kind_is_never_built_before_its_level() {
        for level in 1..=12u32 {
            for step in 0..=200 {
                let kind = kind_at(level, step as f32 / 200.0);
                assert!(
                    kind.unlock_level() <= level,
                    "level {level} built a {kind:?}, unlocked at {}",
                    kind.unlock_level()
                );
            }
        }
    }

    #[test]
    fn every_unlocked_kind_is_actually_reachable() {
        // A weight that is nonzero but never selected would be a difficulty curve
        // that silently flattened out.
        for level in 1..=8u32 {
            let expected: Vec<DroneKind> = DroneKind::ALL
                .into_iter()
                .filter(|kind| kind.unlock_level() <= level)
                .collect();
            let mut seen: Vec<DroneKind> = Vec::new();
            for step in 0..=1_000 {
                let kind = kind_at(level, step as f32 / 1_000.0);
                if !seen.contains(&kind) {
                    seen.push(kind);
                }
            }
            seen.sort_by_key(|kind| kind.unlock_level());
            assert_eq!(seen, expected, "level {level}");
        }
    }

    #[test]
    fn the_mix_settles_on_every_kind_being_equally_likely() {
        // Not on all-assassins. A late level should be varied, and the cheap
        // drones are what keep something on the screen worth shooting at.
        let settled = spawn_weights(7);
        assert_eq!(settled, [3.0, 3.0, 3.0, 3.0]);
        assert_eq!(spawn_weights(40), settled, "the ceiling holds");
    }

    #[test]
    fn the_hardest_kind_gets_commoner_as_the_levels_go_on() {
        let share = |level: u32| {
            let weights = spawn_weights(level);
            weights[KIND_COUNT - 1] / weights.iter().sum::<f32>()
        };
        for level in 4..6u32 {
            assert!(
                share(level + 1) > share(level),
                "assassins are no commoner at level {} than at {level}",
                level + 1
            );
        }
        // And then it stops, at the ceiling every kind reaches.
        assert_eq!(share(6), share(9));
    }

    #[test]
    fn a_wanderer_only_ever_steps_onto_open_ground_next_door() {
        let maze = maze::generate(MazeParams::LEVEL_ONE, 41);
        for cell in maze.grid.open() {
            for roll in ROLLS {
                let next = wander_step(&maze.grid, cell, None, roll).expect("a way out");
                assert!(ORTHOGONAL.contains(&(next - cell)), "{cell:?} to {next:?}");
                assert!(maze.grid.is_open(next), "stepped into {next:?}");
            }
        }
    }

    #[test]
    fn a_wanderer_in_a_corridor_keeps_going_rather_than_doubling_back() {
        let grid = Grid::from_rows(&["#####", "#####", ".....", "#####", "#####"]);
        let at = IVec2::new(2, 2);
        let behind = IVec2::new(1, 2);
        for roll in ROLLS {
            assert_eq!(
                wander_step(&grid, at, Some(behind), roll),
                Some(IVec2::new(3, 2)),
                "roll {roll} turned round in a corridor"
            );
        }
    }

    #[test]
    fn a_wanderer_in_a_dead_end_turns_round_rather_than_stopping() {
        let grid = Grid::from_rows(&["#####", "#####", "..###", "#####", "#####"]);
        let at = IVec2::new(1, 2);
        let behind = IVec2::new(0, 2);
        assert_eq!(wander_step(&grid, at, Some(behind), 0.5), Some(behind));
    }

    #[test]
    fn a_wanderer_walled_in_has_nowhere_to_go() {
        let grid = Grid::from_rows(&["###", "#.#", "###"]);
        assert_eq!(wander_step(&grid, IVec2::new(1, 1), None, 0.5), None);
    }

    #[test]
    fn a_wanderer_uses_the_whole_of_its_roll() {
        // A junction with three ways on. If the roll did not spread across them,
        // one of the three would never be taken and a wanderer would trace the
        // same loop forever.
        let grid = Grid::from_rows(&["#.#", "...", "#.#"]);
        let at = IVec2::new(1, 1);
        let mut seen: Vec<IVec2> = Vec::new();
        let mut rng = SimRng::from_seed(2);
        for _ in 0..200 {
            let next =
                wander_step(&grid, at, Some(IVec2::new(0, 1)), rng.unit()).expect("a way on");
            if !seen.contains(&next) {
                seen.push(next);
            }
        }
        assert_eq!(seen.len(), 3, "only found {seen:?}");
    }

    #[test]
    fn a_hunter_closes_on_its_target_in_an_open_room() {
        let grid = Grid::from_rows(&[
            ".....", //
            ".....", ".....", ".....", ".....",
        ]);
        let target = IVec2::new(4, 4);
        let mut at = IVec2::new(0, 0);
        for _ in 0..8 {
            if at == target {
                break;
            }
            let next = greedy_step(&grid, at, target, None, 0.5).expect("a way on");
            assert!(
                (next - target).length_squared() < (at - target).length_squared(),
                "{at:?} to {next:?} did not close on {target:?}"
            );
            at = next;
        }
        assert_eq!(at, target, "never arrived");
    }

    #[test]
    fn a_hunter_takes_the_bait_of_a_dead_end_where_an_assassin_would_not() {
        // The whole difference between the two kinds, in one grid. The target is
        // straight up from the hunter; the only route to it is the long way round.
        // A hunter walks into the pocket because the pocket points the right way.
        let grid = pocket();
        let at = IVec2::new(2, 1);
        let target = IVec2::new(2, 4);
        assert_eq!(
            greedy_step(&grid, at, target, None, 0.5),
            Some(IVec2::new(2, 2)),
            "a hunter should have gone straight at it"
        );
        let route = crate::path::find_path(&grid, at, target).expect("the long way round");
        assert_eq!(
            route.next_step(),
            Some(IVec2::new(2, 0)),
            "an assassin should have gone the long way round"
        );
        assert_eq!(route.steps(), 9, "three cells apart, nine steps away");
    }

    #[test]
    fn a_hunter_with_nowhere_better_to_go_wanders_instead_of_freezing() {
        // Nose against the lid of the pocket, with the target on the far side of
        // it. Every neighbour but the one it came from is wall, and that one leads
        // away from the target, so nothing is an improvement and it has to fall
        // back on something. Standing still would make it free score.
        let grid = pocket();
        let at = IVec2::new(2, 2);
        let target = IVec2::new(2, 4);
        let next = greedy_step(&grid, at, target, None, 0.5).expect("a way out of the pocket");
        assert_eq!(next, IVec2::new(2, 1), "back down the way it came");
        assert!(grid.is_open(next));
        assert!(ORTHOGONAL.contains(&(next - at)));
    }

    #[test]
    fn a_hunter_standing_on_its_target_still_has_somewhere_to_be() {
        let grid = Grid::from_rows(&["...", "...", "..."]);
        let at = IVec2::new(1, 1);
        assert!(greedy_step(&grid, at, at, None, 0.5).is_some());
    }

    #[test]
    fn steering_is_a_pure_function_of_what_it_is_given() {
        let maze = maze::generate(MazeParams::LEVEL_ONE, 43);
        let target = maze.factories[0];
        for cell in maze.grid.open().step_by(11) {
            for roll in ROLLS {
                assert_eq!(
                    wander_step(&maze.grid, cell, None, roll),
                    wander_step(&maze.grid, cell, None, roll),
                );
                assert_eq!(
                    greedy_step(&maze.grid, cell, target, None, roll),
                    greedy_step(&maze.grid, cell, target, None, roll),
                );
            }
        }
    }

    #[test]
    fn repath_phases_spread_a_fleet_across_the_interval() {
        let interval = DroneKind::Assassin.params().repath;
        let phases: Vec<f32> = (0..12)
            .map(|serial| repath_phase(interval, serial))
            .collect();
        for phase in &phases {
            assert!(
                (0.0..interval).contains(phase),
                "phase {phase} out of range"
            );
        }
        for pair in phases.windows(2) {
            assert!(pair[0] != pair[1], "two drones would repath together");
        }
    }
}
