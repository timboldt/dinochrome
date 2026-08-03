//! Guns: what they fire, and how often they are allowed to.
//!
//! A weapon is a set of numbers and a cooldown. The cooldown lives here rather
//! than in a Bevy timer because fire rate is part of the simulation: it is counted
//! down in fixed ticks, so the same held trigger produces the same shells at the
//! same moments regardless of frame rate.

/// Tunable weapon characteristics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponParams {
    /// Shell speed, in world units per second.
    pub shell_speed: f32,
    /// Seconds between shots.
    pub cooldown: f32,
    /// How far a shell travels before it fizzles out, in world units.
    pub range: f32,
    /// Damage one shell does on impact.
    pub damage: i32,
}

impl WeaponParams {
    /// The player tank's main gun.
    ///
    /// A shell crosses one 64 px cell in a tenth of a second and gives out after
    /// nine of them. That range is further than the player can see and shorter
    /// than the maze, which is the balance being struck: it rewards looking down a
    /// corridor before committing to it, without letting anyone shell a factory
    /// from the far side of the level.
    pub const TANK: Self = Self {
        shell_speed: 640.0,
        cooldown: 0.45,
        range: 576.0,
        damage: 20,
    };

    /// A factory's close-in defence gun.
    ///
    /// A factory is a stationary target that builds the things trying to stop you
    /// reaching it, which makes parking outside its front door and shooting it at
    /// leisure the obvious degenerate strategy. This is the answer to that: short
    /// ranged, so it is no threat at all to a player working at a distance, and
    /// hard-hitting, so standing on top of one is not free.
    ///
    /// Four cells of reach against the tank's nine. You can always out-range it;
    /// the price is that you have to aim from further away.
    pub const FACTORY: Self = Self {
        shell_speed: 420.0,
        cooldown: 1.6,
        range: 256.0,
        damage: 12,
    };
}

impl Default for WeaponParams {
    fn default() -> Self {
        Self::TANK
    }
}

/// A gun, and how long until it can fire again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weapon {
    params: WeaponParams,
    cooldown: f32,
}

impl Weapon {
    /// A weapon loaded and ready to fire.
    pub fn new(params: WeaponParams) -> Self {
        Self {
            params,
            cooldown: 0.0,
        }
    }

    /// What this weapon fires.
    pub fn params(&self) -> WeaponParams {
        self.params
    }

    /// Seconds until it can fire again; zero when it is ready.
    pub fn cooldown(&self) -> f32 {
        self.cooldown
    }

    /// Counts one simulation tick off the cooldown.
    pub fn tick(&mut self, dt: f32) {
        self.cooldown = (self.cooldown - dt).max(0.0);
    }

    /// True if pulling the trigger right now would produce a shell.
    pub fn is_ready(&self) -> bool {
        self.cooldown <= 0.0
    }

    /// Fires if the weapon is ready, and reports whether it did.
    ///
    /// Callers are expected to spawn a shell exactly when this returns true, so it
    /// must not be called speculatively — it is the round leaving the barrel.
    pub fn fire(&mut self) -> bool {
        if !self.is_ready() {
            return false;
        }
        self.cooldown = self.params.cooldown;
        true
    }
}

impl Default for Weapon {
    fn default() -> Self {
        Self::new(WeaponParams::TANK)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FIXED_DT;

    /// Holds the trigger down for `ticks` and counts the shells that come out.
    fn held(weapon: &mut Weapon, ticks: u32) -> u32 {
        let mut shells = 0;
        for _ in 0..ticks {
            weapon.tick(FIXED_DT);
            if weapon.fire() {
                shells += 1;
            }
        }
        shells
    }

    #[test]
    fn a_fresh_weapon_can_fire_at_once() {
        let mut weapon = Weapon::default();
        assert!(weapon.is_ready());
        assert!(weapon.fire());
    }

    #[test]
    fn firing_locks_the_trigger_out_for_the_cooldown() {
        let mut weapon = Weapon::default();
        assert!(weapon.fire());
        assert!(!weapon.is_ready());
        assert!(!weapon.fire(), "a second shot in the same tick");
        assert_eq!(weapon.cooldown(), WeaponParams::TANK.cooldown);
    }

    #[test]
    fn a_failed_trigger_pull_does_not_extend_the_cooldown() {
        // Otherwise holding the trigger down would keep resetting the timer and
        // the gun would never fire again.
        let mut weapon = Weapon::default();
        weapon.fire();
        weapon.tick(FIXED_DT);
        let after_a_tick = weapon.cooldown();
        for _ in 0..10 {
            assert!(!weapon.fire());
        }
        assert_eq!(weapon.cooldown(), after_a_tick);
    }

    #[test]
    fn the_cooldown_runs_out_after_exactly_the_time_it_says() {
        let mut weapon = Weapon::default();
        weapon.fire();
        // 0.45 s at 60 Hz is 27 ticks.
        let ticks = (WeaponParams::TANK.cooldown / FIXED_DT).round() as u32;
        for tick in 1..ticks {
            weapon.tick(FIXED_DT);
            assert!(!weapon.is_ready(), "ready {tick} ticks in, too early");
        }
        weapon.tick(FIXED_DT);
        assert!(weapon.is_ready(), "should be ready after {ticks} ticks");
    }

    #[test]
    fn the_cooldown_never_runs_below_zero() {
        let mut weapon = Weapon::default();
        weapon.fire();
        for _ in 0..600 {
            weapon.tick(FIXED_DT);
        }
        assert_eq!(
            weapon.cooldown(),
            0.0,
            "an unbounded debt would delay a shot"
        );
    }

    #[test]
    fn a_held_trigger_fires_at_the_rate_the_cooldown_sets() {
        // Three seconds at one shot every 0.45 s: the first is free, then six more.
        let mut weapon = Weapon::default();
        let shells = held(&mut weapon, 180);
        assert_eq!(shells, 7, "3 s of held trigger");
    }

    #[test]
    fn fire_rate_does_not_depend_on_how_the_ticks_are_grouped() {
        let mut all_at_once = Weapon::default();
        let mut in_bursts = Weapon::default();
        let one_go = held(&mut all_at_once, 300);
        let split: u32 = (0..10).map(|_| held(&mut in_bursts, 30)).sum();
        assert_eq!(one_go, split);
        assert_eq!(all_at_once, in_bursts);
    }

    #[test]
    fn a_weapon_keeps_the_params_it_was_built_with() {
        let params = WeaponParams {
            shell_speed: 100.0,
            cooldown: 1.0,
            range: 200.0,
            damage: 3,
        };
        let mut weapon = Weapon::new(params);
        assert_eq!(weapon.params(), params);
        weapon.fire();
        assert_eq!(weapon.cooldown(), 1.0);
    }
}
