//! Signals: how far a phase material's hiding reaches, and how far a wire carries.

use crate::{Tunable, Tunables};

/// How signals behave. Without one registered, triggers do nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct SignalDef {
    /// How far hiding a phase group takes other things with it, in sixteenths, rounded:
    /// everything within this many sixteenths of the group, edges and corners counting. 0
    /// takes nothing but the group.
    pub reach: f32,
    /// The most boxels one signal reaches through conductors.
    pub conduct_limit: u32,
}

impl SignalDef {
    /// The longest reach, in sixteenths.
    pub const MAX_REACH: f32 = 4.0;

    /// Checks the values make sense.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.reach.is_finite() && (0.0..=Self::MAX_REACH).contains(&self.reach)) {
            return Err(format!("reach must be 0 to {}, got {}", Self::MAX_REACH, self.reach));
        }
        if self.conduct_limit == 0 {
            return Err("conduct_limit must be at least 1".into());
        }
        Ok(())
    }

    /// The reach in whole sixteenths.
    pub fn reach_texels(&self) -> i32 {
        (self.reach.round() as i32).clamp(0, Self::MAX_REACH as i32)
    }
}

impl Tunables for SignalDef {
    fn tunables(&self) -> Vec<Tunable> {
        vec![Tunable::new("reach", self.reach, 0.0, Self::MAX_REACH)]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "reach" => self.reach = value.clamp(0.0, Self::MAX_REACH),
            _ => return false,
        }
        true
    }
}
