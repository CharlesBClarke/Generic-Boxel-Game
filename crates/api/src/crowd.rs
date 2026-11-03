//! How creatures share space: how far they see each other, and how overlapping bodies, the
//! player's included, are pushed apart. The engine does the pushing the same way for every
//! mod's creatures; the numbers are content.

use crate::{Tunable, Tunables};

/// Without one registered, creatures don't see each other and walk through each other and the
/// player.
#[derive(Clone, Debug, PartialEq)]
pub struct CrowdDef {
    /// How far a creature sees others, in meters, feet to feet.
    pub sight: f32,
    /// How fast overlapping bodies move apart, per meter of overlap, in meters per second: the
    /// deeper they're in each other, the harder they're pushed.
    pub push: f32,
    /// The fastest a pair is pushed apart, in meters per second, however deep the overlap.
    pub max_push: f32,
    /// The player's weight against creatures' [`crate::TargetDef::weight`]: of a push between
    /// them, each moves the other's share of their two weights.
    pub player_weight: f32,
}

impl CrowdDef {
    /// Checks the values make sense.
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [("sight", self.sight), ("push", self.push), ("max_push", self.max_push)] {
            if !(value.is_finite() && value >= 0.0) {
                return Err(format!("{name} must be at least 0, got {value}"));
            }
        }
        if !(self.player_weight.is_finite() && self.player_weight > 0.0) {
            return Err(format!("player_weight must be above 0, got {}", self.player_weight));
        }
        Ok(())
    }
}

impl Tunables for CrowdDef {
    fn tunables(&self) -> Vec<Tunable> {
        vec![
            Tunable::new("sight", self.sight, 0.0, 30.0),
            Tunable::new("push", self.push, 0.0, 40.0),
            Tunable::new("max_push", self.max_push, 0.0, 10.0),
            Tunable::new("player_weight", self.player_weight, 0.1, 10.0),
        ]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        let field = match name {
            "sight" => &mut self.sight,
            "push" => &mut self.push,
            "max_push" => &mut self.max_push,
            "player_weight" => &mut self.player_weight,
            _ => return false,
        };
        *field = value;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def() -> CrowdDef {
        CrowdDef { sight: 12.0, push: 10.0, max_push: 3.0, player_weight: 2.0 }
    }

    #[test]
    fn sensible_values_pass_and_bad_ones_are_named() {
        assert_eq!(def().validate(), Ok(()));
        let err = CrowdDef { push: -1.0, ..def() }.validate().unwrap_err();
        assert!(err.contains("push"), "{err}");
        assert!(CrowdDef { player_weight: 0.0, ..def() }.validate().is_err());
        assert!(CrowdDef { sight: f32::NAN, ..def() }.validate().is_err());
    }

    #[test]
    fn every_value_is_tunable() {
        let mut d = def();
        for t in d.tunables() {
            assert!(d.set_tunable(&t.name, t.min), "{}", t.name);
        }
        assert!(!d.set_tunable("missing", 1.0));
    }
}
