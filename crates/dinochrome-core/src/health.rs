//! Hit points.
//!
//! Integers rather than floats, because "one more shell will do it" should be a
//! thing the player can count on rather than a thing that depends on rounding.

/// How much punishment something can still take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Health {
    current: i32,
    max: i32,
}

impl Health {
    /// Something at full health.
    ///
    /// # Panics
    ///
    /// If `max` is not positive. Nothing in this game is born dead, and a zero
    /// maximum would make [`fraction`] a division by zero.
    ///
    /// [`fraction`]: Self::fraction
    pub fn new(max: i32) -> Self {
        assert!(max > 0, "health must start above zero, got {max}");
        Self { current: max, max }
    }

    /// Hit points left. Can be negative after an overkill.
    pub fn current(&self) -> i32 {
        self.current
    }

    /// Hit points at full health.
    pub fn max(&self) -> i32 {
        self.max
    }

    /// Health left as a fraction of full, clamped to `0.0..=1.0`.
    ///
    /// This is what a damage tint or a health bar reads, so an overkill has to
    /// come back as empty rather than as a negative fraction.
    pub fn fraction(&self) -> f32 {
        self.current.max(0) as f32 / self.max as f32
    }

    /// True once there is nothing left.
    pub fn is_dead(&self) -> bool {
        self.current <= 0
    }

    /// Applies damage, and reports whether *this* blow was the killing one.
    ///
    /// Only one blow can ever be the killing one. Two shells landing on the same
    /// factory in the same tick both do damage, but the second one does not get to
    /// claim the kill as well — otherwise everything a death sets off happens
    /// twice.
    ///
    /// Negative damage is ignored rather than treated as a repair; healing is a
    /// separate decision and does not belong hidden inside a sign.
    pub fn damage(&mut self, amount: i32) -> bool {
        if self.is_dead() {
            return false;
        }
        self.current -= amount.max(0);
        self.is_dead()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn something_new_is_at_full_health() {
        let health = Health::new(100);
        assert_eq!(health.current(), 100);
        assert_eq!(health.max(), 100);
        assert_eq!(health.fraction(), 1.0);
        assert!(!health.is_dead());
    }

    #[test]
    #[should_panic(expected = "health must start above zero")]
    fn nothing_may_be_born_dead() {
        Health::new(0);
    }

    #[test]
    fn damage_comes_off_the_top_without_killing_early() {
        let mut health = Health::new(100);
        assert!(!health.damage(20));
        assert_eq!(health.current(), 80);
        assert_eq!(health.fraction(), 0.8);
        assert!(!health.is_dead());
    }

    #[test]
    fn the_blow_that_empties_it_is_the_one_that_reports_the_kill() {
        let mut health = Health::new(60);
        assert!(!health.damage(20));
        assert!(!health.damage(20));
        assert!(health.damage(20), "the third shell should finish it");
        assert!(health.is_dead());
    }

    #[test]
    fn only_one_blow_can_ever_claim_the_kill() {
        let mut health = Health::new(20);
        assert!(health.damage(20));
        assert!(!health.damage(20), "a second hit must not kill it again");
        assert!(!health.damage(999));
    }

    #[test]
    fn an_overkill_reads_as_empty_rather_than_as_negative() {
        let mut health = Health::new(20);
        assert!(health.damage(500));
        assert!(health.current() < 0, "the arithmetic is not clamped");
        assert_eq!(health.fraction(), 0.0, "but what is displayed is");
    }

    #[test]
    fn damage_of_zero_or_less_changes_nothing() {
        let mut health = Health::new(100);
        assert!(!health.damage(0));
        assert!(!health.damage(-50));
        assert_eq!(health.current(), 100);
    }
}
