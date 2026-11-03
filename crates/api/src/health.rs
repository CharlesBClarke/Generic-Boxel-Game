//! The player's health: how much they have, how it comes back, and how a guard takes the
//! sting out of blows. The engine keeps the health and applies hits the same way for every
//! mod's creatures; the numbers are content.

use crate::{Tunable, Tunables};

/// Without one registered, the player can't be hurt, though blows still knock them about.
#[derive(Clone, Debug, PartialEq)]
pub struct HealthDef {
    /// Health when whole. At none, the player goes back to the spawn point, whole again.
    pub health: f32,
    /// Health back per second, once `regen_delay` seconds have passed since the last hit.
    pub regen: f32,
    pub regen_delay: f32,
    /// How far off the player's look, in radians either way, an attacker can be for a raised
    /// guard to catch its blow.
    pub guard_angle: f32,
}

impl HealthDef {
    /// Checks the values make sense.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.health.is_finite() && self.health > 0.0) {
            return Err(format!("health must be above 0, got {}", self.health));
        }
        for (name, value) in [("regen", self.regen), ("regen_delay", self.regen_delay), ("guard_angle", self.guard_angle)] {
            if !(value.is_finite() && value >= 0.0) {
                return Err(format!("{name} must be at least 0, got {value}"));
            }
        }
        Ok(())
    }
}

impl Tunables for HealthDef {
    fn tunables(&self) -> Vec<Tunable> {
        vec![
            Tunable::new("health", self.health, 1.0, 200.0),
            Tunable::new("regen", self.regen, 0.0, 20.0),
            Tunable::new("regen_delay", self.regen_delay, 0.0, 20.0),
            Tunable::new("guard_angle", self.guard_angle, 0.0, std::f32::consts::PI),
        ]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        let field = match name {
            "health" => &mut self.health,
            "regen" => &mut self.regen,
            "regen_delay" => &mut self.regen_delay,
            "guard_angle" => &mut self.guard_angle,
            _ => return false,
        };
        *field = value;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def() -> HealthDef {
        HealthDef { health: 20.0, regen: 1.0, regen_delay: 4.0, guard_angle: 1.0 }
    }

    #[test]
    fn sensible_values_pass_and_bad_ones_are_named() {
        assert_eq!(def().validate(), Ok(()));
        let err = HealthDef { health: 0.0, ..def() }.validate().unwrap_err();
        assert!(err.contains("health"), "{err}");
        assert!(HealthDef { regen: -1.0, ..def() }.validate().is_err());
        assert!(HealthDef { guard_angle: f32::NAN, ..def() }.validate().is_err());
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
