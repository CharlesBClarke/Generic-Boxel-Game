//! How players break and place blocks. The engine owns the targeting and the rules that keep
//! edits valid; the mod supplies the numbers.

use crate::{Tunable, Tunables};

#[derive(Clone, Debug, PartialEq)]
pub struct InteractionDef {
    /// How far from the eyes blocks can be broken or placed, in meters.
    pub reach: f32,
    /// Breaks or placements per second while the button is held.
    pub repeat_rate: f32,
}

impl InteractionDef {
    /// Checks the values make sense.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.reach.is_finite() && self.reach > 0.0) {
            return Err(format!("reach must be positive, got {}", self.reach));
        }
        if !(self.repeat_rate.is_finite() && self.repeat_rate > 0.0) {
            return Err(format!("repeat_rate must be positive, got {}", self.repeat_rate));
        }
        Ok(())
    }
}

impl Tunables for InteractionDef {
    fn tunables(&self) -> Vec<Tunable> {
        vec![Tunable::new("reach", self.reach, 1.0, 20.0), Tunable::new("repeat_rate", self.repeat_rate, 0.5, 20.0)]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        let field = match name {
            "reach" => &mut self.reach,
            "repeat_rate" => &mut self.repeat_rate,
            _ => return false,
        };
        *field = value;
        true
    }
}
