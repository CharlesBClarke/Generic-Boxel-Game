//! How players break and place blocks, and what a block placed inside a body does to it. The
//! engine owns the targeting and the rules that keep edits valid; the mod supplies the numbers.

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

/// What happens when a solid block appears inside a body, such as a player placing one at
/// their own feet: the body is moved out of it the shortest way that has room, and launched.
/// Without one registered, the engine refuses to place blocks inside bodies.
#[derive(Clone, Debug, PartialEq)]
pub struct PushOutDef {
    /// Launch speed, meters per second.
    pub strength: f32,
    /// How much the launch leans upward. 0 pushes straight from the block's center to the
    /// body's; higher values point it more toward straight up.
    pub upward_bias: f32,
}

impl PushOutDef {
    /// Checks the values make sense.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.strength.is_finite() && self.strength >= 0.0) {
            return Err(format!("strength must be at least 0, got {}", self.strength));
        }
        if !(self.upward_bias.is_finite() && self.upward_bias >= 0.0) {
            return Err(format!("upward_bias must be at least 0, got {}", self.upward_bias));
        }
        Ok(())
    }
}

impl Tunables for PushOutDef {
    fn tunables(&self) -> Vec<Tunable> {
        vec![Tunable::new("strength", self.strength, 0.0, 30.0), Tunable::new("upward_bias", self.upward_bias, 0.0, 5.0)]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        let field = match name {
            "strength" => &mut self.strength,
            "upward_bias" => &mut self.upward_bias,
            _ => return false,
        };
        *field = value;
        true
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
