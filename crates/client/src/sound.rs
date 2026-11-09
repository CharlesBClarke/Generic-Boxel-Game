//! Sound: what the player hears, played through `kira` from where things happen.
//!
//! The simulation is silent. Each frame the client works out what was heard from the world it
//! was sent: events (strikes, slams, blows, block edits, crafts) and changes it sees (a landing
//! counted, a stride of the head bob, a cow covering ground, the inventory growing, the view
//! turning). [`Hearing::listen`] turns those into [`Cue`]s, plain data that tests can check;
//! [`play_sounds`] hands them to the mixer.
//!
//! The mixer has one track per [`Group`] under the main track, each with its own volume. A cue
//! with a place gets its own spatial track under its group, so it pans and fades from where it
//! happened; one without (the player's own body) plays straight on its group. Spatial
//! positions are render positions, Y-up and relative to the render origin, converted only
//! through [`coords`].
//!
//! Two loops are started once and never stopped, their loudness and pitch following a
//! [`Drive`]: the servo, by how fast the view turns, and the wind, by how fast the body moves.
//! Each is silent until driven, so normal aiming and walking are quiet.

use std::collections::HashMap;
use std::io::Cursor;

use api::{PlayerSounds, Tunable, Tunables, Vec3 as ZVec3, WorldPos};
use bevy::prelude::*;
use kira::listener::ListenerHandle;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::track::{SpatialTrackBuilder, TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween};
use protocol::{Defs, Event};
use sim::{BlockRegistry, Body, Grid, Target};

use crate::coords;
use crate::feel::FeelState;
use crate::world::ClientWorld;
use crate::{Look, PlayerCamera, RenderOrigin};

/// Which volume slider a sound answers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// The player's steps and landings.
    Movement,
    /// Breaking and placing blocks.
    Blocks,
    /// Swings, hits, slams, guards and blows.
    Combat,
    /// Creatures' voices and steps.
    Creatures,
    /// The robot's own noises: the servo, pickups, crafting.
    Robot,
}

impl Group {
    pub const ALL: [Group; 5] = [Group::Movement, Group::Blocks, Group::Combat, Group::Creatures, Group::Robot];

    fn index(self) -> usize {
        self as usize
    }
}

/// A sound to play once.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    /// A registered sound's name.
    pub sound: String,
    /// Where it happens; `None` for the player's own body, heard in the middle of the head.
    pub at: Option<WorldPos>,
    pub group: Group,
    /// A gain on the sound's own volume.
    pub volume: f32,
    /// A playback speed on the sound's own pitch.
    pub pitch: f32,
}

impl Cue {
    fn new(sound: &str, at: Option<WorldPos>, group: Group) -> Self {
        Self { sound: sound.to_owned(), at, group, volume: 1.0, pitch: 1.0 }
    }

    fn volume(self, volume: f32) -> Self {
        Self { volume, ..self }
    }

    fn pitch(self, pitch: f32) -> Self {
        Self { pitch, ..self }
    }
}

/// The client's sound values, all in the tuning panel.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct SoundTuning {
    /// Gains: everything, then each group.
    pub master: f32,
    pub movement: f32,
    pub blocks: f32,
    pub combat: f32,
    pub creatures: f32,
    pub robot: f32,
    /// How strongly a sound pans toward the ear nearer it: 0 hears everything in the middle.
    pub spatial: f32,
    /// Distance, meters, within which a sound plays at full loudness, and beyond which it's
    /// silent; it fades evenly between.
    pub near: f32,
    pub far: f32,
    /// Landing speeds, m/s: no thud below `land_quiet`, a full one from `land_full`.
    pub land_quiet: f32,
    pub land_full: f32,
    /// Meters a creature walks per footstep.
    pub creature_stride: f32,
    /// How much louder than a footstep a swing sounds when it hits the block's surface.
    pub block_hit_volume: f32,
    /// How much louder than a footstep a full landing (from `land_full`) sounds, and how much
    /// lower in pitch, as a share.
    pub land_loudness: f32,
    pub land_pitch_drop: f32,
    /// How loud the picked-up material's place sound plays under a pick-up, and the fewest
    /// seconds between pick-up sounds, so a burst doesn't pile up.
    pub pickup_material: f32,
    pub pickup_gap: f32,
    /// The servo: silent below `servo_start` (radians a second of view turn), loudest from
    /// `servo_full`, at `servo_volume`; pitch from `servo_pitch_low` to `servo_pitch_high`
    /// across the same range. It fades in over `servo_attack` seconds and out over
    /// `servo_release`.
    pub servo_volume: f32,
    pub servo_start: f32,
    pub servo_full: f32,
    pub servo_pitch_low: f32,
    pub servo_pitch_high: f32,
    pub servo_attack: f32,
    pub servo_release: f32,
    /// The wind, the same way by the body's speed in meters a second.
    pub wind_volume: f32,
    pub wind_start: f32,
    pub wind_full: f32,
    pub wind_pitch_low: f32,
    pub wind_pitch_high: f32,
    pub wind_attack: f32,
    pub wind_release: f32,
}

impl Default for SoundTuning {
    fn default() -> Self {
        Self {
            master: 0.8,
            movement: 1.0,
            blocks: 1.0,
            combat: 1.0,
            creatures: 1.6,
            robot: 1.0,
            spatial: 0.75,
            near: 1.5,
            far: 40.0,
            land_quiet: 2.0,
            land_full: 14.0,
            creature_stride: 1.1,
            block_hit_volume: 12.0,
            land_loudness: 3.0,
            land_pitch_drop: 0.0,
            pickup_material: 0.5,
            pickup_gap: 0.06,
            servo_volume: 0.06,
            servo_start: 0.0,
            servo_full: 11.0,
            servo_pitch_low: 0.75,
            servo_pitch_high: 1.8,
            servo_attack: 0.02,
            servo_release: 0.03,
            // Well over a sprint, to full on a long fall.
            wind_volume: 0.3,
            wind_start: 10.0,
            wind_full: 30.0,
            wind_pitch_low: 0.6,
            wind_pitch_high: 0.9,
            wind_attack: 0.15,
            wind_release: 0.4,
        }
    }
}

macro_rules! tunable_fields {
    ($($field:ident: $min:expr, $max:expr;)*) => {
        impl Tunables for SoundTuning {
            fn tunables(&self) -> Vec<Tunable> {
                vec![$(Tunable::new(stringify!($field), self.$field, $min, $max)),*]
            }

            fn set_tunable(&mut self, name: &str, value: f32) -> bool {
                match name {
                    $(stringify!($field) => self.$field = value,)*
                    _ => return false,
                }
                true
            }
        }
    };
}

tunable_fields! {
    master: 0.0, 2.0;
    movement: 0.0, 2.0;
    blocks: 0.0, 2.0;
    combat: 0.0, 2.0;
    creatures: 0.0, 2.0;
    robot: 0.0, 2.0;
    spatial: 0.0, 1.0;
    near: 0.1, 10.0;
    far: 5.0, 150.0;
    land_quiet: 0.0, 10.0;
    land_full: 2.0, 40.0;
    creature_stride: 0.3, 4.0;
    block_hit_volume: 0.0, 20.0;
    land_loudness: 0.0, 8.0;
    land_pitch_drop: 0.0, 0.6;
    pickup_material: 0.0, 2.0;
    pickup_gap: 0.0, 0.5;
    servo_volume: 0.0, 1.0;
    servo_start: 0.0, 20.0;
    servo_full: 1.0, 40.0;
    servo_pitch_low: 0.25, 4.0;
    servo_pitch_high: 0.25, 4.0;
    servo_attack: 0.0, 0.5;
    servo_release: 0.0, 1.0;
    wind_volume: 0.0, 2.0;
    wind_start: 0.0, 40.0;
    wind_full: 1.0, 80.0;
    wind_pitch_low: 0.25, 4.0;
    wind_pitch_high: 0.25, 4.0;
    wind_attack: 0.0, 2.0;
    wind_release: 0.0, 3.0;
}

impl SoundTuning {
    fn group(&self, group: Group) -> f32 {
        match group {
            Group::Movement => self.movement,
            Group::Blocks => self.blocks,
            Group::Combat => self.combat,
            Group::Creatures => self.creatures,
            Group::Robot => self.robot,
        }
    }
}

/// How a loop follows what drives it: silent below `start`, loudest from `full`, pitch from
/// `pitch_low` to `pitch_high` across the same range, fading in over `attack` seconds and out
/// over `release`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Envelope {
    pub start: f32,
    pub full: f32,
    pub pitch_low: f32,
    pub pitch_high: f32,
    pub attack: f32,
    pub release: f32,
}

impl SoundTuning {
    pub fn servo(&self) -> Envelope {
        Envelope {
            start: self.servo_start,
            full: self.servo_full,
            pitch_low: self.servo_pitch_low,
            pitch_high: self.servo_pitch_high,
            attack: self.servo_attack,
            release: self.servo_release,
        }
    }

    pub fn wind(&self) -> Envelope {
        Envelope {
            start: self.wind_start,
            full: self.wind_full,
            pitch_low: self.wind_pitch_low,
            pitch_high: self.wind_pitch_high,
            attack: self.wind_attack,
            release: self.wind_release,
        }
    }
}

/// How loud and how high a driven loop plays.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Drive {
    /// From 0, silent, to 1, as loud as its volume allows.
    pub level: f32,
    pub pitch: f32,
}

impl Drive {
    /// Follows `input` (a turn or a speed) through `envelope` for `dt` seconds.
    pub fn update(&mut self, envelope: Envelope, input: f32, dt: f32) {
        let span = (envelope.full - envelope.start).max(1e-3);
        let drive = ((input - envelope.start) / span).clamp(0.0, 1.0);
        let time = if drive > self.level { envelope.attack } else { envelope.release };
        self.level = if time <= 0.0 { drive } else { drive + (self.level - drive) * (-dt / time).exp() };
        // Pitch follows how hard it's driven, and holds while it fades.
        if drive > 0.0 || self.pitch == 0.0 {
            self.pitch = envelope.pitch_low + (envelope.pitch_high - envelope.pitch_low) * drive;
        }
    }
}

/// What the client heard last frame, to tell what's new this one.
#[derive(Resource, Default)]
pub struct Hearing {
    seen_landings: Option<u32>,
    seen_strides: Option<i64>,
    /// What was carried, by material.
    seen_inventory: Option<HashMap<sim::MaterialId, u32>>,
    /// Seconds before another pick-up may sound.
    pickup_wait: f32,
    /// Meters each creature has walked since its last footstep, by ID.
    creature_walked: HashMap<u64, f32>,
    last_look: Option<(f32, f32)>,
    pub servo: Drive,
    pub wind: Drive,
}

/// What the client sees this frame that sound depends on.
pub struct Frame<'a> {
    pub defs: &'a Defs,
    pub blocks: &'a BlockRegistry,
    pub grid: &'a Grid,
    pub player: Option<&'a Body>,
    /// Head-bob strides taken so far (see [`FeelState::strides`]).
    pub strides: i64,
    /// Landing speed from which the shock absorber bottoms out.
    pub hard_landing_speed: f32,
    pub inventory: &'a sim::Inventory,
    /// Sounds being tried in place of the usual ones.
    pub picks: &'a SoundPicks,
    pub targets: &'a [Target],
    /// Events since the last frame, oldest first.
    pub events: &'a [Event],
    /// How much higher a head hit's voice plays.
    pub crit_pitch: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub dt: f32,
}

impl Hearing {
    /// The cues `frame` makes, and moves the servo and the wind.
    pub fn listen(&mut self, tuning: &SoundTuning, frame: &Frame) -> Vec<Cue> {
        let mut cues = Vec::new();
        let player_sounds = frame.defs.player.as_ref().map(|p| &p.sounds);
        let none = PlayerSounds::default();
        let me = player_sounds.unwrap_or(&none);

        if let Some(body) = frame.player {
            let walking = body.on_ground() && !body.flying();
            // A stride of the bob is a footstep, so steps keep time with the bob and quicken
            // with speed. Not while flying, falling or standing still.
            if self.seen_strides.is_some_and(|seen| frame.strides != seen)
                && walking
                && body.velocity().truncate().length() > 0.5
                && let Some(step) = step_sound(frame.blocks, frame.grid, body.pos(), me, frame.picks)
            {
                cues.push(step);
            }
            if self.seen_landings.is_some_and(|seen| body.landings() != seen) {
                let surface = step_sound(frame.blocks, frame.grid, body.pos(), me, frame.picks);
                let hard = frame.picks.combat(Kind::HardLand, me.hard_land.as_ref(), None, Group::Movement);
                cues.extend(landing(tuning, me, surface, hard, body.last_landing_speed(), frame.hard_landing_speed));
            }
            self.seen_strides = Some(frame.strides);
            self.seen_landings = Some(body.landings());
        }

        let crafted = frame.events.iter().any(|e| matches!(e, Event::Craft(_)));
        for event in frame.events {
            event_cues(event, tuning, frame, me, &mut cues);
        }
        // More of a material carried without a craft is a pickup: the robot's sound, and the
        // material's own place sound under it. A burst of pickups sounds once a `pickup_gap`.
        let carried: HashMap<sim::MaterialId, u32> = frame.inventory.iter().collect();
        self.pickup_wait -= frame.dt;
        if let Some(seen) = &self.seen_inventory
            && !crafted
            && self.pickup_wait <= 0.0
        {
            let mut gained: Vec<sim::MaterialId> =
                carried.iter().filter(|(m, amount)| **amount > seen.get(m).copied().unwrap_or(0)).map(|(m, _)| *m).collect();
            gained.sort();
            if let Some(&material) = gained.first() {
                self.pickup_wait = tuning.pickup_gap;
                cues.extend(frame.picks.combat(Kind::Pickup, me.pickup.as_ref(), None, Group::Robot));
                let def = frame.blocks.get(frame.blocks.material_block(material));
                if tuning.pickup_material > 0.0
                    && let Some(placed) = def.and_then(|d| d.sounds.placed.as_ref())
                {
                    cues.push(Cue::new(placed, None, Group::Robot).volume(tuning.pickup_material));
                }
            }
        }
        self.seen_inventory = Some(carried);

        self.creature_steps(tuning, frame, &mut cues);

        let turn = match self.last_look {
            Some((yaw, pitch)) if frame.dt > 0.0 => {
                let yawed = (frame.yaw - yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                yawed.hypot(frame.pitch - pitch) / frame.dt
            }
            _ => 0.0,
        };
        self.last_look = Some((frame.yaw, frame.pitch));
        self.servo.update(tuning.servo(), turn, frame.dt);
        let speed = frame.player.map_or(0.0, |b| b.velocity().length());
        self.wind.update(tuning.wind(), speed, frame.dt);
        cues
    }

    /// A footstep for every stride a creature walks on the ground.
    fn creature_steps(&mut self, tuning: &SoundTuning, frame: &Frame, cues: &mut Vec<Cue>) {
        self.creature_walked.retain(|id, _| frame.targets.iter().any(|t| t.id == *id));
        for target in frame.targets {
            let Some(step) = frame.defs.targets.get(target.kind.index()).and_then(|d| d.sounds.step.as_ref()) else {
                continue;
            };
            let walked = self.creature_walked.entry(target.id).or_default();
            if !target.on_ground {
                continue;
            }
            *walked += target.velocity.truncate().length() * frame.dt;
            if *walked >= tuning.creature_stride {
                *walked %= tuning.creature_stride.max(0.01);
                cues.push(Cue::new(step, Some(target.pos), Group::Creatures));
            }
        }
    }
}

/// A sound picked in the tuning panel to try in place of the usual one, and a gain and speed
/// on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub sound: String,
    pub volume: f32,
    pub pitch: f32,
}

impl Pick {
    fn cue(&self, at: Option<WorldPos>, group: Group) -> Cue {
        Cue::new(&self.sound, at, group).volume(self.volume).pitch(self.pitch)
    }
}

/// Picks by what they replace: a surface's name, or a [`Kind`]'s key.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Picks(pub HashMap<String, Pick>);

impl Picks {
    /// The picks as `(key, "sound volume pitch")`, by key, for the tuning file.
    pub fn to_lines(&self) -> Vec<(String, String)> {
        let mut lines: Vec<(String, String)> =
            self.0.iter().map(|(key, p)| (key.clone(), format!("{} {:.2} {:.2}", p.sound, p.volume, p.pitch))).collect();
        lines.sort();
        lines
    }

    /// Picks read back from [`Picks::to_lines`]' text. Lines that don't read, or name a key or
    /// sound not among `keys` and `sounds`, are skipped and described in `problems`, which
    /// name the tuning file's `section`.
    pub fn from_lines(
        section: &str,
        lines: &[(String, String)],
        keys: &[String],
        sounds: &[String],
        problems: &mut Vec<String>,
    ) -> Self {
        let mut picks = Self::default();
        for (key, text) in lines {
            let parts: Vec<&str> = text.split_whitespace().collect();
            let numbers = parts.get(1..3).map(|n| n.iter().map(|v| v.parse::<f32>()).collect::<Result<Vec<_>, _>>());
            let (Some(sound), Some(Ok(numbers)), 3) = (parts.first(), numbers, parts.len()) else {
                problems.push(format!("[{section}] {key}: expected `sound volume pitch`, got {text:?}"));
                continue;
            };
            if !keys.contains(key) {
                problems.push(format!("[{section}] nothing named {key:?}"));
            } else if !sounds.iter().any(|s| s == sound) {
                problems.push(format!("[{section}] {key}: no sound named {sound:?}"));
            } else {
                let pick = Pick { sound: (*sound).to_owned(), volume: numbers[0].clamp(0.0, 2.0), pitch: numbers[1].clamp(0.5, 2.0) };
                picks.0.insert(key.clone(), pick);
            }
        }
        picks
    }
}

/// Sounds picked in the tuning panel to try: footsteps by surface (the full block's name), and
/// combat sounds by [`Kind`]. What isn't picked plays its own sound.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct SoundPicks {
    pub steps: Picks,
    pub combat: Picks,
}

/// The kinds of combat sound the tuning panel lets the player pick from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Swing,
    HitCreature,
    KillCreature,
    Hurt,
    KnockedOut,
    HardLand,
    Pickup,
    Blast,
}

impl Kind {
    pub const ALL: [Kind; 8] =
        [Kind::Swing, Kind::HitCreature, Kind::KillCreature, Kind::Hurt, Kind::KnockedOut, Kind::HardLand, Kind::Pickup, Kind::Blast];
    /// The body's own, listed apart from combat in the panel.
    pub const BODY: [Kind; 2] = [Kind::HardLand, Kind::Pickup];
    pub const COMBAT: [Kind; 6] = [Kind::Swing, Kind::HitCreature, Kind::KillCreature, Kind::Hurt, Kind::KnockedOut, Kind::Blast];

    /// Its name in the tuning file.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Swing => "swing",
            Kind::HitCreature => "hit_creature",
            Kind::KillCreature => "kill_creature",
            Kind::Hurt => "hurt",
            Kind::KnockedOut => "knocked_out",
            Kind::HardLand => "hard_land",
            Kind::Pickup => "pickup",
            Kind::Blast => "blast",
        }
    }

    /// The start of every sound's name (after the namespace) offered for it.
    pub fn prefix(self) -> &'static str {
        match self {
            Kind::Swing => "swing_",
            Kind::HitCreature => "hit_",
            Kind::KillCreature => "kill_",
            Kind::Hurt => "hurt_",
            Kind::KnockedOut => "knockout_",
            Kind::HardLand => "hardland_",
            Kind::Pickup => "pickup_",
            Kind::Blast => "blast_",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Swing => "swing",
            Kind::HitCreature => "hit creature",
            Kind::KillCreature => "kill creature",
            Kind::Hurt => "hit player",
            Kind::KnockedOut => "knock out",
            Kind::HardLand => "hard land",
            Kind::Pickup => "pick up",
            Kind::Blast => "crystal blast",
        }
    }

    /// The sound it plays when nothing is picked, from the first tool, the first creature and
    /// the player.
    pub fn own(self, defs: &Defs) -> Option<String> {
        let tool = defs.tools.first().map(|t| &t.sounds);
        let creature = defs.targets.first().map(|t| &t.sounds);
        let me = defs.player.as_ref().map(|p| &p.sounds);
        match self {
            Kind::Swing => tool.and_then(|s| s.swing.clone()),
            Kind::HitCreature => tool.and_then(|s| s.hit.clone()),
            Kind::KillCreature => creature.and_then(|s| s.death.clone()),
            Kind::Hurt => me.and_then(|s| s.hurt.clone()),
            Kind::KnockedOut => me.and_then(|s| s.knocked_out.clone()),
            Kind::HardLand => me.and_then(|s| s.hard_land.clone()),
            Kind::Pickup => me.and_then(|s| s.pickup.clone()),
            // The lens's own breaking sound, which differs by lens.
            Kind::Blast => None,
        }
    }
}

impl SoundPicks {
    /// The cue for a combat sound of `kind`: the pick if there is one, else `own`.
    fn combat(&self, kind: Kind, own: Option<&String>, at: Option<WorldPos>, group: Group) -> Option<Cue> {
        match self.combat.0.get(kind.key()) {
            Some(pick) => Some(pick.cue(at, group)),
            None => own.map(|sound| Cue::new(sound, at, group)),
        }
    }

    /// The step sound of a block, through its surface's pick if there is one.
    fn step(&self, blocks: &BlockRegistry, block: sim::BlockId, at: Option<WorldPos>, group: Group) -> Option<Cue> {
        let def = blocks.get(surface(blocks, block))?;
        match self.steps.0.get(&def.name) {
            Some(pick) => Some(pick.cue(at, group)),
            None => def.sounds.step.as_ref().map(|step| Cue::new(step, at, group)),
        }
    }
}

/// The full block a block is a shape of, so a slab steps like its material's block. Grass and
/// such, which can't be shaped, are their own.
pub fn surface(blocks: &BlockRegistry, id: sim::BlockId) -> sim::BlockId {
    if blocks.get(id).is_none_or(|def| def.is_full()) {
        return id;
    }
    let Some(drop) = blocks.drop(id) else {
        return id;
    };
    blocks.material_block(blocks.part_of(drop.material).unwrap_or(drop.material))
}

/// The footstep for feet at `feet`: the first block just under them that has one (or a pick for
/// its surface), else the player's own.
pub fn step_sound(blocks: &BlockRegistry, grid: &Grid, feet: WorldPos, me: &PlayerSounds, picks: &SoundPicks) -> Option<Cue> {
    let under = feet.translate(ZVec3::NEG_Z * 0.05).boxel();
    grid.cell(under)
        .iter()
        .find_map(|o| picks.step(blocks, o.block, None, Group::Movement))
        .or_else(|| me.step.as_ref().map(|step| Cue::new(step, None, Group::Movement)))
}

/// A landing: the footstep of the surface landed on (`surface`), louder and deeper the harder
/// it was, the body's own landing sound if it has one, and `hard` (the shock absorber's clunk)
/// on top when it bottoms out.
pub fn landing(tuning: &SoundTuning, me: &PlayerSounds, surface: Option<Cue>, hard: Option<Cue>, speed: f32, hard_speed: f32) -> Vec<Cue> {
    let mut cues = Vec::new();
    let loud = ((speed - tuning.land_quiet) / (tuning.land_full - tuning.land_quiet).max(1e-3)).clamp(0.0, 1.0);
    if loud > 0.0 {
        if let Some(step) = surface {
            let pitch = step.pitch * (1.0 - tuning.land_pitch_drop * loud).max(0.1);
            cues.push(Cue { volume: step.volume * tuning.land_loudness * loud, pitch, ..step });
        }
        if let Some(land) = &me.land {
            cues.push(Cue::new(land, None, Group::Movement).volume(loud));
        }
    }
    if speed >= hard_speed {
        cues.extend(hard);
    }
    cues
}

/// The cues an event makes.
fn event_cues(event: &Event, tuning: &SoundTuning, frame: &Frame, me: &PlayerSounds, cues: &mut Vec<Cue>) {
    let picks = frame.picks;
    let target_sounds = |kind: sim::TargetKind| frame.defs.targets.get(kind.index()).map(|d| &d.sounds);
    match event {
        Event::Strike(strike) => {
            // Every swing whooshes as it strikes, hit or miss.
            let tool = frame.defs.tools.get(strike.tool.index());
            cues.extend(picks.combat(Kind::Swing, tool.and_then(|t| t.sounds.swing.as_ref()), None, Group::Combat));
            let Some(hit) = &strike.hit else {
                // A swing a block cut short hits the block, which sounds like stepping on it, hard.
                if let Some(tool) = tool
                    && strike.reach < tool.reach - 1e-3
                {
                    let point = strike.eye.translate(strike.look * strike.reach);
                    let inside = point.translate(strike.look * 0.02).boxel();
                    let thud = frame.grid.cell(inside).iter().find_map(|o| picks.step(frame.blocks, o.block, Some(point), Group::Combat));
                    cues.extend(thud.map(|c| Cue { volume: c.volume * tuning.block_hit_volume, ..c }));
                }
                return;
            };
            cues.extend(picks.combat(Kind::HitCreature, tool.and_then(|t| t.sounds.hit.as_ref()), Some(hit.point), Group::Combat));
            let def = frame.defs.targets.get(hit.kind.index());
            let crit = hit.zone.and_then(|z| def.and_then(|d| d.hit_boxes.get(z))).is_some_and(|z| z.damage > 1.0);
            let pitch = if hit.broke { 0.7 } else if crit { frame.crit_pitch } else { 1.0 };
            if let Some(voice) = def.and_then(|d| d.sounds.hit.as_ref()) {
                cues.push(Cue::new(voice, Some(hit.pos), Group::Creatures).pitch(pitch));
            }
            if hit.broke {
                let death = def.and_then(|d| d.sounds.death.as_ref());
                cues.extend(picks.combat(Kind::KillCreature, death, Some(hit.pos), Group::Creatures));
            }
        }
        Event::Slam(slam) => {
            let sounds = target_sounds(slam.kind);
            if let Some(sound) = sounds.and_then(|s| s.slam.as_ref()) {
                cues.push(Cue::new(sound, Some(slam.pos), Group::Combat));
            }
            if let Some(voice) = sounds.and_then(|s| s.hit.as_ref()) {
                cues.push(Cue::new(voice, Some(slam.pos), Group::Creatures).pitch(0.8));
            }
            if slam.broke {
                let death = sounds.and_then(|s| s.death.as_ref());
                cues.extend(picks.combat(Kind::KillCreature, death, Some(slam.pos), Group::Creatures));
            }
        }
        Event::PlayerHurt(hurt) => {
            if hurt.guarded {
                cues.extend(me.guard.as_ref().map(|guard| Cue::new(guard, None, Group::Combat)));
            } else {
                cues.extend(picks.combat(Kind::Hurt, me.hurt.as_ref(), None, Group::Combat));
            }
            if hurt.died {
                cues.extend(picks.combat(Kind::KnockedOut, me.knocked_out.as_ref(), None, Group::Robot));
            }
            if let Some((id, kind)) = hurt.attacker
                && let Some(voice) = target_sounds(kind).and_then(|s| s.hit.as_ref())
            {
                let at = frame.targets.iter().find(|t| t.id == id).map(|t| t.pos);
                cues.push(Cue::new(voice, at, Group::Creatures).pitch(0.75));
            }
        }
        Event::BlockEdit(edit) => {
            let Some(def) = frame.blocks.get(edit.block) else {
                return;
            };
            let sound = if edit.placed { &def.sounds.placed } else { &def.sounds.broken };
            if let Some(sound) = sound {
                let middle = WorldPos::at_boxel(edit.pos).translate(ZVec3::splat(0.5));
                cues.push(Cue::new(sound, Some(middle), Group::Blocks));
            }
        }
        Event::Blast(blast) => {
            let own = frame.blocks.get(blast.lens).and_then(|d| d.sounds.broken.as_ref());
            cues.extend(picks.combat(Kind::Blast, own, Some(blast.from), Group::Combat));
        }
        Event::Craft(result) => {
            let sound = if result.is_ok() { &me.craft } else { &me.craft_refused };
            if let Some(sound) = sound {
                cues.push(Cue::new(sound, None, Group::Robot));
            }
        }
        // Only a refusal makes a sound: edits are many small clicks.
        Event::Blueprint(Err(_)) => {
            if let Some(sound) = &me.craft_refused {
                cues.push(Cue::new(sound, None, Group::Robot));
            }
        }
        Event::Blueprint(Ok(_)) => {}
    }
}

/// A gain as decibels, silent at 0.
fn decibels(gain: f32) -> Decibels {
    if gain <= 0.001 { Decibels::SILENCE } else { Decibels(20.0 * gain.log10()) }
}

/// A number from 0 to 1 from a seed.
fn unit(seed: u64) -> f32 {
    let mut x = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((x ^ (x >> 31)) >> 40) as f32 / (1u64 << 24) as f32
}

/// A sound the tuning panel asked to hear, played once in the middle of the head: the sound,
/// and a gain and speed on its own, as a pick plays it in the game.
#[derive(Resource, Default)]
pub struct SoundPreview(pub Option<Pick>);

impl Pick {
    /// A sound as it is, with no change of its own volume or pitch.
    pub fn plain(sound: impl Into<String>) -> Self {
        Self { sound: sound.into(), volume: 1.0, pitch: 1.0 }
    }
}

/// The gain and playback rate a sound of `volume` and `pitch` plays at with `gain` and
/// `speed` on top, the same in a preview as in the game.
fn played(volume: f32, pitch: f32, gain: f32, speed: f32) -> (f32, f64) {
    (volume * gain, (pitch * speed).max(0.05) as f64)
}

/// A registered sound, decoded.
struct Loaded {
    variations: Vec<StaticSoundData>,
    volume: f32,
    pitch: f32,
    pitch_spread: f32,
}

/// A loop that's always playing, with its sound's own volume and pitch.
struct Looping {
    handle: StaticSoundHandle,
    volume: f32,
    pitch: f32,
}

/// The mixer and every sound, decoded. Absent when there's no audio device; the game plays on
/// in silence.
pub struct Audio {
    manager: AudioManager<DefaultBackend>,
    listener: ListenerHandle,
    /// By [`Group::index`].
    groups: Vec<TrackHandle>,
    sounds: HashMap<String, Loaded>,
    servo: Option<Looping>,
    wind: Option<Looping>,
    /// Picks variations and pitch wobble.
    plays: u64,
    /// The values last applied to the tracks.
    applied: Option<SoundTuning>,
}

impl Audio {
    fn new(world: &ClientWorld) -> Result<Self, String> {
        let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()).map_err(|e| e.to_string())?;
        let listener = manager.add_listener(Vec3::ZERO, Quat::IDENTITY).map_err(|e| e.to_string())?;
        let mut groups = Vec::new();
        for _ in Group::ALL {
            groups.push(manager.add_sub_track(TrackBuilder::new().sub_track_capacity(512)).map_err(|e| e.to_string())?);
        }
        let mut audio = Self { manager, listener, groups, sounds: HashMap::new(), servo: None, wind: None, plays: 0, applied: None };
        audio.load(world);
        Ok(audio)
    }

    /// Decodes every sound in `world` and starts the servo and the wind, replacing what was there.
    fn load(&mut self, world: &ClientWorld) {
        self.sounds.clear();
        for def in world.sounds() {
            let mut variations = Vec::new();
            for (i, ogg) in def.variations.iter().enumerate() {
                match StaticSoundData::from_cursor(Cursor::new(ogg.clone())) {
                    Ok(data) => variations.push(if def.looping { data.loop_region(..) } else { data }),
                    Err(e) => warn!("sound {} variation {}: {e}", def.name, i + 1),
                }
            }
            if !variations.is_empty() {
                let loaded = Loaded { variations, volume: def.volume, pitch: def.pitch, pitch_spread: def.pitch_spread };
                self.sounds.insert(def.name.clone(), loaded);
            }
        }
        for old in [self.servo.take(), self.wind.take()].into_iter().flatten() {
            let mut handle = old.handle;
            handle.stop(Tween::default());
        }
        let me = world.defs().player.as_ref().map(|p| &p.sounds);
        self.servo = self.start_loop(me.and_then(|s| s.servo.as_ref()), Group::Robot);
        self.wind = self.start_loop(me.and_then(|s| s.wind.as_ref()), Group::Movement);
    }

    /// Starts a loop silent on `group`, for a [`Drive`] to bring up.
    fn start_loop(&mut self, name: Option<&String>, group: Group) -> Option<Looping> {
        let sound = self.sounds.get(name?)?;
        let (volume, pitch) = (sound.volume, sound.pitch);
        let data = sound.variations[0].volume(Decibels::SILENCE);
        let handle = self.groups[group.index()].play(data).ok()?;
        Some(Looping { handle, volume, pitch })
    }

    /// Plays one variation of a pick's sound once, with its own volume and pitch and the
    /// pick's on top.
    fn preview(&mut self, pick: &Pick) {
        let Some(sound) = self.sounds.get(&pick.sound) else {
            return;
        };
        self.plays += 1;
        let variation = &sound.variations[self.plays as usize % sound.variations.len()];
        let (gain, rate) = played(sound.volume, sound.pitch, pick.volume, pick.pitch);
        let data = variation.volume(decibels(gain)).playback_rate(rate);
        if let Err(e) = self.manager.play(data) {
            warn!("couldn't play {}: {e}", pick.sound);
        }
    }

    fn apply(&mut self, tuning: &SoundTuning) {
        if self.applied.as_ref() == Some(tuning) {
            return;
        }
        self.manager.main_track().set_volume(decibels(tuning.master), Tween::default());
        for group in Group::ALL {
            self.groups[group.index()].set_volume(decibels(tuning.group(group)), Tween::default());
        }
        self.applied = Some(tuning.clone());
    }

    /// Plays `cue`, from `at` in render space if it has a place.
    fn play(&mut self, tuning: &SoundTuning, cue: &Cue, at: Option<Vec3>) {
        let Some(sound) = self.sounds.get(&cue.sound) else {
            return;
        };
        self.plays += 1;
        let seed = self.plays.wrapping_mul(0x2545_f491_4f6c_dd1d);
        let variation = &sound.variations[(unit(seed) * sound.variations.len() as f32) as usize % sound.variations.len()];
        let wobble = 1.0 + (unit(seed ^ 0x5bd1) * 2.0 - 1.0) * sound.pitch_spread;
        let (gain, rate) = played(sound.volume, sound.pitch * wobble, cue.volume, cue.pitch);
        let data = variation.volume(decibels(gain)).playback_rate(rate);
        let track = &mut self.groups[cue.group.index()];
        let played = match at {
            None => track.play(data).map(|_| ()).map_err(|e| e.to_string()),
            Some(at) => {
                let builder = SpatialTrackBuilder::new()
                    .distances((tuning.near, tuning.far.max(tuning.near + 0.1)))
                    .spatialization_strength(tuning.spatial)
                    .persist_until_sounds_finish(true);
                track
                    .add_spatial_sub_track(&self.listener, at, builder)
                    .map_err(|e| e.to_string())
                    .and_then(|mut spatial| spatial.play(data).map(|_| ()).map_err(|e| e.to_string()))
            }
        };
        if let Err(e) = played {
            warn!("couldn't play {}: {e}", cue.sound);
        }
    }

    /// Sets the loops to their drives' loudness and pitch, easing over the frame.
    fn set_loops(&mut self, tuning: &SoundTuning, hearing: &Hearing, dt: f32) {
        let tween = Tween { duration: std::time::Duration::from_secs_f32(dt.clamp(0.0, 0.1)), ..Tween::default() };
        for (looping, drive, volume) in
            [(&mut self.servo, hearing.servo, tuning.servo_volume), (&mut self.wind, hearing.wind, tuning.wind_volume)]
        {
            if let Some(looping) = looping {
                let (gain, rate) = played(looping.volume, looping.pitch, drive.level * volume, drive.pitch);
                looping.handle.set_volume(decibels(gain), tween);
                looping.handle.set_playback_rate(rate, tween);
            }
        }
    }
}

/// Opens the audio device and decodes every sound. Without a device, the game stays silent.
pub(crate) fn setup_audio(world: &mut World) {
    let audio = match Audio::new(world.resource::<ClientWorld>()) {
        Ok(audio) => audio,
        Err(e) => {
            warn!("no sound: {e}");
            return;
        }
    };
    world.insert_non_send_resource(audio);
}

/// Hears what happened this frame and plays it, from the camera's ears.
#[allow(clippy::too_many_arguments)]
pub(crate) fn play_sounds(
    mut world: ResMut<ClientWorld>,
    tuning: Res<SoundTuning>,
    mut hearing: ResMut<Hearing>,
    feel: Res<crate::feel::CameraFeel>,
    feel_state: Res<FeelState>,
    combat: Res<crate::combat::CombatFeel>,
    look: Res<Look>,
    origin: Res<RenderOrigin>,
    time: Res<Time>,
    camera: Single<&Transform, With<PlayerCamera>>,
    mut preview: ResMut<SoundPreview>,
    picks: Res<SoundPicks>,
    mut readout: ResMut<crate::combat::HitReadout>,
    mut audio: Option<NonSendMut<Audio>>,
) {
    if let Some(reload) = world.take_sound_reload() {
        match reload {
            Ok(()) => {
                if let Some(audio) = &mut audio {
                    audio.load(&world);
                }
                readout.show("SOUNDS RELOADED".into());
            }
            Err(why) => {
                warn!("reloading sounds: {why}");
                readout.show(format!("SOUNDS NOT RELOADED: {why}"));
            }
        }
    }
    let events = world.take_heard();
    let frame = Frame {
        defs: world.defs(),
        blocks: world.blocks(),
        grid: world.universe().main(),
        player: world.player(),
        strides: feel_state.strides(),
        hard_landing_speed: crate::landing_feel(&feel, &world).hard_landing_speed,
        inventory: world.inventory(),
        picks: &picks,
        targets: world.targets(),
        events: &events,
        crit_pitch: combat.crit_pitch,
        yaw: look.yaw,
        pitch: look.pitch,
        dt: time.delta_secs(),
    };
    let cues = hearing.listen(&tuning, &frame);
    let Some(mut audio) = audio else {
        return;
    };
    audio.apply(&tuning);
    if let Some(pick) = preview.0.take() {
        audio.preview(&pick);
    }
    // Render space, Y-up: kira's listener faces -Z with +X to the right, as Bevy's camera does.
    audio.listener.set_position(camera.translation, Tween::default());
    audio.listener.set_orientation(camera.rotation, Tween::default());
    for cue in &cues {
        let at = cue.at.map(|at| coords::render_translation(at, origin.cell));
        audio.play(&tuning, cue, at);
    }
    audio.set_loops(&tuning, &hearing, time.delta_secs());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::testing::{base_sim, connect, step};
    use sim::{PlayerHurt, PlayerInput, Slam, Strike, TargetHit, ToolId};

    static NO_PICKS: std::sync::LazyLock<SoundPicks> = std::sync::LazyLock::new(SoundPicks::default);

    /// What a base world looks like this frame, with `events` heard.
    fn frame<'a>(world: &'a ClientWorld, events: &'a [Event], strides: i64) -> Frame<'a> {
        Frame {
            defs: world.defs(),
            blocks: world.blocks(),
            grid: world.universe().main(),
            player: world.player(),
            strides,
            hard_landing_speed: 20.0,
            inventory: world.inventory(),
            picks: &NO_PICKS,
            targets: world.targets(),
            events,
            crit_pitch: 1.35,
            yaw: 0.0,
            pitch: 0.0,
            dt: 1.0 / 60.0,
        }
    }

    fn names(cues: &[Cue]) -> Vec<&str> {
        cues.iter().map(|c| c.sound.as_str()).collect()
    }

    /// A base world with the player walking forward on the floor, short of the wall ahead.
    fn walking() -> (crate::world::Link, ClientWorld) {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        let (mut link, mut world) = connect(sim);
        let forward = PlayerInput { movement: Vec2::Y, ..Default::default() };
        for _ in 0..12 {
            step(&mut link, &mut world, &forward);
        }
        let body = world.player().unwrap();
        assert!(body.on_ground() && body.velocity().length() > 2.0, "walking: {:?}", body.velocity());
        (link, world)
    }

    #[test]
    fn a_footstep_for_each_stride_sounds_like_the_floor() {
        let (_link, world) = walking();
        let body = world.player().unwrap();
        let under = body.pos().translate(ZVec3::NEG_Z * 0.05).boxel();
        let floor = world.universe().main().cell(under)[0].block;
        let floor_step = world.blocks().get(floor).unwrap().sounds.step.clone().expect("the floor has steps");

        let tuning = SoundTuning::default();
        let mut hearing = Hearing::default();
        assert!(hearing.listen(&tuning, &frame(&world, &[], 0)).is_empty(), "the first frame only looks");
        assert!(hearing.listen(&tuning, &frame(&world, &[], 0)).is_empty(), "mid-stride");
        let cues = hearing.listen(&tuning, &frame(&world, &[], 1));
        assert_eq!(names(&cues), [floor_step.as_str()]);
        assert_eq!((cues[0].at, cues[0].group), (None, Group::Movement));
        assert!(hearing.listen(&tuning, &frame(&world, &[], 1)).is_empty(), "one step a stride");
    }

    #[test]
    fn steps_fall_back_to_the_players_own() {
        let world = connect({
            let mut sim = base_sim();
            sim.spawn_player().unwrap();
            sim
        })
        .1;
        let me = PlayerSounds { step: Some("t:clank".into()), ..Default::default() };
        let high = WorldPos::at_boxel(api::BoxelPos::new(0, 0, 200));
        let heard = step_sound(world.blocks(), world.universe().main(), high, &me, &SoundPicks::default());
        assert_eq!(heard.map(|c| c.sound).as_deref(), Some("t:clank"), "over air");
    }

    #[test]
    fn a_pick_changes_a_surfaces_steps_and_its_shapes_follow() {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        let (_link, world) = connect(sim);
        let feet = world.player().unwrap().pos();
        let under = feet.translate(ZVec3::NEG_Z * 0.05).boxel();
        let floor = world.blocks().get(world.universe().main().cell(under)[0].block).unwrap().clone();
        let me = PlayerSounds::default();
        let pick = Pick { sound: "base:step_cloth".into(), volume: 0.5, pitch: 1.3 };
        let steps = Picks([(floor.name.clone(), pick)].into_iter().collect());
        let picks = SoundPicks { steps: steps.clone(), ..Default::default() };
        let heard = step_sound(world.blocks(), world.universe().main(), feet, &me, &picks).unwrap();
        assert_eq!((heard.sound.as_str(), heard.volume, heard.pitch), ("base:step_cloth", 0.5, 1.3));
        let picks = steps;
        let lines = picks.to_lines();
        assert_eq!(lines, [(floor.name.clone(), "base:step_cloth 0.50 1.30".to_owned())]);
        let mut problems = Vec::new();
        let sounds = ["base:step_cloth".to_owned()];
        let surfaces = [floor.name.clone()];
        assert_eq!(Picks::from_lines("footsteps", &lines, &surfaces, &sounds, &mut problems), picks, "reads back");
        assert!(problems.is_empty(), "{problems:?}");
        let bad = [
            ("base:nowhere".to_owned(), "base:step_cloth 1 1".to_owned()),
            (floor.name.clone(), "base:step_silk 1 1".to_owned()),
            (floor.name.clone(), "base:step_cloth loud".to_owned()),
        ];
        assert_eq!(Picks::from_lines("footsteps", &bad, &surfaces, &sounds, &mut problems), Picks::default());
        assert_eq!(problems.len(), 3, "{problems:?}");

        // Every crafted shape is a surface of its material's full block.
        let blocks = world.blocks();
        let shaped: Vec<_> = blocks.iter().filter(|(id, d)| !d.is_full() && blocks.drop(*id).is_some()).collect();
        assert!(!shaped.is_empty(), "the test area is built from crafted shapes");
        for (id, _) in &shaped {
            let full = blocks.get(surface(blocks, *id)).unwrap();
            assert!(full.is_full(), "{} steps like a full block", blocks.get(*id).unwrap().name);
        }
        let grass = blocks.id(base::GRASS).unwrap();
        assert_eq!(surface(blocks, grass), grass, "grass is its own surface, not dirt's");
    }

    #[test]
    fn breaking_and_placing_sound_like_the_block() {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        let (mut link, mut world) = connect(sim);
        let down = -89f32.to_radians();
        let hit = world.target(0.0, down).expect("the floor is in reach");
        let block = world.blocks().get(world.universe().main().cell(hit.pos)[0].block).unwrap().clone();
        let tuning = SoundTuning::default();
        let mut hearing = Hearing::default();
        hearing.listen(&tuning, &frame(&world, &[], 0));
        world.take_heard();

        step(&mut link, &mut world, &PlayerInput { pitch: down, use_pressed: true, ..Default::default() });
        let events = world.take_heard();
        let cues = hearing.listen(&tuning, &frame(&world, &events, 0));
        let broke = cues.iter().find(|c| c.group == Group::Blocks).expect("a break is heard");
        assert_eq!(Some(&broke.sound), block.sounds.broken.as_ref());
        let middle = WorldPos::at_boxel(hit.pos).translate(ZVec3::splat(0.5));
        assert_eq!(broke.at, Some(middle), "from the boxel");

        // The player starts with wood.
        let wood = world.blocks().id(base::WOOD).unwrap();
        let place = PlayerInput { pitch: down, alt_pressed: true, place_block: wood, ..Default::default() };
        step(&mut link, &mut world, &place);
        let events = world.take_heard();
        let cues = hearing.listen(&tuning, &frame(&world, &events, 0));
        let wood_placed = world.blocks().get(wood).unwrap().sounds.placed.clone();
        assert!(cues.iter().any(|c| c.group == Group::Blocks && Some(&c.sound) == wood_placed.as_ref()), "{cues:?}");
    }

    #[test]
    fn a_swing_whooshes_once_as_it_strikes() {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        let (mut link, mut world) = connect(sim);
        let club = ToolId::first(1).next().unwrap();
        let whoosh = world.tool(club).sounds.swing.clone().unwrap();
        let tuning = SoundTuning::default();
        let mut hearing = Hearing::default();
        let mut heard = Vec::new();
        // Up at the sky, so it hits nothing.
        let swing = PlayerInput { tool: Some(club), use_pressed: true, use_held: true, pitch: 1.4, ..Default::default() };
        for i in 0..40 {
            let input = if i == 0 { swing.clone() } else { PlayerInput { tool: Some(club), pitch: 1.4, ..Default::default() } };
            step(&mut link, &mut world, &input);
            let events = world.take_heard();
            heard.extend(hearing.listen(&tuning, &frame(&world, &events, 0)));
        }
        assert_eq!(names(&heard), [whoosh.as_str()], "a miss whooshes, and only once");
    }

    #[test]
    fn hits_slams_and_blows_are_heard_from_where_they_land() {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        let (_link, world) = connect(sim);
        let cow = world.targets()[0].clone();
        let def = world.target_def(cow.kind).clone();
        let head = def.hit_boxes.iter().position(|b| b.damage > 1.0).expect("the cow has a head");
        let club = ToolId::first(1).next().unwrap();
        let point = cow.pos.translate(ZVec3::Z);
        let hit = TargetHit {
            target: cow.id,
            kind: cow.kind,
            zone: Some(head),
            point,
            push: ZVec3::ZERO,
            damage: 10.0,
            carried: 0.0,
            broke: false,
            pos: cow.pos,
        };
        let strike = Strike { tick: 1, tool: club, eye: point, look: ZVec3::Y, reach: 3.0, hit: Some(hit), recoil: ZVec3::ZERO };
        let slam = Slam { target: cow.id, kind: cow.kind, pos: cow.pos, into: ZVec3::X, speed: 8.0, damage: 2.0, broke: false };
        let hurt = |guarded| PlayerHurt {
            tick: 1,
            attacker: Some((cow.id, cow.kind)),
            from: ZVec3::X,
            damage: 2.0,
            push: ZVec3::ZERO,
            guarded,
            died: false,
        };
        let events = [Event::Strike(strike), Event::Slam(slam), Event::PlayerHurt(hurt(false)), Event::PlayerHurt(hurt(true))];
        let me = world.defs().player.as_ref().unwrap().sounds.clone();
        let mut cues = Vec::new();
        for event in &events {
            event_cues(event, &SoundTuning::default(), &frame(&world, &[], 0), &me, &mut cues);
        }
        let s = |o: &Option<String>| o.clone().unwrap();
        let moo = s(&def.sounds.hit);
        let expected = [
            (s(&world.tool(club).sounds.swing), None, 1.0),
            (s(&world.tool(club).sounds.hit), Some(point), 1.0),
            (moo.clone(), Some(cow.pos), 1.35),
            (s(&def.sounds.slam), Some(cow.pos), 1.0),
            (moo.clone(), Some(cow.pos), 0.8),
            (s(&me.hurt), None, 1.0),
            (moo.clone(), Some(cow.pos), 0.75),
            (s(&me.guard), None, 1.0),
            (moo, Some(cow.pos), 0.75),
        ];
        let got: Vec<(String, Option<WorldPos>, f32)> = cues.iter().map(|c| (c.sound.clone(), c.at, c.pitch)).collect();
        assert_eq!(got, expected, "a head hit's moo is higher, a slam's lower");
    }

    #[test]
    fn kills_knock_outs_and_swings_into_blocks_are_heard() {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        let (_link, world) = connect(sim);
        let cow = world.targets()[0].clone();
        let def = world.target_def(cow.kind).clone();
        let club = ToolId::first(1).next().unwrap();
        let tool = world.tool(club).clone();
        let me = world.defs().player.as_ref().unwrap().sounds.clone();
        let tuning = SoundTuning::default();
        let heard = |events: &[Event], picks: &SoundPicks| {
            let mut f = frame(&world, &[], 0);
            f.picks = picks;
            let mut cues = Vec::new();
            for e in events {
                event_cues(e, &tuning, &f, &me, &mut cues);
            }
            cues
        };
        let none = SoundPicks::default();

        let broke = TargetHit {
            target: cow.id,
            kind: cow.kind,
            zone: None,
            point: cow.pos,
            push: ZVec3::ZERO,
            damage: 99.0,
            carried: 0.0,
            broke: true,
            pos: cow.pos,
        };
        let kill = Event::Strike(Strike { tick: 1, tool: club, eye: cow.pos, look: ZVec3::Y, reach: tool.reach, hit: Some(broke), recoil: ZVec3::ZERO });
        let cues = heard(std::slice::from_ref(&kill), &none);
        let death = def.sounds.death.clone().unwrap();
        assert!(cues.iter().any(|c| c.sound == death && c.at == Some(cow.pos)), "{cues:?}");

        // A pick replaces the swing and the death, with its volume and pitch.
        let picked = |sound: &str| Pick { sound: sound.into(), volume: 0.5, pitch: 1.5 };
        let combat = Picks(
            [(Kind::Swing.key().to_owned(), picked("base:swing_rpg")), (Kind::KillCreature.key().to_owned(), picked("base:kill_splat"))]
                .into_iter()
                .collect(),
        );
        let picks = SoundPicks { combat, ..Default::default() };
        let cues = heard(&[kill], &picks);
        let got: Vec<(&str, f32, f32)> = cues.iter().map(|c| (c.sound.as_str(), c.volume, c.pitch)).collect();
        assert!(got.contains(&("base:swing_rpg", 0.5, 1.5)) && got.contains(&("base:kill_splat", 0.5, 1.5)), "{got:?}");
        assert!(!cues.iter().any(|c| Some(&c.sound) == tool.sounds.swing.as_ref() || c.sound == death), "replaced");

        let out = PlayerHurt { tick: 1, attacker: None, from: ZVec3::ZERO, damage: 20.0, push: ZVec3::ZERO, guarded: false, died: true };
        let cues = heard(&[Event::PlayerHurt(out)], &none);
        assert_eq!(names(&cues), [me.hurt.clone().unwrap(), me.knocked_out.clone().unwrap()]);

        // A swing a block cut short: straight down at the floor from the player's eyes.
        let eye = world.player().unwrap().pos().translate(ZVec3::Z * 1.6);
        let floor_hit = world.target(0.0, -89f32.to_radians()).unwrap();
        let reach = (eye.delta_to(WorldPos::at_boxel(floor_hit.pos)).z.abs() - 1.0).max(0.1);
        let short = Strike { tick: 1, tool: club, eye, look: ZVec3::NEG_Z, reach, hit: None, recoil: ZVec3::ZERO };
        let floor = world.universe().main().cell(floor_hit.pos)[0].block;
        let step = world.blocks().get(floor).unwrap().sounds.step.clone().unwrap();
        let cues = heard(&[Event::Strike(short)], &none);
        let thud = cues.iter().find(|c| c.sound == step).expect("the block is heard");
        assert_eq!(thud.volume, tuning.block_hit_volume);
        assert!(thud.at.is_some() && thud.group == Group::Combat);
        let miss = Strike { reach: tool.reach, ..short };
        assert!(heard(&[Event::Strike(miss)], &none).iter().all(|c| c.sound != step), "a full-length miss hits nothing");
    }

    #[test]
    fn creatures_step_once_a_stride_on_the_ground() {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        let (_link, world) = connect(sim);
        let tuning = SoundTuning::default();
        let mut cow = world.targets()[0].clone();
        cow.on_ground = true;
        cow.velocity = ZVec3::new(tuning.creature_stride * 60.0 / 10.0, 0.0, 0.0);
        let cows = [cow.clone()];
        let mut hearing = Hearing::default();
        let mut steps = 0;
        for _ in 0..30 {
            let mut f = frame(&world, &[], 0);
            f.targets = &cows;
            steps += hearing.listen(&tuning, &f).iter().filter(|c| c.group == Group::Creatures).count();
        }
        assert_eq!(steps, 3, "a stride every 10 frames");

        cow.on_ground = false;
        let flying = [cow];
        let mut f = frame(&world, &[], 0);
        f.targets = &flying;
        let mut quiet = Hearing::default();
        assert!((0..30).all(|_| quiet.listen(&tuning, &f).is_empty()), "not in the air");
    }

    #[test]
    fn more_carried_is_a_pickup_unless_it_was_crafted() {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        let (_link, world) = connect(sim);
        let tuning = SoundTuning::default();
        let pickup = world.defs().player.as_ref().unwrap().sounds.pickup.clone().unwrap();
        let stone = world.blocks().material(base::STONE).unwrap();
        let placed = world.blocks().get(world.blocks().id(base::STONE).unwrap()).unwrap().sounds.placed.clone().unwrap();
        let mut more = world.inventory().clone();
        let mut hearing = Hearing::default();
        hearing.listen(&tuning, &frame(&world, &[], 0));
        more.add(stone, 4, None);
        let mut f = frame(&world, &[], 0);
        f.inventory = &more;
        let cues = hearing.listen(&tuning, &f);
        assert_eq!(names(&cues), [pickup.as_str(), placed.as_str()], "the robot, then the material");
        assert_eq!(cues[1].volume, tuning.pickup_material);

        // Another right away waits out the gap; one after it sounds.
        let mut even_more = more.clone();
        even_more.add(stone, 4, None);
        f.inventory = &even_more;
        assert!(hearing.listen(&tuning, &f).is_empty(), "within pickup_gap");
        let mut most = even_more.clone();
        most.add(stone, 4, None);
        f.inventory = &most;
        f.dt = tuning.pickup_gap;
        assert_eq!(names(&hearing.listen(&tuning, &f))[0], pickup.as_str());

        let crafted = [Event::Craft(Ok((stone, 1)))];
        let mut crafted_more = most.clone();
        crafted_more.add(stone, 8, None);
        let mut f = frame(&world, &crafted, 0);
        f.inventory = &crafted_more;
        f.dt = 1.0;
        let craft = world.defs().player.as_ref().unwrap().sounds.craft.clone().unwrap();
        assert_eq!(names(&hearing.listen(&tuning, &f)), [craft.as_str()], "a craft, not a pickup");
    }

    #[test]
    fn a_preview_plays_a_pick_as_the_game_does() {
        // A sound of volume 0.5 and pitch 2, picked at volume 0.4 and pitch 0.75.
        assert_eq!(played(0.5, 2.0, 0.4, 0.75), (0.2, 1.5));
        assert_eq!(played(0.5, 2.0, 1.0, 1.0), (0.5, 2.0), "a plain one plays as it is");
    }

    #[test]
    fn decibels_follow_gain() {
        assert_eq!(decibels(1.0), Decibels(0.0));
        assert!((decibels(0.5).0 + 6.02).abs() < 0.01);
        assert_eq!(decibels(0.0), Decibels::SILENCE);
    }

    #[test]
    fn units_spread_over_zero_to_one() {
        let values: Vec<f32> = (0..1000).map(unit).collect();
        assert!(values.iter().all(|v| (0.0..1.0).contains(v)));
        assert!(values.iter().any(|v| *v < 0.1) && values.iter().any(|v| *v > 0.9));
    }

    #[test]
    fn every_value_is_tunable() {
        let mut t = SoundTuning::default();
        for v in t.tunables() {
            assert!(t.set_tunable(&v.name, v.min), "{} sets", v.name);
        }
        assert!(!t.set_tunable("missing", 1.0));
    }

    #[test]
    fn the_wind_is_silent_at_a_sprint_and_rises_on_a_long_fall() {
        let tuning = SoundTuning::default();
        let mut wind = Drive::default();
        for _ in 0..120 {
            wind.update(tuning.wind(), 7.5, 1.0 / 60.0);
        }
        assert_eq!(wind.level, 0.0, "sprinting is quiet");
        for _ in 0..120 {
            wind.update(tuning.wind(), (tuning.wind_start + tuning.wind_full) / 2.0, 1.0 / 60.0);
        }
        assert!((wind.level - 0.5).abs() < 0.01, "halfway between start and full: {}", wind.level);
        assert!((wind.pitch - (tuning.wind_pitch_low + tuning.wind_pitch_high) / 2.0).abs() < 1e-4);
        for _ in 0..120 {
            wind.update(tuning.wind(), 60.0, 1.0 / 60.0);
        }
        assert!(wind.level > 0.99 && (wind.pitch - tuning.wind_pitch_high).abs() < 1e-5, "a long fall roars");
    }

    #[test]
    fn a_drive_is_silent_below_its_start_and_fades_in_fast_and_out_slower() {
        let tuning = SoundTuning {
            servo_start: 4.0,
            servo_full: 14.0,
            servo_attack: 0.03,
            servo_release: 0.12,
            ..Default::default()
        };
        let mut servo = Drive::default();
        for _ in 0..60 {
            servo.update(tuning.servo(), tuning.servo_start * 0.9, 1.0 / 60.0);
        }
        assert_eq!(servo.level, 0.0, "normal aiming is silent");
        // A fast pan: most of the way up within the attack time and a bit.
        let mut t = 0.0;
        while servo.level < 0.9 {
            servo.update(tuning.servo(), tuning.servo_full * 2.0, 1.0 / 240.0);
            t += 1.0 / 240.0;
        }
        assert!(t < tuning.servo_attack * 3.0, "{t}");
        assert!((servo.pitch - tuning.servo_pitch_high).abs() < 1e-5);
        // Stopping: down to a tenth takes longer than coming up did.
        let mut fall = 0.0;
        while servo.level > 0.09 {
            servo.update(tuning.servo(), 0.0, 1.0 / 240.0);
            fall += 1.0 / 240.0;
        }
        assert!(fall > t * 2.0, "{fall} vs {t}");
        assert!(fall < tuning.servo_release * 3.0, "{fall}");
        assert!((servo.pitch - tuning.servo_pitch_high).abs() < 1e-5, "holds its pitch while fading");
    }

    #[test]
    fn landings_sound_like_the_surface_by_speed_and_clunk_when_hard() {
        let tuning = SoundTuning { land_pitch_drop: 0.2, ..Default::default() };
        let me = PlayerSounds::default();
        let step = || Some(Cue::new("t:gravel", None, Group::Movement).volume(0.5));
        let clunk = || Some(Cue::new("t:clunk", None, Group::Movement));
        let land = |speed: f32| landing(&tuning, &me, step(), clunk(), speed, 20.0);
        assert!(land(1.0).is_empty(), "a step down is silent");
        let (soft, harder) = (land(8.0), land(12.0));
        assert_eq!(names(&soft), ["t:gravel"]);
        assert!(soft[0].volume < harder[0].volume && harder[0].pitch < soft[0].pitch, "louder and deeper");
        let hard = land(25.0);
        assert_eq!(names(&hard), ["t:gravel", "t:clunk"]);
        assert_eq!(hard[0].volume, 0.5 * tuning.land_loudness, "a full landing is land_loudness footsteps");
        assert!((hard[0].pitch - 0.8).abs() < 1e-6);
        assert!(hard.iter().all(|c| c.at.is_none() && c.group == Group::Movement), "the body is heard from inside");

        let thud = PlayerSounds { land: Some("t:thud".into()), ..Default::default() };
        assert_eq!(names(&landing(&tuning, &thud, None, None, 25.0, 20.0)), ["t:thud"], "a body's own landing sound plays too");
    }
}
