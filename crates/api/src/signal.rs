//! Signals: how far a phase material's hiding grabs, and how far a wire carries.

use crate::{Tunable, Tunables};

/// How signals behave. Without one registered, triggers do nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct SignalDef {
    /// How far hiding a phase group takes other things with it, in texels, rounded: everything
    /// within this many texels of the group, edges and corners counting. 0 takes nothing but
    /// the group. Not the interaction's reach, which is how far a player can break and place.
    pub grab: f32,
    /// The most boxels one signal reaches through conductors.
    pub conduct_limit: u32,
}

impl SignalDef {
    /// The farthest grab, in texels.
    pub const MAX_GRAB: f32 = 4.0;

    /// Checks the values make sense.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.grab.is_finite() && (0.0..=Self::MAX_GRAB).contains(&self.grab)) {
            return Err(format!("grab must be 0 to {}, got {}", Self::MAX_GRAB, self.grab));
        }
        if self.conduct_limit == 0 {
            return Err("conduct_limit must be at least 1".into());
        }
        Ok(())
    }

    /// The grab in whole texels.
    pub fn grab_texels(&self) -> i32 {
        (self.grab.round() as i32).clamp(0, Self::MAX_GRAB as i32)
    }
}

impl Tunables for SignalDef {
    fn tunables(&self) -> Vec<Tunable> {
        vec![Tunable::new("grab", self.grab, 0.0, Self::MAX_GRAB)]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "grab" => self.grab = value.clamp(0.0, Self::MAX_GRAB),
            _ => return false,
        }
        true
    }
}
