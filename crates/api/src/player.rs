//! Player parameters. The engine owns how movement works; the mod supplies every number.

/// How the player is shaped and moves, in meters and seconds. One boxel is one meter.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerDef {
    /// Width and depth of the collision box.
    pub width: f32,
    /// Height of the collision box, from the feet.
    pub height: f32,
    /// Camera height above the feet.
    pub eye_height: f32,
    /// Ground speed, meters per second.
    pub walk_speed: f32,
    /// How high a jump lifts the feet, in meters.
    pub jump_height: f32,
    /// Downward acceleration, meters per second squared.
    pub gravity: f32,
    /// Speed in fly mode, meters per second.
    pub fly_speed: f32,
}

impl PlayerDef {
    /// Checks the values make physical sense.
    pub fn validate(&self) -> Result<(), String> {
        let positive = [
            ("width", self.width),
            ("height", self.height),
            ("walk_speed", self.walk_speed),
            ("jump_height", self.jump_height),
            ("gravity", self.gravity),
            ("fly_speed", self.fly_speed),
        ];
        for (field, value) in positive {
            if !(value.is_finite() && value > 0.0) {
                return Err(format!("{field} must be positive, got {value}"));
            }
        }
        if !(self.eye_height > 0.0 && self.eye_height <= self.height) {
            return Err(format!("eye_height must be within the body, got {}", self.eye_height));
        }
        Ok(())
    }
}
