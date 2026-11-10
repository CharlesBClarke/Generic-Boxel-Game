//! Signals: how far a phase material's hiding grabs, how far a wire carries, and what a crystal
//! blast does.

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
    /// What a crystal blast does (see [`crate::Property::Lens`]).
    pub blast: BlastDef,
}

/// What a crystal blast does: a straight beam from the lens, one boxel wide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlastDef {
    /// How far the beam runs, in boxels past the lens's own.
    pub range: f32,
    /// How much it breaks before it stops, in full blocks' worth: a slab uses up half.
    pub strength: f32,
    /// How far from the beam's middle line a body is hit, in meters, past its own width.
    pub radius: f32,
    /// Health a creature in the beam loses.
    pub damage: f32,
    /// Health the player in the beam loses.
    pub player_damage: f32,
    /// How hard a body in the beam is thrown along it, in meters a second (a creature's weight
    /// softens it).
    pub push: f32,
}

impl BlastDef {
    /// The farthest a beam runs, in boxels.
    pub const MAX_RANGE: f32 = 64.0;

    /// Checks the values make sense.
    pub fn validate(&self) -> Result<(), String> {
        let ok = |v: f32, max: f32| v.is_finite() && (0.0..=max).contains(&v);
        if !ok(self.range, Self::MAX_RANGE) {
            return Err(format!("blast range must be 0 to {}, got {}", Self::MAX_RANGE, self.range));
        }
        for (name, v) in [("strength", self.strength), ("radius", self.radius), ("damage", self.damage)]
            .into_iter()
            .chain([("player_damage", self.player_damage), ("push", self.push)])
        {
            if !ok(v, f32::MAX) {
                return Err(format!("blast {name} must be 0 or more, got {v}"));
            }
        }
        Ok(())
    }
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
        self.blast.validate()
    }

    /// The grab in whole texels.
    pub fn grab_texels(&self) -> i32 {
        (self.grab.round() as i32).clamp(0, Self::MAX_GRAB as i32)
    }
}

impl Tunables for SignalDef {
    fn tunables(&self) -> Vec<Tunable> {
        let b = &self.blast;
        vec![
            Tunable::new("grab", self.grab, 0.0, Self::MAX_GRAB),
            Tunable::new("blast_range", b.range, 0.0, BlastDef::MAX_RANGE),
            Tunable::new("blast_strength", b.strength, 0.0, 64.0),
            Tunable::new("blast_radius", b.radius, 0.0, 4.0),
            Tunable::new("blast_damage", b.damage, 0.0, 200.0),
            Tunable::new("blast_player_damage", b.player_damage, 0.0, 20.0),
            Tunable::new("blast_push", b.push, 0.0, 60.0),
        ]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "grab" => self.grab = value.clamp(0.0, Self::MAX_GRAB),
            "blast_range" => self.blast.range = value.clamp(0.0, BlastDef::MAX_RANGE),
            "blast_strength" => self.blast.strength = value.max(0.0),
            "blast_radius" => self.blast.radius = value.max(0.0),
            "blast_damage" => self.blast.damage = value.max(0.0),
            "blast_player_damage" => self.blast.player_damage = value.max(0.0),
            "blast_push" => self.blast.push = value.max(0.0),
            _ => return false,
        }
        true
    }
}
