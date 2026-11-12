//! Signals: how far a phase material's hiding grabs, how far a wire carries, what a crystal
//! blast does, and how far a mender remembers.

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
    /// How far a mender remembers, in texels, rounded: every texel within this many of a
    /// mender piece, edges and corners counting (see [`crate::Property::Mender`]). 0 remembers
    /// only the mender itself.
    pub mend_reach: f32,
    /// The most texels one signal puts back; a mend needing more puts back nothing.
    pub mend_limit: u32,
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
    /// The farthest a mender remembers, in texels: a boxel, so what it remembers is always in
    /// its own boxel or one beside it.
    pub const MAX_MEND_REACH: f32 = 16.0;

    /// Checks the values make sense.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.grab.is_finite() && (0.0..=Self::MAX_GRAB).contains(&self.grab)) {
            return Err(format!("grab must be 0 to {}, got {}", Self::MAX_GRAB, self.grab));
        }
        if self.conduct_limit == 0 {
            return Err("conduct_limit must be at least 1".into());
        }
        if !(self.mend_reach.is_finite() && (0.0..=Self::MAX_MEND_REACH).contains(&self.mend_reach)) {
            return Err(format!("mend_reach must be 0 to {}, got {}", Self::MAX_MEND_REACH, self.mend_reach));
        }
        self.blast.validate()
    }

    /// The mend reach in whole texels.
    pub fn mend_texels(&self) -> i32 {
        (self.mend_reach.round() as i32).clamp(0, Self::MAX_MEND_REACH as i32)
    }

    /// The grab in whole texels.
    pub fn grab_texels(&self) -> i32 {
        (self.grab.round() as i32).clamp(0, Self::MAX_GRAB as i32)
    }
}

/// The tuning panel's groups for [`SignalDef`]'s values.
const SIGNAL_GROUPS: &[(&str, &[&str])] = &[
    ("Phasing", &["grab"]),
    ("Mending", &["mend_reach", "mend_limit"]),
    ("Crystal blast", &["blast_range", "blast_strength", "blast_radius", "blast_damage", "blast_player_damage", "blast_push"]),
];
/// What each of [`SignalDef`]'s values does, for the tuning panel.
const SIGNAL_HELP: &[(&str, &str)] = &[
    ("grab", "How far hiding a phase group takes other things with it, texels: everything within this many texels of the group, edges and corners counting. 0 takes only the group."),
    ("mend_reach", "How far a mender remembers, texels: every texel within this many of a mender piece, edges and corners counting. 0 remembers only the mender itself."),
    ("mend_limit", "The most texels one signal puts back (4096 is a full block); a mend needing more puts back nothing."),
    ("blast_range", "How far a crystal's beam runs, in boxels past the lens's own."),
    ("blast_strength", "How much the beam breaks before it stops, in full blocks' worth (a slab uses up half)."),
    ("blast_radius", "How far from the beam's middle line a body is hit, meters, past its own width."),
    ("blast_damage", "Health a creature in the beam loses."),
    ("blast_player_damage", "Health you lose standing in the beam."),
    ("blast_push", "How hard a body in the beam is thrown along it, m/s (a creature's weight softens it)."),
];

impl Tunables for SignalDef {
    fn tunables(&self) -> Vec<Tunable> {
        let b = &self.blast;
        crate::tuning::described(vec![
            Tunable::new("grab", self.grab, 0.0, Self::MAX_GRAB),
            Tunable::new("mend_reach", self.mend_reach, 0.0, Self::MAX_MEND_REACH),
            Tunable::new("mend_limit", self.mend_limit as f32, 0.0, 65536.0),
            Tunable::new("blast_range", b.range, 0.0, BlastDef::MAX_RANGE),
            Tunable::new("blast_strength", b.strength, 0.0, 64.0),
            Tunable::new("blast_radius", b.radius, 0.0, 4.0),
            Tunable::new("blast_damage", b.damage, 0.0, 200.0),
            Tunable::new("blast_player_damage", b.player_damage, 0.0, 20.0),
            Tunable::new("blast_push", b.push, 0.0, 60.0),
        ], SIGNAL_GROUPS, SIGNAL_HELP)
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "grab" => self.grab = value.clamp(0.0, Self::MAX_GRAB),
            "mend_reach" => self.mend_reach = value.clamp(0.0, Self::MAX_MEND_REACH),
            "mend_limit" => self.mend_limit = value.clamp(0.0, 1e7).round() as u32,
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
