//! Brains decide what a creature wants: where to walk, which way to face, which clip to play.
//! The engine owns the body and moves it, with the same collision, stepping and edge guarding
//! as the player's, so knockback and terrain work the same whatever the brain wants.
//!
//! A brain keeps nothing between calls. Each creature's state lives in the engine as plain
//! data, its `memory`, passed in on every call and handed back changed, so one brain serves
//! every creature of its kind and a mod's own memory stays small however many there are.

use glam::{Vec2, Vec3};

use crate::Tunables;

/// The player, as a creature sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    /// From the creature's feet to the player's, meters, Z-up.
    pub offset: Vec3,
    pub velocity: Vec3,
}

/// How many other creatures a brain is shown at most, nearest first.
pub const MAX_NEIGHBORS: usize = 8;

/// Another creature, as a creature sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Neighbor {
    /// From this creature's feet to the other's, meters, Z-up.
    pub offset: Vec3,
    pub velocity: Vec3,
    /// Its kind's registered name, such as `"base:cow"`.
    pub kind: String,
    /// Stays the same for as long as it lives, and newer creatures get bigger ones, so brains can
    /// agree on things like a leader without talking.
    pub id: u64,
    /// What its brain is showing, from [`BrainOutput::signal`].
    pub signal: String,
    /// Its health, as a share of its full health: 1 is unhurt.
    pub health: f32,
}

/// Something hurt the creature since its last think.
#[derive(Clone, Debug, PartialEq)]
pub struct Hurt {
    /// Level unit direction from the creature toward where the hurt came from.
    pub from: Vec3,
    pub damage: f32,
}

/// Everything a brain sees for one tick.
#[derive(Clone, Debug, PartialEq)]
pub struct BrainContext {
    /// Seconds this tick covers.
    pub dt: f32,
    /// This creature's [`Neighbor::id`], as others see it.
    pub id: u64,
    pub velocity: Vec3,
    /// Facing, radians counterclockwise from +X (east) around +Z.
    pub yaw: f32,
    pub on_ground: bool,
    /// Its sideways movement was stopped by something last tick: a wall, or a drop the edge
    /// guard kept it from.
    pub blocked: bool,
    pub health: f32,
    /// Health when whole, from its definition.
    pub full_health: f32,
    /// The player, if there is one.
    pub player: Option<Seen>,
    pub hurt: Option<Hurt>,
    /// The attack it last sent ([`BrainOutput::attack`]) hit the player since its last think.
    pub attack_landed: bool,
    /// Other creatures within sight (see [`crate::CrowdDef`]), nearest first, at most
    /// [`MAX_NEIGHBORS`]. Empty without a crowd definition.
    pub others: Vec<Neighbor>,
    /// A random number for this creature and tick. The same game gives the same numbers, so
    /// replays and multiplayer stay in step.
    pub random: u64,
    /// What the brain handed back last tick; empty for a newly spawned creature.
    pub memory: Vec<f32>,
}

impl BrainContext {
    /// `random` as a number from 0 to 1, picked by `salt` so several can be drawn per tick.
    pub fn random_unit(&self, salt: u64) -> f32 {
        let mut x = (self.random ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15)).wrapping_mul(0x2545_f491_4f6c_dd1d);
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^= x >> 33;
        (x >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// A brain's decision for one tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrainOutput {
    /// The level velocity it wants, meters per second, world space. On the ground the body
    /// eases toward it at its drag; in the air it can't steer.
    pub walk: Vec2,
    /// Its new facing, radians counterclockwise from +X. The brain turns as fast as it likes.
    pub yaw: f32,
    /// Keeps it from walking off edges, like [`crate::MoveOutput::edge_guard`]: the engine cuts
    /// level movement that would leave nothing solid under its feet within this many meters.
    pub edge_guard: Option<f32>,
    /// The clip its model plays, by name. A clip that doesn't loop plays once each time the
    /// brain starts asking for it, then holds its last pose. Empty leaves the choice to the
    /// renderer.
    pub clip: String,
    /// How fast the clip plays, as a share of its own speed.
    pub clip_speed: f32,
    /// A short label other creatures see as [`Neighbor::signal`], such as `"alarmed"`. What it
    /// means is up to the brains; empty shows nothing.
    pub signal: String,
    /// An attack under way this tick, if any. See [`Attack`].
    pub attack: Option<Attack>,
    /// Handed back on the next call.
    pub memory: Vec<f32>,
}

/// A blow a creature is dealing. While its brain keeps sending one, tick after tick, the
/// engine tests it against the player and hits them at most once; to hit again, the brain
/// stops sending it for at least a tick. Lag rules stay with the engine, so brains only say
/// what the blow is and when it's live, and telegraph it with a wind-up first.
#[derive(Clone, Debug, PartialEq)]
pub struct Attack {
    /// The box it hits, in the creature's own space like [`crate::HitBox`]: from its feet,
    /// meters, with its front along +Y, turned with its facing.
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub damage: f32,
    /// Knockback, meters per second: level, away from the creature, and up.
    pub push: f32,
    pub lift: f32,
    /// What gets through a guard that faces it: shares of the damage and of the knockback.
    pub guarded_damage: f32,
    pub guarded_push: f32,
}

/// Decides what one kind of creature wants. One per kind, shared by every creature of it, so
/// it keeps no state of its own: what it needs between ticks goes in `memory`.
pub trait Brain: Tunables + Send + Sync {
    fn think(&self, ctx: &BrainContext) -> BrainOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(random: u64) -> BrainContext {
        BrainContext {
            dt: 1.0 / 60.0,
            id: 0,
            velocity: Vec3::ZERO,
            yaw: 0.0,
            on_ground: true,
            blocked: false,
            health: 1.0,
            full_health: 1.0,
            player: None,
            hurt: None,
            attack_landed: false,
            others: Vec::new(),
            random,
            memory: Vec::new(),
        }
    }

    #[test]
    fn random_units_are_in_range_and_differ_by_salt() {
        let mut sum = 0.0;
        for seed in 0..1000 {
            let c = ctx(seed);
            let (a, b) = (c.random_unit(0), c.random_unit(1));
            assert!((0.0..1.0).contains(&a) && (0.0..1.0).contains(&b));
            assert_eq!(a, ctx(seed).random_unit(0), "the same seed gives the same number");
            sum += a;
        }
        assert!((sum / 1000.0 - 0.5).abs() < 0.05, "spread evenly: mean {}", sum / 1000.0);
        assert_ne!(ctx(7).random_unit(0), ctx(7).random_unit(1));
    }
}
