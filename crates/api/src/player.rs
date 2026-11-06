//! Player definitions. The engine owns the body and collisions; the mod supplies its shape
//! and names the controller that decides how it moves.

use crate::PlayerSounds;

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerDef {
    /// Width and depth of the collision box, in meters. One boxel is one meter.
    pub width: f32,
    /// Height of the collision box, from the feet.
    pub height: f32,
    /// Camera height above the feet.
    pub eye_height: f32,
    /// Ledges up to this height are climbed automatically while walking. 0 turns it off.
    pub step_height: f32,
    /// The registered movement controller's name, such as `"base:quake"`.
    pub controller: String,
    /// What the body sounds like.
    pub sounds: PlayerSounds,
}

impl PlayerDef {
    /// Checks the shape makes physical sense.
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [("width", self.width), ("height", self.height)] {
            if !(value.is_finite() && value > 0.0) {
                return Err(format!("{field} must be positive, got {value}"));
            }
        }
        if !(self.eye_height > 0.0 && self.eye_height <= self.height) {
            return Err(format!("eye_height must be within the body, got {}", self.eye_height));
        }
        if !(self.step_height >= 0.0 && self.step_height < self.height) {
            return Err(format!("step_height must be at least 0 and below the height, got {}", self.step_height));
        }
        Ok(())
    }
}
