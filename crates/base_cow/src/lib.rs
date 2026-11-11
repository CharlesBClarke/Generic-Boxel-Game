//! The cow's brain. A cow stands about, grazes and wanders; it stops to watch the player when
//! they come near, and runs off when hit, or turns and fights.
//!
//! Cows keep together as a herd. One that strays wanders back toward the others, one that's
//! crowded steps away, and they loosely follow a leader: the oldest cow each one can see, which
//! is the one with the lowest ID. A hit cow shows it's alarmed, and cows near an alarmed one run
//! too, so a scare spreads; once calm, a cow ignores alarms for a while, so it dies out.
//!
//! A cow that fights back squares up to the player. Close in it tosses its head: down, then flung
//! up, throwing the player up and away for a little damage. From further off it lowers its head
//! and charges in a line it can only bend a little, running well past, for big damage and
//! knockback; dodged into a wall, it's stunned for a moment. It only charges with a clear run
//! at the player, and a charge has no edge guard: baited along a cliff, it runs off. Without a
//! clear run it closes in along the path the engine finds ([`api::Route`]), round walls and
//! hopping up ledges. With no path all the way, it goes as near as it can and waits there, facing
//! the player, until its anger runs out; if it gets no closer for a while with no path at all,
//! it gives up and runs.
//!
//! A cow running away and one walking back to the herd go by path too, so walls and ledges
//! don't trap them.
//!
//! A hurt cow holds a grudge until the herd is reset, and so does one that sees a hurt herdmate
//! or one fighting the player. It goes back to its routine, but when the
//! player comes near again it fights or runs, likelier to fight the healthier it is. Run down
//! while it runs from the player, it turns and fights, however hurt.
//!
//! Each cow has a boldness, fixed by its ID. A cow that sees another in trouble (fighting, or
//! hurt and alarmed) chooses once whether to stand up for it or run: bold cows, cows seeing
//! others fight, and cows seeing one badly hurt are likelier to fight; hurt ones less. A
//! fighting cow that's badly hurt, with no other cow fighting beside it, breaks off and runs.
//!
//! Everything it remembers between ticks is in its memory (see [`Slot`]), which the engine
//! keeps, so one brain serves every cow. Trig goes through `libm`, so it decides the same
//! natively and in WebAssembly.
//!
//! The game runs this crate compiled to WebAssembly, as a component exporting the brain. It
//! also builds natively, for its tests and the `--native-brain` flag.

use std::f32::consts::{PI, TAU};

use api::{
    ApiError, Attack, Brain, BrainContext, BrainOutput, Mod, Neighbor, Registrar, Route, Seen, Tunable, Tunables, Vec2, Vec3,
};

/// The cow's target kind, defined in `mods/base/targets.toml`, as other cows see it.
pub const COW: &str = "base:cow";

/// The registered name `targets.toml` refers to.
pub const COW_BRAIN: &str = "base:cow";

#[cfg(target_arch = "wasm32")]
api::export_brain!(COW_BRAIN, CowBrain);

/// Registers the brain natively, for running without WebAssembly.
pub struct BaseCowMod;

impl Mod for BaseCowMod {
    fn name(&self) -> &str {
        "base"
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        r.register_brain(COW_BRAIN, Box::new(CowBrain::default()))
    }
}

/// What a fleeing cow shows other cows.
pub const ALARMED: &str = "alarmed";
/// What a grazing cow shows other cows.
pub const GRAZING: &str = "grazing";
/// What a fighting cow shows other cows, once its first alarm has passed.
pub const ANGRY: &str = "angry";

/// What a cow is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Standing about.
    Idle,
    /// Walking a while in one direction.
    Wander,
    /// Head down, eating.
    Graze,
    /// Standing still, turned toward the player.
    Watch,
    /// Running away from what hit it.
    Flee,
    /// Squaring up to the player: closing in, or catching its breath between attacks.
    Angry,
    /// Head down, then flung up.
    Toss,
    /// Head down, about to charge.
    ChargeUp,
    Charge,
    /// Ran into something while charging.
    Stunned,
}

impl Mode {
    const ALL: [Mode; 10] = [
        Mode::Idle,
        Mode::Wander,
        Mode::Graze,
        Mode::Watch,
        Mode::Flee,
        Mode::Angry,
        Mode::Toss,
        Mode::ChargeUp,
        Mode::Charge,
        Mode::Stunned,
    ];

    fn fighting(self) -> bool {
        matches!(self, Mode::Angry | Mode::Toss | Mode::ChargeUp | Mode::Charge | Mode::Stunned)
    }

    fn from_memory(value: f32) -> Mode {
        Self::ALL.get(value as usize).copied().unwrap_or(Mode::Idle)
    }

    fn clip(self) -> &'static str {
        match self {
            Mode::Idle | Mode::Watch | Mode::Angry => "stand",
            // Crumples to the ground, and blends back up when it's over.
            Mode::Stunned => "die",
            Mode::Wander => "walk",
            // The start of grazing puts the head down.
            Mode::Graze | Mode::ChargeUp => "eat",
            Mode::Flee | Mode::Charge => "run",
            Mode::Toss => "punch",
        }
    }
}

/// The places in a cow's memory.
#[derive(Clone, Copy)]
enum Slot {
    /// Its [`Mode`], by index.
    Mode,
    /// Seconds left in the mode, where the mode has an end.
    Left,
    /// The way it's heading while wandering or fleeing, radians counterclockwise from +X.
    Heading,
    /// Seconds left in which it ignores other cows' alarms, after a scare.
    Calm,
    /// Seconds left before it calms down from a fight, unless hit again.
    Anger,
    /// Seconds it has squared up to the player without getting any closer.
    Stuck,
    /// How near it was to the player when it last got [`PROGRESS`] closer, meters.
    Best,
    /// 1 once it has been hurt: it holds a grudge against the player.
    Grudge,
    /// Seconds the player has been within `corner_range` of it while it runs from them.
    Cornered,
    /// 1 while it fights because it was cornered: it doesn't break off for being badly hurt.
    Desperate,
    /// How far off the player was when its charge started, meters.
    Start,
}

const MEMORY: usize = 11;

/// How much closer, in meters, counts as getting closer when deciding to give up.
const PROGRESS: f32 = 0.5;
/// Seconds a cow that turns to fight takes to face the player before doing anything.
const SQUARE_UP: f32 = 0.4;
/// Seconds after turning to fight in which it shows it's alarmed, so the herd scatters.
const ALARM_FLASH: f32 = 0.5;
/// The `punch` clip lowers the head over its first half second, then flings it up over a
/// quarter.
const PUNCH_DOWN: f32 = 0.5;
const PUNCH_UP: f32 = 0.25;
/// Seconds a charge runs before a run-into-something counts, so one starting against a wall
/// doesn't stun itself straight away.
const CHARGE_GRACE: f32 = 0.15;
/// How fast the `die` clip plays for a stun: it crumples in about half a second.
const STUN_FALL: f32 = 1.5;
/// How much shorter than the distance to the player, in meters, a clear run can be and still
/// count as reaching them: a wall right behind the player stops the cow's wider body a little
/// short of their feet.
const RUN_SLACK: f32 = 0.5;
/// Where the toss and the charge hit, in the cow's own space: in front of the head.
const TOSS_BOX: ([f32; 3], [f32; 3]) = ([-0.6, 0.9, 0.0], [0.6, 2.4, 1.9]);
const CHARGE_BOX: ([f32; 3], [f32; 3]) = ([-0.6, 0.5, 0.0], [0.6, 2.1, 1.8]);

/// How much further than `notice_range` the player has to go before a watching cow loses
/// interest, so it doesn't flicker at the edge.
const LOSE_RANGE: f32 = 1.5;
/// How far ahead along its heading a running cow aims its path, meters.
const FLEE_AHEAD: f32 = 12.0;
/// Within this of the end of a path that doesn't reach the player, level in meters, it's as
/// near as it can get, and waits.
const WAIT_NEAR: f32 = 1.2;
/// How much further down than its edge guard a cow on a path lets itself step, meters, when
/// the path goes down.
const PATH_DROP_SLACK: f32 = 0.3;
/// Moving at least this fast, meters per second, along a path that reaches the player, a
/// fighting cow is getting somewhere.
const ON_ITS_WAY: f32 = 0.5;

#[derive(Clone, Debug, PartialEq)]
pub struct CowBrain {
    /// Meters per second while wandering and while fleeing.
    pub walk_speed: f32,
    pub run_speed: f32,
    /// How fast it turns, radians per second; fleeing turns twice as fast.
    pub turn_speed: f32,
    /// How near the player has to come, in meters, for it to stop and watch.
    pub notice_range: f32,
    /// Seconds it runs after a hit.
    pub flee_time: f32,
    /// Seconds it stands about between doing things: at least the first, plus up to the second.
    pub idle_min: f32,
    pub idle_spread: f32,
    /// Seconds a wander lasts, the same way.
    pub wander_min: f32,
    pub wander_spread: f32,
    /// Seconds a graze lasts: one `eat` clip is 1.5.
    pub graze_time: f32,
    /// Share of the times it stops standing about that it grazes instead of wandering.
    pub graze_chance: f32,
    /// The biggest drop it walks off, meters: just over a slab, so it walks down slab steps but
    /// stops at a full block's edge.
    pub edge_drop: f32,
    /// Speeds, in meters per second, at which the walk and run clips play at their own pace.
    /// Faster or slower plays them faster or slower, so the feet keep up with the ground.
    pub walk_pace: f32,
    pub run_pace: f32,
    /// How far from the middle of the cows it sees it can be, in meters, before it wanders back.
    pub stray_range: f32,
    /// How near another cow can be, feet to feet in meters, before a wandering cow turns away.
    pub personal_space: f32,
    /// Share of wanders that go roughly the leader's way, and of rests that turn to grazing
    /// when the leader grazes. 0 is no leader.
    pub follow: f32,
    /// How far, in radians either way, a follower's heading strays from the leader's.
    pub follow_spread: f32,
    /// How near an alarmed cow has to be, in meters, to scare it.
    pub alarm_range: f32,
    /// Seconds a cow runs when scared by another, rather than hit.
    pub spook_time: f32,
    /// Seconds after a scare in which it ignores alarms.
    pub calm_time: f32,
    /// Share of cows that fight back when hit rather than run: the boldest ones, by their IDs.
    pub fight_share: f32,
    /// How near, in meters, a cow in trouble (fighting, or hurt and alarmed) has to be for a
    /// cow to choose between standing up for it and running.
    pub defend_range: f32,
    /// The chance it stands up for one, for a cow of middling boldness: twice this for the
    /// boldest, nothing for the most timid.
    pub defend_chance: f32,
    /// Added to that chance for each cow it sees fighting.
    pub courage: f32,
    /// Added for how badly the one in trouble is hurt: all of it for one at no health.
    pub injury_pull: f32,
    /// Below this share of its health, with no other cow fighting nearby, it breaks off and runs.
    pub flee_health: f32,
    /// Seconds after the last hit before a fighting cow calms down.
    pub anger_time: f32,
    /// Seconds without getting closer to the player before it gives up.
    pub give_up_time: f32,
    /// Further off than this, level in meters, it loses interest in the player.
    pub chase_range: f32,
    /// Meters per second while closing in.
    pub approach_speed: f32,
    /// It tosses with the player's feet within this level distance of its own, in meters, and
    /// no higher than `toss_height` above them.
    pub toss_range: f32,
    pub toss_height: f32,
    /// Seconds with the head going down before the toss, and catching its breath after it.
    pub toss_windup: f32,
    pub toss_recover: f32,
    pub toss_damage: f32,
    /// The toss's knockback, meters per second: level and up.
    pub toss_push: f32,
    pub toss_lift: f32,
    /// What a facing guard lets through of the toss: shares of the damage and the knockback.
    pub toss_guarded_damage: f32,
    pub toss_guarded_push: f32,
    /// It charges with the player at least this far, level, in meters, with a clear run at them
    /// and their feet no higher or lower than `toss_height`.
    pub charge_range: f32,
    /// How far past the player a charge runs before stopping, as a share of how far off they
    /// were when it started.
    pub charge_overshoot: f32,
    /// Seconds with its head down before the charge.
    pub charge_windup: f32,
    /// Meters per second while charging.
    pub charge_speed: f32,
    /// How fast a charge bends toward the player, radians per second.
    pub charge_turn: f32,
    /// The longest a charge runs, seconds.
    pub charge_time: f32,
    pub charge_damage: f32,
    pub charge_push: f32,
    pub charge_lift: f32,
    pub charge_guarded_damage: f32,
    pub charge_guarded_push: f32,
    /// Seconds stunned after charging into something.
    pub stun_time: f32,
    /// Seconds it catches its breath after a charge.
    pub charge_recover: f32,
    /// How near, in meters, the player has to come for a cow holding a grudge to fight or run.
    pub grudge_range: f32,
    /// The chance a cow holding a grudge fights rather than runs, when whole; times its share
    /// of health when hurt.
    pub grudge_fight: f32,
    /// Running from the player with them this near, in meters, for `corner_time` seconds, it
    /// turns and fights.
    pub corner_range: f32,
    pub corner_time: f32,
}

impl Default for CowBrain {
    fn default() -> Self {
        Self {
            walk_speed: 1.5,
            run_speed: 8.0,
            turn_speed: 2.5,
            notice_range: 7.5,
            flee_time: 3.0,
            idle_min: 2.0,
            idle_spread: 4.0,
            wander_min: 2.0,
            wander_spread: 4.0,
            graze_time: 1.5,
            graze_chance: 0.5,
            edge_drop: 0.6,
            walk_pace: 1.0,
            run_pace: 2.5,
            stray_range: 5.0,
            personal_space: 2.2,
            follow: 0.7,
            follow_spread: 0.6,
            alarm_range: 6.0,
            spook_time: 5.3,
            calm_time: 4.0,
            fight_share: 1.0,
            defend_range: 8.0,
            defend_chance: 0.25,
            courage: 0.35,
            injury_pull: 0.85,
            flee_health: 0.3,
            anger_time: 29.0,
            give_up_time: 6.0,
            chase_range: 27.0,
            approach_speed: 2.6,
            toss_range: 2.4,
            toss_height: 1.2,
            toss_windup: 0.5,
            toss_recover: 0.7,
            toss_damage: 2.0,
            toss_push: 11.0,
            toss_lift: 13.5,
            toss_guarded_damage: 0.3,
            toss_guarded_push: 0.82,
            charge_range: 6.0,
            charge_overshoot: 1.5,
            charge_windup: 0.9,
            charge_speed: 15.0,
            charge_turn: 0.8,
            charge_time: 2.5,
            charge_damage: 13.0,
            charge_push: 12.0,
            charge_lift: 14.5,
            charge_guarded_damage: 0.5,
            charge_guarded_push: 0.9,
            stun_time: 2.0,
            charge_recover: 1.2,
            grudge_range: 7.5,
            grudge_fight: 0.8,
            corner_range: 3.0,
            corner_time: 1.5,
        }
    }
}

/// The tuning panel's groups for [`CowBrain`]'s values.
const COW_GROUPS: &[(&str, &[&str])] = &[
    ("Walking", &["walk_speed", "run_speed", "turn_speed", "edge_drop", "walk_pace", "run_pace"]),
    ("Idle life", &["idle_min", "idle_spread", "wander_min", "wander_spread", "graze_time", "graze_chance", "notice_range"]),
    ("Herd", &["stray_range", "personal_space", "follow", "follow_spread"]),
    ("Fear", &["flee_time", "alarm_range", "spook_time", "calm_time", "flee_health"]),
    ("Fight or run", &["fight_share", "defend_range", "defend_chance", "courage", "injury_pull"]),
    ("Chasing", &["anger_time", "give_up_time", "chase_range", "approach_speed"]),
    ("Grudges and corners", &["grudge_range", "grudge_fight", "corner_range", "corner_time"]),
    ("Head toss (up close)", &["toss_range", "toss_height", "toss_windup", "toss_recover", "toss_damage", "toss_push", "toss_lift", "toss_guarded_damage", "toss_guarded_push"]),
    ("Charge (from far off)", &["charge_range", "charge_overshoot", "charge_windup", "charge_speed", "charge_turn", "charge_time", "charge_damage", "charge_push", "charge_lift", "charge_guarded_damage", "charge_guarded_push", "stun_time", "charge_recover"]),
];
/// What each of [`CowBrain`]'s values does, for the tuning panel.
const COW_HELP: &[(&str, &str)] = &[
    ("walk_speed", "Speed while wandering, m/s."),
    ("run_speed", "Speed while fleeing, m/s."),
    ("turn_speed", "How fast it turns, radians per second; fleeing turns twice as fast."),
    ("edge_drop", "The biggest drop it walks off while wandering, meters: just over a slab walks down slab steps but stops at a full block's edge."),
    ("walk_pace", "Speed at which the walk clip plays at its own pace, m/s; faster or slower plays it faster or slower so the feet keep up with the ground."),
    ("run_pace", "Speed at which the run clip plays at its own pace, m/s."),
    ("idle_min", "Seconds it stands about between doing things, at least."),
    ("idle_spread", "Up to this many extra seconds of standing about, at random."),
    ("wander_min", "Seconds a wander lasts, at least."),
    ("wander_spread", "Up to this many extra seconds of wandering, at random."),
    ("graze_time", "Seconds a graze lasts (one eat clip is 1.5)."),
    ("graze_chance", "Share of the times it stops standing about that it grazes instead of wandering."),
    ("notice_range", "How near you have to come, meters, for it to stop and watch you."),
    ("stray_range", "How far from the middle of the cows it sees it can be, meters, before it wanders back."),
    ("personal_space", "How near another cow can be, feet to feet in meters, before a wandering cow turns away."),
    ("follow", "Share of wanders that go roughly the leader's way, and of rests that turn to grazing when the leader grazes. 0 is no leader."),
    ("follow_spread", "How far a follower's heading strays from the leader's, radians either way."),
    ("flee_time", "Seconds it runs after a hit."),
    ("alarm_range", "How near an alarmed cow has to be, meters, to scare it."),
    ("spook_time", "Seconds it runs when scared by another cow, rather than hit."),
    ("calm_time", "Seconds after a scare in which it ignores alarms."),
    ("flee_health", "Below this share of its health, with no other cow fighting nearby, it breaks off and runs."),
    ("fight_share", "Share of cows that fight back when hit rather than run: the boldest ones."),
    ("defend_range", "How near a cow in trouble (fighting, or hurt and alarmed) has to be, meters, for a cow to choose between standing up for it and running."),
    ("defend_chance", "The chance it stands up for one, for a cow of middling boldness: twice this for the boldest, nothing for the most timid."),
    ("courage", "Added to defend_chance for each cow it sees fighting."),
    ("injury_pull", "Added to defend_chance for how badly the one in trouble is hurt: all of it for one at no health."),
    ("anger_time", "Seconds after the last hit before a fighting cow calms down."),
    ("give_up_time", "Seconds without getting closer to you before it gives up the fight (and runs)."),
    ("chase_range", "Further off than this, level in meters, it loses interest in you."),
    ("approach_speed", "Speed while closing in to fight, m/s."),
    ("grudge_range", "How near you have to come, meters, for a cow holding a grudge to fight or run."),
    ("grudge_fight", "The chance a cow holding a grudge fights rather than runs, when whole; times its share of health when hurt."),
    ("corner_range", "Running from you with you this near, meters, for corner_time seconds, it turns and fights."),
    ("corner_time", "Seconds you have to stay within corner_range of a fleeing cow before it turns and fights."),
    ("toss_range", "It tosses with your feet within this level distance of its own, meters."),
    ("toss_height", "...and no higher than this above its feet, meters. Also the most a charge allows above or below."),
    ("toss_windup", "Seconds with the head going down before the toss: your window to dodge or guard."),
    ("toss_recover", "Seconds catching its breath after a toss."),
    ("toss_damage", "Health a toss takes from you."),
    ("toss_push", "The toss's level knockback, m/s."),
    ("toss_lift", "The toss's upward knockback, m/s."),
    ("toss_guarded_damage", "Share of the toss's damage a facing guard lets through."),
    ("toss_guarded_push", "Share of the toss's knockback a facing guard lets through."),
    ("charge_range", "It charges with you at least this far, level in meters, with a clear straight run at you."),
    ("charge_overshoot", "How far past you a charge runs before stopping, as a share of how far off you were when it started."),
    ("charge_windup", "Seconds with its head down before the charge."),
    ("charge_speed", "Speed while charging, m/s."),
    ("charge_turn", "How fast a charge bends toward you, radians per second."),
    ("charge_time", "The longest a charge runs, seconds."),
    ("charge_damage", "Health a charge takes from you."),
    ("charge_push", "The charge's level knockback, m/s."),
    ("charge_lift", "The charge's upward knockback, m/s."),
    ("charge_guarded_damage", "Share of the charge's damage a facing guard lets through."),
    ("charge_guarded_push", "Share of the charge's knockback a facing guard lets through."),
    ("stun_time", "Seconds stunned after charging into something (dodge a charge into a wall)."),
    ("charge_recover", "Seconds it catches its breath after a charge."),
];

impl Tunables for CowBrain {
    fn tunables(&self) -> Vec<Tunable> {
        api::tuning::described(vec![
            Tunable::new("walk_speed", self.walk_speed, 0.0, 4.0),
            Tunable::new("run_speed", self.run_speed, 0.0, 8.0),
            Tunable::new("turn_speed", self.turn_speed, 0.1, 10.0),
            Tunable::new("notice_range", self.notice_range, 0.0, 20.0),
            Tunable::new("flee_time", self.flee_time, 0.0, 10.0),
            Tunable::new("idle_min", self.idle_min, 0.0, 10.0),
            Tunable::new("idle_spread", self.idle_spread, 0.0, 10.0),
            Tunable::new("wander_min", self.wander_min, 0.0, 10.0),
            Tunable::new("wander_spread", self.wander_spread, 0.0, 10.0),
            Tunable::new("graze_time", self.graze_time, 0.0, 10.0),
            Tunable::new("graze_chance", self.graze_chance, 0.0, 1.0),
            Tunable::new("edge_drop", self.edge_drop, 0.0, 3.0),
            Tunable::new("walk_pace", self.walk_pace, 0.1, 4.0),
            Tunable::new("run_pace", self.run_pace, 0.1, 8.0),
            Tunable::new("stray_range", self.stray_range, 0.0, 20.0),
            Tunable::new("personal_space", self.personal_space, 0.0, 6.0),
            Tunable::new("follow", self.follow, 0.0, 1.0),
            Tunable::new("follow_spread", self.follow_spread, 0.0, PI),
            Tunable::new("alarm_range", self.alarm_range, 0.0, 20.0),
            Tunable::new("spook_time", self.spook_time, 0.0, 10.0),
            Tunable::new("calm_time", self.calm_time, 0.0, 20.0),
            Tunable::new("fight_share", self.fight_share, 0.0, 1.0),
            Tunable::new("defend_range", self.defend_range, 0.0, 20.0),
            Tunable::new("defend_chance", self.defend_chance, 0.0, 1.0),
            Tunable::new("courage", self.courage, 0.0, 1.0),
            Tunable::new("injury_pull", self.injury_pull, 0.0, 2.0),
            Tunable::new("flee_health", self.flee_health, 0.0, 1.0),
            Tunable::new("anger_time", self.anger_time, 0.0, 60.0),
            Tunable::new("give_up_time", self.give_up_time, 0.0, 20.0),
            Tunable::new("chase_range", self.chase_range, 0.0, 60.0),
            Tunable::new("approach_speed", self.approach_speed, 0.0, 6.0),
            Tunable::new("toss_range", self.toss_range, 0.0, 5.0),
            Tunable::new("toss_height", self.toss_height, 0.0, 3.0),
            Tunable::new("toss_windup", self.toss_windup, 0.05, 2.0),
            Tunable::new("toss_recover", self.toss_recover, 0.0, 3.0),
            Tunable::new("toss_damage", self.toss_damage, 0.0, 20.0),
            Tunable::new("toss_push", self.toss_push, 0.0, 30.0),
            Tunable::new("toss_lift", self.toss_lift, 0.0, 30.0),
            Tunable::new("toss_guarded_damage", self.toss_guarded_damage, 0.0, 1.0),
            Tunable::new("toss_guarded_push", self.toss_guarded_push, 0.0, 1.0),
            Tunable::new("charge_range", self.charge_range, 0.0, 20.0),
            Tunable::new("charge_overshoot", self.charge_overshoot, 0.0, 4.0),
            Tunable::new("charge_windup", self.charge_windup, 0.0, 3.0),
            Tunable::new("charge_speed", self.charge_speed, 0.0, 15.0),
            Tunable::new("charge_turn", self.charge_turn, 0.0, 6.0),
            Tunable::new("charge_time", self.charge_time, 0.0, 6.0),
            Tunable::new("charge_damage", self.charge_damage, 0.0, 40.0),
            Tunable::new("charge_push", self.charge_push, 0.0, 40.0),
            Tunable::new("charge_lift", self.charge_lift, 0.0, 30.0),
            Tunable::new("charge_guarded_damage", self.charge_guarded_damage, 0.0, 1.0),
            Tunable::new("charge_guarded_push", self.charge_guarded_push, 0.0, 1.0),
            Tunable::new("stun_time", self.stun_time, 0.0, 6.0),
            Tunable::new("charge_recover", self.charge_recover, 0.0, 5.0),
            Tunable::new("grudge_range", self.grudge_range, 0.0, 20.0),
            Tunable::new("grudge_fight", self.grudge_fight, 0.0, 1.0),
            Tunable::new("corner_range", self.corner_range, 0.0, 10.0),
            Tunable::new("corner_time", self.corner_time, 0.0, 10.0),
        ], COW_GROUPS, COW_HELP)
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        let field = match name {
            "walk_speed" => &mut self.walk_speed,
            "run_speed" => &mut self.run_speed,
            "turn_speed" => &mut self.turn_speed,
            "notice_range" => &mut self.notice_range,
            "flee_time" => &mut self.flee_time,
            "idle_min" => &mut self.idle_min,
            "idle_spread" => &mut self.idle_spread,
            "wander_min" => &mut self.wander_min,
            "wander_spread" => &mut self.wander_spread,
            "graze_time" => &mut self.graze_time,
            "graze_chance" => &mut self.graze_chance,
            "edge_drop" => &mut self.edge_drop,
            "walk_pace" => &mut self.walk_pace,
            "run_pace" => &mut self.run_pace,
            "stray_range" => &mut self.stray_range,
            "personal_space" => &mut self.personal_space,
            "follow" => &mut self.follow,
            "follow_spread" => &mut self.follow_spread,
            "alarm_range" => &mut self.alarm_range,
            "spook_time" => &mut self.spook_time,
            "calm_time" => &mut self.calm_time,
            "fight_share" => &mut self.fight_share,
            "defend_range" => &mut self.defend_range,
            "defend_chance" => &mut self.defend_chance,
            "courage" => &mut self.courage,
            "injury_pull" => &mut self.injury_pull,
            "flee_health" => &mut self.flee_health,
            "anger_time" => &mut self.anger_time,
            "give_up_time" => &mut self.give_up_time,
            "chase_range" => &mut self.chase_range,
            "approach_speed" => &mut self.approach_speed,
            "toss_range" => &mut self.toss_range,
            "toss_height" => &mut self.toss_height,
            "toss_windup" => &mut self.toss_windup,
            "toss_recover" => &mut self.toss_recover,
            "toss_damage" => &mut self.toss_damage,
            "toss_push" => &mut self.toss_push,
            "toss_lift" => &mut self.toss_lift,
            "toss_guarded_damage" => &mut self.toss_guarded_damage,
            "toss_guarded_push" => &mut self.toss_guarded_push,
            "charge_range" => &mut self.charge_range,
            "charge_overshoot" => &mut self.charge_overshoot,
            "charge_windup" => &mut self.charge_windup,
            "charge_speed" => &mut self.charge_speed,
            "charge_turn" => &mut self.charge_turn,
            "charge_time" => &mut self.charge_time,
            "charge_damage" => &mut self.charge_damage,
            "charge_push" => &mut self.charge_push,
            "charge_lift" => &mut self.charge_lift,
            "charge_guarded_damage" => &mut self.charge_guarded_damage,
            "charge_guarded_push" => &mut self.charge_guarded_push,
            "stun_time" => &mut self.stun_time,
            "charge_recover" => &mut self.charge_recover,
            "grudge_range" => &mut self.grudge_range,
            "grudge_fight" => &mut self.grudge_fight,
            "corner_range" => &mut self.corner_range,
            "corner_time" => &mut self.corner_time,
            _ => return false,
        };
        *field = value;
        true
    }
}

impl Brain for CowBrain {
    fn think(&self, ctx: &BrainContext) -> BrainOutput {
        let mut memory = ctx.memory.clone();
        let fresh = memory.len() != MEMORY;
        memory.resize(MEMORY, 0.0);
        let get = |m: &[f32], s: Slot| m[s as usize];
        let mut mode = Mode::from_memory(get(&memory, Slot::Mode));
        let mut left = get(&memory, Slot::Left) - ctx.dt;
        let mut heading = get(&memory, Slot::Heading);
        let mut calm = (get(&memory, Slot::Calm) - ctx.dt).max(0.0);
        let mut fight = Fight {
            anger: (get(&memory, Slot::Anger) - ctx.dt).max(0.0),
            stuck: get(&memory, Slot::Stuck),
            best: get(&memory, Slot::Best),
            desperate: get(&memory, Slot::Desperate) > 0.5,
            start: get(&memory, Slot::Start),
            gave_up: false,
        };
        let mut grudge = get(&memory, Slot::Grudge) > 0.5;
        let mut cornered = get(&memory, Slot::Cornered);
        if fresh {
            // Newly spawned: stand a random while first, so cows don't all move at once.
            mode = Mode::Idle;
            left = self.idle_min * ctx.random_unit(1) + self.idle_spread * ctx.random_unit(2);
            calm = 0.0;
            fight = Fight::calm();
            grudge = false;
            cornered = 0.0;
        }
        let herd = Herd::seen(ctx);

        let player = ctx.player.as_ref().map(|p| Vec2::new(p.offset.x, p.offset.y));
        let near = player.map(|p| p.length());
        // Hurt, or seeing a herdmate hurt or fighting: it holds it against the player. It acts on
        // a grudge from the next tick, so a cow seeing a herdmate hit still first chooses
        // between standing up for it and running.
        let held = grudge;
        grudge |= ctx.hurt.is_some() || herd.wronged();
        // Running from the player and run down, the clock runs; getting clear, it runs back.
        cornered = if mode == Mode::Flee && grudge && near.is_some_and(|d| d <= self.corner_range) {
            cornered + ctx.dt
        } else if mode == Mode::Flee {
            (cornered - ctx.dt).max(0.0)
        } else {
            0.0
        };
        let settled = calm <= 0.0 && mode != Mode::Flee && !mode.fighting();
        let alarm = herd.alarm(self.alarm_range).filter(|_| settled);
        let trouble = herd.trouble(self.defend_range).filter(|_| settled && player.is_some());
        // Badly hurt with nobody fighting beside it, it loses its nerve, unless it's cornered.
        let own = ctx.health / ctx.full_health.max(1e-6);
        let nerve = own >= self.flee_health || herd.fighters(self.defend_range) > 0 || fight.desperate;
        let away_from_player = || angle(-player.filter(|p| *p != Vec2::ZERO).unwrap_or(Vec2::X));
        if ctx.hurt.is_some() && mode.fighting() && nerve {
            fight.anger = self.anger_time;
        } else if ctx.hurt.is_some() && !mode.fighting() && player.is_some() && nerve && self.fights(ctx.id) {
            mode = Mode::Angry;
            left = SQUARE_UP;
            fight = Fight::new(self.anger_time, false);
        } else if let Some(hurt) = &ctx.hurt {
            mode = Mode::Flee;
            left = self.flee_time;
            calm = 0.0;
            // Away from the hit; straight up hits give no direction, so away from the player.
            let away = if hurt.from.truncate() != Vec2::ZERO { -hurt.from.truncate() } else { -player.unwrap_or(Vec2::X) };
            heading = angle(away);
        } else if mode == Mode::Flee && cornered >= self.corner_time {
            // Running isn't working: it turns and fights, however hurt.
            mode = Mode::Angry;
            left = SQUARE_UP;
            fight = Fight::new(self.anger_time, true);
            cornered = 0.0;
        } else if mode.fighting() && !nerve {
            // Breaks off, away from the player.
            mode = Mode::Flee;
            left = self.flee_time;
            fight = Fight::calm();
            heading = away_from_player();
        } else if trouble.is_some_and(|t| ctx.random_unit(11) < self.defend_odds(ctx, &herd, t)) {
            // Stands up for it.
            mode = Mode::Angry;
            left = SQUARE_UP;
            fight = Fight::new(self.anger_time, false);
        } else if let Some(other) = trouble.or(alarm) {
            // Run the way it runs, and away from it.
            mode = Mode::Flee;
            left = self.spook_time;
            let at = level(other.offset);
            let away = level(other.velocity).normalize_or_zero() - at.normalize_or_zero() * 0.5;
            heading = if away != Vec2::ZERO { angle(away) } else { TAU * ctx.random_unit(8) };
        } else if held && settled && near.is_some_and(|d| d <= self.grudge_range) {
            // It remembers: fights if it can get at the player and has the heart for it, or runs.
            let reachable = ctx.player.as_ref().is_some_and(|p| self.reachable(p));
            if reachable && nerve && ctx.random_unit(12) < self.grudge_fight * own {
                mode = Mode::Angry;
                left = SQUARE_UP;
                fight = Fight::new(self.anger_time, false);
            } else {
                mode = Mode::Flee;
                left = self.flee_time;
                heading = away_from_player();
            }
        } else if mode != Mode::Flee && !mode.fighting() {
            match (mode, near) {
                (Mode::Watch, Some(d)) if d <= self.notice_range + LOSE_RANGE => {}
                (Mode::Watch, _) => {
                    mode = Mode::Idle;
                    left = self.idle_time(ctx);
                }
                (_, Some(d)) if d <= self.notice_range => mode = Mode::Watch,
                _ => {}
            }
        }

        if mode.fighting() {
            if let Some(out) = self.fight(ctx, &mut mode, &mut left, &mut heading, &mut fight) {
                remember(&mut memory, Remembered { mode, left, heading, calm, fight: &fight, grudge, cornered });
                return BrainOutput { memory, ..out };
            }
            if fight.gave_up {
                // Got nowhere: runs from the player.
                mode = Mode::Flee;
                left = self.flee_time;
                heading = away_from_player();
            } else {
                // Calmed down, or lost interest.
                mode = Mode::Idle;
                left = self.idle_time(ctx);
                calm = self.calm_time;
            }
            fight = Fight::calm();
        }

        if matches!(mode, Mode::Wander | Mode::Flee) && ctx.blocked && ctx.on_ground && ctx.route.is_none() {
            // Turn away from what's in the way, by a quarter to three quarters of a turn.
            heading = ctx.yaw + PI * (0.5 + ctx.random_unit(3));
        } else if mode == Mode::Flee && grudge && near.is_some_and(|d| d <= self.grudge_range) {
            // Keeps running away from the player it remembers, while they're near; further off
            // it runs with the herd.
            heading = away_from_player();
        }
        if mode != Mode::Watch && left <= 0.0 {
            if mode == Mode::Flee {
                calm = self.calm_time;
            }
            let strayed = herd.strayed(self.stray_range);
            let crowded = herd.crowding(self.personal_space);
            let leader = herd.leader(ctx.id);
            let follows = leader.is_some() && ctx.random_unit(9) < self.follow;
            let graze = if follows && leader.is_some_and(|l| l.signal == GRAZING) {
                true
            } else {
                ctx.random_unit(4) < self.graze_chance
            };
            (mode, left) = match mode {
                Mode::Idle if strayed.is_none() && crowded.is_none() && graze => (Mode::Graze, self.graze_time),
                Mode::Idle => {
                    let led = leader.filter(|_| follows).map(|l| level(l.velocity)).filter(|v| v.length() > 0.2);
                    let jitter = self.follow_spread * (2.0 * ctx.random_unit(10) - 1.0);
                    heading = match (strayed, crowded, led) {
                        (Some(home), ..) => angle(home),
                        (None, Some(away), _) => angle(away),
                        (None, None, Some(way)) => angle(way) + jitter,
                        (None, None, None) => TAU * ctx.random_unit(5),
                    };
                    (Mode::Wander, self.wander_min + self.wander_spread * ctx.random_unit(6))
                }
                _ => (Mode::Idle, self.idle_time(ctx)),
            };
        }
        if mode == Mode::Wander && !ctx.blocked {
            // Steer while walking: back toward the herd once too far out, away from one too close.
            if let Some(home) = herd.strayed(self.stray_range) {
                heading = angle(home);
            } else if let Some(away) = herd.crowding(self.personal_space) {
                let (sin, cos) = libm::sincosf(heading);
                let blend = Vec2::new(cos, sin) + away.normalize_or_zero();
                if blend != Vec2::ZERO {
                    heading = angle(blend);
                }
            }
        }

        // Running, it heads for a point ahead; strayed, for the middle of the herd: by path.
        let go_to = match mode {
            Mode::Flee => {
                let (sin, cos) = libm::sincosf(heading);
                Some(Vec3::new(cos, sin, 0.0) * FLEE_AHEAD)
            }
            Mode::Wander => herd.strayed(self.stray_range).map(|_| herd.middle()),
            _ => None,
        };
        let route = go_to.and(ctx.route.as_ref()).filter(|r| level(r.next) != Vec2::ZERO);
        let way = route.map_or(heading, |r| angle(level(r.next)));
        let (face, speed, turn) = match mode {
            Mode::Wander => (Some(way), self.walk_speed, self.turn_speed),
            Mode::Flee => (Some(way), self.run_speed, self.turn_speed * 2.0),
            Mode::Watch => (player.filter(|p| *p != Vec2::ZERO).map(angle), 0.0, self.turn_speed),
            _ => (None, 0.0, 0.0),
        };
        let yaw = libm::remainderf(face.map_or(ctx.yaw, |f| turn_toward(ctx.yaw, f, turn * ctx.dt)), TAU);
        // It walks the way it faces, so it arcs round as it turns.
        let (sin, cos) = libm::sincosf(yaw);
        let walk = Vec2::new(cos, sin) * speed;
        let moving = Vec2::new(ctx.velocity.x, ctx.velocity.y).length();
        let clip_speed = match mode {
            Mode::Wander => moving / self.walk_pace.max(0.01),
            Mode::Flee => moving / self.run_pace.max(0.01),
            _ => 1.0,
        };
        let signal = match mode {
            Mode::Flee => ALARMED,
            Mode::Graze => GRAZING,
            _ => "",
        };

        remember(&mut memory, Remembered { mode, left, heading, calm, fight: &fight, grudge, cornered });
        BrainOutput {
            walk,
            yaw,
            edge_guard: Some(self.path_guard(route)),
            clip: mode.clip().into(),
            clip_speed,
            signal: signal.into(),
            attack: None,
            go_to,
            hop: hops(ctx, route),
            memory,
        }
    }
}

impl CowBrain {
    /// Its edge guard: the usual one, or deep enough to step down where its path goes down.
    fn path_guard(&self, route: Option<&Route>) -> f32 {
        route.map_or(self.edge_drop, |r| self.edge_drop.max(-r.next.z + PATH_DROP_SLACK))
    }

    fn idle_time(&self, ctx: &BrainContext) -> f32 {
        self.idle_min + self.idle_spread * ctx.random_unit(7)
    }

    /// Whether it could charge straight at the player: a clear run that reaches them, and their
    /// feet no higher or lower than its head toss reaches.
    fn reachable(&self, seen: &Seen) -> bool {
        seen.clear_run >= level(seen.offset).length() - RUN_SLACK && seen.offset.z.abs() <= self.toss_height
    }

    /// Whether the cow with this ID fights back when hit, rather than running.
    fn fights(&self, id: u64) -> bool {
        boldness(id) >= 1.0 - self.fight_share
    }

    /// The chance a cow stands up for `other`, a cow in trouble: higher the bolder it is, the
    /// more cows it sees fighting and the worse `other` is hurt, lower the worse it's hurt
    /// itself.
    fn defend_odds(&self, ctx: &BrainContext, herd: &Herd, other: &Neighbor) -> f32 {
        let own = ctx.health / ctx.full_health.max(1e-6);
        let odds = self.defend_chance * 2.0 * boldness(ctx.id) + self.courage * herd.fighters(self.defend_range) as f32
            + self.injury_pull * (1.0 - other.health).max(0.0)
            - (1.0 - own).max(0.0);
        odds.clamp(0.0, 1.0)
    }

    /// A tick of fighting: picks the next move when one ends, and says what the cow does.
    /// `None` once it has calmed down or given up, leaving `mode` to the caller.
    fn fight(&self, ctx: &BrainContext, mode: &mut Mode, left: &mut f32, heading: &mut f32, fight: &mut Fight) -> Option<BrainOutput> {
        let seen = ctx.player.as_ref()?;
        let to = level(seen.offset);
        let distance = to.length();
        if distance > self.chase_range {
            return None;
        }
        let rise = seen.offset.z;
        let toward = if to != Vec2::ZERO { angle(to) } else { ctx.yaw };
        let moving = Vec2::new(ctx.velocity.x, ctx.velocity.y).length();
        // Getting nowhere, the whole fight through: wind-ups, charges and stuns count too, so a
        // cow charging a fence again and again gives up. The player in reach, or a hit landing,
        // is getting somewhere.
        let in_reach = distance <= self.toss_range && rise.abs() <= self.toss_height;
        // No clear run at the player: it closes in by path, and with none all the way it waits
        // at the nearest point it can get to.
        let straight = self.reachable(seen);
        let route = ctx.route.as_ref().filter(|_| !straight && !in_reach);
        let waiting = route.is_some_and(|r| !r.reaches && level(r.end).length() < WAIT_NEAR);
        let following = route.filter(|r| !waiting && level(r.next) != Vec2::ZERO);
        // Getting closer is measured along the path when it has one that reaches, and moving
        // along such a path is getting somewhere, however far the player has gone.
        let reach = route.filter(|r| r.reaches).map_or(seen.offset.length(), |r| r.length);
        let on_its_way = following.is_some_and(|r| r.reaches) && moving > ON_ITS_WAY;
        if ctx.attack_landed || in_reach || on_its_way || reach < fight.best - PROGRESS {
            fight.best = reach;
            fight.stuck = 0.0;
        } else if !waiting {
            fight.stuck += ctx.dt;
        }
        // A move running out leads back to squaring up.
        match *mode {
            Mode::Toss if *left <= 0.0 => *mode = Mode::Angry,
            Mode::ChargeUp if *left <= 0.0 => {
                *mode = Mode::Charge;
                *left = self.charge_time;
                *heading = toward;
                fight.start = distance;
            }
            Mode::Charge => {
                let (sin, cos) = libm::sincosf(*heading);
                let passed = to.dot(Vec2::new(cos, sin)) < -self.charge_overshoot * fight.start;
                if ctx.blocked && ctx.on_ground && self.charge_time - *left > CHARGE_GRACE {
                    *mode = Mode::Stunned;
                    *left = self.stun_time;
                } else if ctx.attack_landed || passed || *left <= 0.0 {
                    *mode = Mode::Angry;
                    *left = self.charge_recover;
                }
            }
            Mode::Stunned if *left <= 0.0 => *mode = Mode::Angry,
            _ => {}
        }
        if *mode == Mode::Angry && *left <= 0.0 {
            if fight.anger <= 0.0 {
                return None;
            }
            if fight.stuck >= self.give_up_time {
                fight.gave_up = true;
                return None;
            }
            if distance <= self.toss_range && rise <= self.toss_height && rise >= -self.toss_height {
                *mode = Mode::Toss;
                *left = self.toss_windup + PUNCH_UP + self.toss_recover;
            } else if distance >= self.charge_range && self.reachable(seen) {
                *mode = Mode::ChargeUp;
                *left = self.charge_windup;
            }
        }

        let face = |turn: f32| libm::remainderf(turn_toward(ctx.yaw, toward, turn * ctx.dt), TAU);
        let (yaw, speed, clip_speed, attack) = match *mode {
            Mode::Angry if *left > 0.0 || waiting => (face(self.turn_speed * 2.0), 0.0, 1.0, None),
            Mode::Angry if following.is_some() => {
                let way = following.map_or(toward, |r| angle(level(r.next)));
                let yaw = libm::remainderf(turn_toward(ctx.yaw, way, self.turn_speed * 2.0 * ctx.dt), TAU);
                (yaw, self.approach_speed, self.gait(moving), None)
            }
            Mode::Angry => (face(self.turn_speed * 2.0), self.approach_speed, self.gait(moving), None),
            Mode::Toss => {
                let into = self.toss_windup + PUNCH_UP + self.toss_recover - *left;
                let striking = into >= self.toss_windup && into < self.toss_windup + PUNCH_UP;
                let yaw = if into < self.toss_windup { face(self.turn_speed * 2.0) } else { ctx.yaw };
                let attack = striking.then_some(Attack {
                    min: TOSS_BOX.0,
                    max: TOSS_BOX.1,
                    damage: self.toss_damage,
                    push: self.toss_push,
                    lift: self.toss_lift,
                    guarded_damage: self.toss_guarded_damage,
                    guarded_push: self.toss_guarded_push,
                });
                // The clip's head goes down over the wind-up, whatever its length.
                let clip_speed = if into < self.toss_windup { PUNCH_DOWN / self.toss_windup.max(0.01) } else { 1.0 };
                (yaw, 0.0, clip_speed, attack)
            }
            Mode::ChargeUp => (face(self.turn_speed * 2.0), 0.0, 1.0, None),
            Mode::Charge => {
                *heading = libm::remainderf(turn_toward(*heading, toward, self.charge_turn * ctx.dt), TAU);
                let attack = Attack {
                    min: CHARGE_BOX.0,
                    max: CHARGE_BOX.1,
                    damage: self.charge_damage,
                    push: self.charge_push,
                    lift: self.charge_lift,
                    guarded_damage: self.charge_guarded_damage,
                    guarded_push: self.charge_guarded_push,
                };
                (*heading, self.charge_speed, moving / self.run_pace.max(0.01), Some(attack))
            }
            Mode::Stunned => (ctx.yaw, 0.0, STUN_FALL, None),
            _ => (ctx.yaw, 0.0, 1.0, None),
        };
        let (sin, cos) = libm::sincosf(yaw);
        let clip = match *mode {
            Mode::Angry if speed > 0.0 => self.gait_clip(),
            m => m.clip(),
        };
        // Just hit: alarmed for a moment, so the herd hears of it. One that came to help is
        // angry from the start.
        let hit = ctx.health < ctx.full_health;
        let signal = if hit && self.anger_time - fight.anger < ALARM_FLASH { ALARMED } else { ANGRY };
        let walking = *mode == Mode::Angry && speed > 0.0;
        Some(BrainOutput {
            walk: Vec2::new(cos, sin) * speed,
            yaw,
            // A charge runs off edges, so it can be baited off a cliff.
            edge_guard: (*mode != Mode::Charge).then(|| self.path_guard(following)),
            clip: clip.into(),
            clip_speed,
            signal: signal.into(),
            attack,
            go_to: Some(seen.offset),
            hop: walking && hops(ctx, following),
            memory: Vec::new(),
        })
    }

    /// The clip for closing in at `approach_speed`.
    fn gait_clip(&self) -> &'static str {
        if self.approach_speed > (self.walk_speed + self.run_speed) * 0.5 { "run" } else { "walk" }
    }

    /// How fast that clip plays at `moving` meters per second.
    fn gait(&self, moving: f32) -> f32 {
        let pace = if self.gait_clip() == "run" { self.run_pace } else { self.walk_pace };
        moving / pace.max(0.01)
    }
}

/// How bold the cow with this ID is, from 0 to 1, for good: bold ones fight, timid ones run.
fn boldness(id: u64) -> f32 {
    let mut x = id.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// What a cow keeps while fighting; see [`Slot`].
struct Fight {
    anger: f32,
    stuck: f32,
    best: f32,
    desperate: bool,
    start: f32,
    /// Gave up this tick for getting nowhere; not remembered.
    gave_up: bool,
}

impl Fight {
    /// A fight just starting, with `anger` seconds of anger; `desperate` when cornered.
    fn new(anger: f32, desperate: bool) -> Self {
        Fight { anger, stuck: 0.0, best: f32::MAX, desperate, start: 0.0, gave_up: false }
    }

    /// No fight.
    fn calm() -> Self {
        Fight::new(0.0, false)
    }
}

/// Everything a cow keeps between ticks; see [`Slot`].
struct Remembered<'a> {
    mode: Mode,
    left: f32,
    heading: f32,
    calm: f32,
    fight: &'a Fight,
    grudge: bool,
    cornered: f32,
}

fn remember(memory: &mut [f32], r: Remembered<'_>) {
    let mut set = |s: Slot, v: f32| memory[s as usize] = v;
    set(Slot::Mode, r.mode as usize as f32);
    set(Slot::Left, r.left);
    set(Slot::Heading, r.heading);
    set(Slot::Calm, r.calm);
    set(Slot::Anger, r.fight.anger);
    set(Slot::Stuck, r.fight.stuck);
    set(Slot::Best, r.fight.best);
    set(Slot::Grudge, if r.grudge { 1.0 } else { 0.0 });
    set(Slot::Cornered, r.cornered);
    set(Slot::Desperate, if r.fight.desperate { 1.0 } else { 0.0 });
    set(Slot::Start, r.fight.start);
}

/// The other cows one cow sees.
struct Herd<'a> {
    cows: Vec<&'a Neighbor>,
}

impl<'a> Herd<'a> {
    fn seen(ctx: &'a BrainContext) -> Self {
        Herd { cows: ctx.others.iter().filter(|n| n.kind == COW).collect() }
    }

    /// The way back to the middle of the herd, if that's further than `range`.
    fn strayed(&self, range: f32) -> Option<Vec2> {
        if self.cows.is_empty() {
            return None;
        }
        let middle = self.cows.iter().map(|n| level(n.offset)).sum::<Vec2>() / self.cows.len() as f32;
        (middle.length() > range).then_some(middle)
    }

    /// The middle of the cows it sees, from its feet. Only for a cow that sees some.
    fn middle(&self) -> Vec3 {
        self.cows.iter().map(|n| n.offset).sum::<Vec3>() / self.cows.len().max(1) as f32
    }

    /// The way away from the nearest cow, if it's nearer than `space`.
    fn crowding(&self, space: f32) -> Option<Vec2> {
        let nearest = self.cows.first()?;
        let at = level(nearest.offset);
        (at.length() < space).then(|| if at != Vec2::ZERO { -at } else { Vec2::X })
    }

    /// The leader: the oldest cow it sees, unless it's older than all of them.
    fn leader(&self, id: u64) -> Option<&'a Neighbor> {
        self.cows.iter().copied().filter(|n| n.id < id).min_by_key(|n| n.id)
    }

    /// The nearest cow in trouble within `range`: fighting, or hurt and alarmed.
    fn trouble(&self, range: f32) -> Option<&'a Neighbor> {
        self.cows
            .iter()
            .copied()
            .find(|n| (n.signal == ANGRY || (n.signal == ALARMED && n.health < 1.0)) && level(n.offset).length() <= range)
    }

    /// Whether it sees a cow the player has wronged: hurt, or fighting them.
    fn wronged(&self) -> bool {
        self.cows.iter().any(|n| n.health < 1.0 || n.signal == ANGRY)
    }

    /// How many cows within `range` are fighting.
    fn fighters(&self, range: f32) -> usize {
        self.cows.iter().filter(|n| n.signal == ANGRY && level(n.offset).length() <= range).count()
    }

    /// The nearest alarmed cow within `range`.
    fn alarm(&self, range: f32) -> Option<&'a Neighbor> {
        self.cows.iter().copied().find(|n| n.signal == ALARMED && level(n.offset).length() <= range)
    }
}

/// Whether a cow following `route` hops now: the route says so, and it's up against the ledge.
fn hops(ctx: &BrainContext, route: Option<&Route>) -> bool {
    route.is_some_and(|r| r.hop) && ctx.blocked && ctx.on_ground
}

fn level(v: api::Vec3) -> Vec2 {
    Vec2::new(v.x, v.y)
}

/// The direction of `v`, radians counterclockwise from +X.
fn angle(v: Vec2) -> f32 {
    libm::atan2f(v.y, v.x)
}

/// `from` turned toward `to` by at most `step` radians, the short way round.
fn turn_toward(from: f32, to: f32, step: f32) -> f32 {
    let diff = libm::remainderf(to - from, TAU);
    if diff.abs() <= step { from + diff } else { from + step * diff.signum() }
}

#[cfg(test)]
mod tests {
    use api::Hurt;

    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn ctx() -> BrainContext {
        BrainContext {
            dt: DT,
            id: 10,
            velocity: Vec3::ZERO,
            yaw: 0.0,
            on_ground: true,
            blocked: false,
            health: 20.0,
            full_health: 20.0,
            player: None,
            hurt: None,
            attack_landed: false,
            others: Vec::new(),
            random: 1,
            route: None,
            memory: Vec::new(),
        }
    }

    /// A cow in `mode` with `left` seconds of it and the given heading.
    fn remembering(mode: Mode, left: f32, heading: f32) -> BrainContext {
        BrainContext { memory: memory(mode, left, heading, 0.0), ..ctx() }
    }

    /// A full memory: `mode` with `left` seconds of it, a heading and `calm` seconds of calm.
    fn memory(mode: Mode, left: f32, heading: f32, calm: f32) -> Vec<f32> {
        let mut m = vec![0.0; MEMORY];
        m[Slot::Mode as usize] = mode as usize as f32;
        m[Slot::Left as usize] = left;
        m[Slot::Heading as usize] = heading;
        m[Slot::Calm as usize] = calm;
        m
    }

    fn mode(out: &BrainOutput) -> Mode {
        Mode::from_memory(out.memory[Slot::Mode as usize])
    }

    /// Runs `seconds` of ticks from `start`, feeding memory, facing and the walk back in as the
    /// engine would. Returns every output.
    fn run(brain: &CowBrain, start: BrainContext, seconds: f32) -> Vec<BrainOutput> {
        let mut c = start;
        let mut outs = Vec::new();
        for tick in 0..(seconds / DT) as u64 {
            let out = brain.think(&c);
            c = BrainContext {
                memory: out.memory.clone(),
                yaw: out.yaw,
                velocity: out.walk.extend(0.0),
                random: c.random.wrapping_add(tick * 7919),
                hurt: None,
                ..c
            };
            outs.push(out);
        }
        outs
    }

    #[test]
    fn a_new_cow_stands_about_then_grazes_or_wanders() {
        let brain = CowBrain::default();
        let outs = run(&brain, ctx(), 30.0);
        assert_eq!(mode(&outs[0]), Mode::Idle);
        assert_eq!(outs[0].clip, "stand");
        assert!(outs.iter().any(|o| mode(o) == Mode::Graze && o.clip == "eat"), "grazes in 30 s");
        assert!(outs.iter().any(|o| mode(o) == Mode::Wander && o.clip == "walk"), "wanders in 30 s");
        for o in &outs {
            let speed = o.walk.length();
            match mode(o) {
                Mode::Wander => assert!((speed - brain.walk_speed).abs() < 1e-4),
                _ => assert_eq!(speed, 0.0, "only walks when wandering"),
            }
            assert_eq!(o.edge_guard, Some(brain.edge_drop));
        }
    }

    #[test]
    fn cows_with_different_random_numbers_act_differently() {
        let brain = CowBrain::default();
        let modes = |random| run(&brain, BrainContext { random, ..ctx() }, 20.0).iter().map(mode).collect::<Vec<_>>();
        assert_ne!(modes(1), modes(2));
        assert_eq!(modes(1), modes(1), "and the same ones alike");
    }

    #[test]
    fn a_wandering_cow_turns_away_from_a_wall() {
        let brain = CowBrain::default();
        let c = BrainContext { blocked: true, ..remembering(Mode::Wander, 5.0, 0.0) };
        let out = brain.think(&c);
        let heading = out.memory[Slot::Heading as usize];
        let back = libm::remainderf(heading - 0.0, TAU).abs();
        assert!(back >= PI * 0.5 - 1e-4, "a quarter turn or more: {heading}");
        assert_eq!(mode(&out), Mode::Wander);
    }

    #[test]
    fn it_watches_the_player_in_range_and_ignores_them_out_of_range() {
        let brain = CowBrain::default();
        let north = |d: f32| Some(seen(Vec3::new(0.0, d, 0.0)));
        let out = brain.think(&BrainContext { player: north(brain.notice_range - 0.5), ..remembering(Mode::Wander, 5.0, 0.0) });
        assert_eq!(mode(&out), Mode::Watch);
        assert_eq!((out.walk, out.clip.as_str()), (Vec2::ZERO, "stand"));
        assert!(out.yaw > 0.0, "turning north, toward them");
        // Facing them in the end.
        let outs = run(&brain, BrainContext { player: north(3.0), ..ctx() }, 2.0);
        assert!((outs.last().unwrap().yaw - PI / 2.0).abs() < 1e-3);
        // Out of range it carries on.
        let out = brain.think(&BrainContext { player: north(brain.notice_range + 0.5), ..remembering(Mode::Wander, 5.0, 0.0) });
        assert_eq!(mode(&out), Mode::Wander);
        // A watching cow keeps watching a little past the range, then loses interest.
        let watching = remembering(Mode::Watch, 0.0, 0.0);
        let out = brain.think(&BrainContext { player: north(brain.notice_range + 1.0), ..watching.clone() });
        assert_eq!(mode(&out), Mode::Watch);
        let out = brain.think(&BrainContext { player: north(brain.notice_range + LOSE_RANGE + 0.5), ..watching });
        assert_eq!(mode(&out), Mode::Idle);
    }

    #[test]
    fn a_hit_cow_runs_away_from_the_hit_for_the_flee_time_then_calms_down() {
        let brain = CowBrain { fight_share: 0.0, ..CowBrain::default() };
        // Hit from the west, while watching the player there, out of cornering range.
        let player = Some(seen(Vec3::new(-brain.corner_range - 1.0, 0.0, 0.0)));
        let hit = BrainContext {
            hurt: Some(Hurt { from: -Vec3::X, damage: 5.0 }),
            player: player.clone(),
            yaw: PI,
            ..remembering(Mode::Watch, 0.0, 0.0)
        };
        let out = brain.think(&hit);
        assert_eq!((mode(&out), out.clip.as_str()), (Mode::Flee, "run"));
        assert!(libm::remainderf(out.memory[Slot::Heading as usize], TAU).abs() < 1e-4, "heading east, away");
        // Fleeing ignores the player nearby, turns east and runs at run speed.
        let outs = run(&brain, BrainContext { hurt: None, memory: out.memory, yaw: out.yaw, ..hit }, brain.flee_time + 0.5);
        let fled = (brain.flee_time / DT) as usize - 2;
        assert!(outs[..fled].iter().all(|o| mode(o) == Mode::Flee));
        let running = &outs[fled - 1];
        assert!((running.walk - Vec2::X * brain.run_speed).length() < 1e-3, "{}", running.walk);
        assert_ne!(mode(outs.last().unwrap()), Mode::Flee, "calmed down");
    }

    #[test]
    fn the_walk_clip_keeps_pace_with_the_ground() {
        let brain = CowBrain::default();
        let c = |speed: f32| BrainContext { velocity: Vec3::X * speed, ..remembering(Mode::Wander, 5.0, 0.0) };
        assert!((brain.think(&c(brain.walk_pace)).clip_speed - 1.0).abs() < 1e-5);
        assert!((brain.think(&c(brain.walk_pace * 2.0)).clip_speed - 2.0).abs() < 1e-5);
        assert_eq!(brain.think(&c(0.0)).clip_speed, 0.0, "stopped against something, the legs stop");
    }

    #[test]
    fn turning_takes_the_short_way_and_stops_on_target() {
        assert!((turn_toward(0.1, -0.1, 1.0) + 0.1).abs() < 1e-6);
        assert!((turn_toward(3.0, -3.0, 0.1) - 3.1).abs() < 1e-6, "across the back, not the front");
        assert!((turn_toward(0.0, 1.0, 0.25) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn every_value_is_tunable() {
        let mut b = CowBrain::default();
        for t in b.tunables() {
            assert!(b.set_tunable(&t.name, t.min), "{}", t.name);
        }
        assert!(!b.set_tunable("missing", 1.0));
    }

    fn cow(x: f32, y: f32, id: u64, signal: &str) -> Neighbor {
        Neighbor { offset: Vec3::new(x, y, 0.0), velocity: Vec3::ZERO, kind: COW.into(), id, signal: signal.into(), health: 1.0 }
    }

    /// Nearest first, as the engine gives them.
    fn seeing(mut others: Vec<Neighbor>, c: BrainContext) -> BrainContext {
        others.sort_by(|a, b| a.offset.length().total_cmp(&b.offset.length()));
        BrainContext { others, ..c }
    }

    fn heading(out: &BrainOutput) -> f32 {
        out.memory[Slot::Heading as usize]
    }

    fn near_angle(a: f32, b: f32, within: f32) -> bool {
        libm::remainderf(a - b, TAU).abs() <= within
    }

    /// A cow whose rest is just up, with no wish to graze.
    fn rested(brain: &CowBrain, others: Vec<Neighbor>) -> BrainOutput {
        brain.think(&seeing(others, remembering(Mode::Idle, 0.0, 0.0)))
    }

    #[test]
    fn a_straying_cow_wanders_back_to_the_middle_of_the_herd() {
        let brain = CowBrain { graze_chance: 1.0, follow: 0.0, ..CowBrain::default() };
        // The others are all well to the north.
        let herd = vec![cow(-1.0, 8.0, 1, ""), cow(1.0, 9.0, 2, ""), cow(0.0, 10.0, 3, "")];
        let out = rested(&brain, herd.clone());
        assert_eq!(mode(&out), Mode::Wander, "not grazing while strayed");
        assert!(near_angle(heading(&out), PI / 2.0, 0.2), "north: {}", heading(&out));
        // Already wandering east, it turns for home.
        let out = brain.think(&seeing(herd, remembering(Mode::Wander, 5.0, 0.0)));
        assert!(near_angle(heading(&out), PI / 2.0, 0.2), "north: {}", heading(&out));
        // Near enough, it grazes as it likes.
        let close = vec![cow(-1.0, 3.0, 1, ""), cow(1.0, 3.0, 2, ""), cow(0.0, 4.0, 3, "")];
        assert_eq!(mode(&rested(&brain, close)), Mode::Graze);
    }

    #[test]
    fn a_crowded_cow_steps_away_from_the_nearest() {
        let brain = CowBrain { graze_chance: 1.0, follow: 0.0, ..CowBrain::default() };
        let out = rested(&brain, vec![cow(1.0, 0.0, 1, ""), cow(3.0, 0.0, 2, "")]);
        assert_eq!(mode(&out), Mode::Wander);
        assert!(near_angle(heading(&out), PI, 0.01), "west, away from it: {}", heading(&out));
        // Wandering toward it, it veers off.
        let out = brain.think(&seeing(vec![cow(1.0, 0.5, 1, "")], remembering(Mode::Wander, 5.0, 0.0)));
        assert!(heading(&out) < -0.3, "turned south, away: {}", heading(&out));
    }

    #[test]
    fn followers_wander_the_leaders_way_and_graze_when_it_grazes() {
        let brain = CowBrain { graze_chance: 0.0, follow: 1.0, follow_spread: 0.0, ..CowBrain::default() };
        let mut leader = cow(3.0, 0.0, 1, "");
        leader.velocity = Vec3::new(0.0, -1.0, 0.0);
        let other = cow(-3.0, 0.0, 4, "");
        let out = rested(&brain, vec![leader.clone(), other.clone()]);
        assert_eq!(mode(&out), Mode::Wander);
        assert!(near_angle(heading(&out), -PI / 2.0, 1e-4), "south with the leader: {}", heading(&out));
        // The leader is the oldest it sees, not the nearest.
        let mut young = cow(2.5, 0.0, 5, "");
        young.velocity = Vec3::X;
        let out = rested(&brain, vec![young, leader.clone(), other.clone()]);
        assert!(near_angle(heading(&out), -PI / 2.0, 1e-4));
        // The leader grazing, it grazes too, however little it likes to.
        let grazing = Neighbor { signal: GRAZING.into(), velocity: Vec3::ZERO, ..leader };
        assert_eq!(mode(&rested(&brain, vec![grazing.clone(), other.clone()])), Mode::Graze);
        // A cow older than all it sees leads itself.
        let elder = BrainContext { id: 0, ..remembering(Mode::Idle, 0.0, 0.0) };
        assert_eq!(mode(&brain.think(&seeing(vec![grazing, other], elder))), Mode::Wander);
    }

    #[test]
    fn an_alarmed_cow_nearby_sets_it_running_and_it_passes_the_alarm_on() {
        let brain = CowBrain::default();
        let mut scared = cow(2.0, 0.0, 1, ALARMED);
        scared.velocity = Vec3::new(0.0, 3.0, 0.0);
        let out = brain.think(&seeing(vec![scared.clone()], remembering(Mode::Graze, 1.0, 0.0)));
        assert_eq!((mode(&out), out.clip.as_str(), out.signal.as_str()), (Mode::Flee, "run", ALARMED));
        assert!((out.memory[Slot::Left as usize] - brain.spook_time).abs() < 1e-4, "for the spook time");
        let h = heading(&out);
        assert!(h > PI / 2.0 && h < PI, "north with it, and west away from it: {h}");
        // Too far, or another kind, and it carries on.
        let far = Neighbor { offset: Vec3::new(brain.alarm_range + 1.0, 0.0, 0.0), ..scared.clone() };
        assert_eq!(mode(&brain.think(&seeing(vec![far], remembering(Mode::Graze, 1.0, 0.0)))), Mode::Graze);
        let pig = Neighbor { kind: "other:pig".into(), ..scared.clone() };
        assert_eq!(mode(&brain.think(&seeing(vec![pig], remembering(Mode::Graze, 1.0, 0.0)))), Mode::Graze);
    }

    #[test]
    fn after_a_scare_it_ignores_alarms_for_a_while() {
        let brain = CowBrain::default();
        let scared = cow(2.0, 0.0, 1, ALARMED);
        // The flee just running out: it calms down and stays calm with the alarm still showing.
        let out = brain.think(&seeing(vec![scared.clone()], remembering(Mode::Flee, 0.001, 0.0)));
        assert_eq!(mode(&out), Mode::Idle);
        assert_eq!(out.signal, "");
        assert!((out.memory[Slot::Calm as usize] - brain.calm_time).abs() < 1e-4);
        let outs = run(&brain, seeing(vec![scared.clone()], BrainContext { memory: out.memory, ..ctx() }), brain.calm_time - 0.2);
        assert!(outs.iter().all(|o| mode(o) != Mode::Flee), "calm for its calm time");
        // Then it can be scared again.
        let last = outs.last().unwrap().memory.clone();
        let outs = run(&brain, seeing(vec![scared], BrainContext { memory: last, ..ctx() }), 0.5);
        assert_eq!(mode(outs.last().unwrap()), Mode::Flee);
        // A hit scares it even while calm.
        let calm = BrainContext { memory: memory(Mode::Idle, 2.0, 0.0, 3.0), ..ctx() };
        let hit = BrainContext { hurt: Some(Hurt { from: Vec3::X, damage: 1.0 }), ..calm };
        assert_eq!(mode(&brain.think(&hit)), Mode::Flee);
    }

    #[test]
    fn it_shows_what_it_is_doing() {
        let brain = CowBrain::default();
        assert_eq!(brain.think(&remembering(Mode::Graze, 1.0, 0.0)).signal, GRAZING);
        assert_eq!(brain.think(&remembering(Mode::Flee, 1.0, 0.0)).signal, ALARMED);
        assert_eq!(brain.think(&remembering(Mode::Wander, 1.0, 0.0)).signal, "");
    }

    /// A player `x` meters east and `z` up.
    fn player_at(x: f32, z: f32) -> Option<Seen> {
        Some(seen(Vec3::new(x, 0.0, z)))
    }

    /// The player at `offset`, standing still, with a clear run all the way.
    fn seen(offset: Vec3) -> Seen {
        Seen { offset, velocity: Vec3::ZERO, clear_run: level(offset).length() }
    }

    /// A cow fighting the player `x` meters east, in `mode` with `left` seconds of it and
    /// `anger` seconds of anger left.
    fn fighting(mode: Mode, left: f32, anger: f32, x: f32) -> BrainContext {
        let mut memory = memory(mode, left, 0.0, 0.0);
        memory[Slot::Anger as usize] = anger;
        memory[Slot::Best as usize] = f32::MAX;
        BrainContext { memory, player: player_at(x, 0.0), ..ctx() }
    }

    /// Like [`run`], but handing back `attack_landed` and `blocked` as the test says, by tick.
    fn run_with(
        brain: &CowBrain,
        start: BrainContext,
        seconds: f32,
        mut each: impl FnMut(usize, &mut BrainContext),
    ) -> Vec<BrainOutput> {
        let mut c = start;
        let mut outs = Vec::new();
        for tick in 0..(seconds / DT) as usize {
            each(tick, &mut c);
            let out = brain.think(&c);
            c = BrainContext {
                memory: out.memory.clone(),
                yaw: out.yaw,
                velocity: out.walk.extend(0.0),
                hurt: None,
                attack_landed: false,
                blocked: false,
                ..c
            };
            outs.push(out);
        }
        outs
    }

    #[test]
    fn a_hit_cow_that_fights_squares_up_alarmed_then_angry() {
        let brain = CowBrain::default();
        let hit = BrainContext { hurt: Some(Hurt { from: Vec3::X, damage: 5.0 }), health: 15.0, player: player_at(3.0, 0.0), ..ctx() };
        let out = brain.think(&hit);
        assert_eq!(mode(&out), Mode::Angry);
        assert_eq!(out.signal, ALARMED, "the herd scatters");
        assert_eq!(out.walk, Vec2::ZERO, "facing up first");
        let outs = run(&brain, BrainContext { hurt: None, memory: out.memory, ..hit }, ALARM_FLASH + 0.1);
        assert!(outs.iter().all(|o| mode(o).fighting()));
        assert_eq!(outs.last().unwrap().signal, ANGRY);
    }

    #[test]
    fn which_cows_fight_is_fixed_by_their_id() {
        let some = CowBrain { fight_share: 0.5, ..CowBrain::default() };
        let fighters = (0..200).filter(|&id| some.fights(id)).count();
        assert!((60..140).contains(&fighters), "{fighters} of 200 fight");
        assert_eq!(some.fights(17), some.fights(17));
        assert!((0..200).all(|id| CowBrain::default().fights(id)), "all fight by default");
    }

    #[test]
    fn close_in_it_tosses_once_as_its_head_comes_up() {
        let brain = CowBrain::default();
        let outs = run(&brain, fighting(Mode::Angry, 0.0, 10.0, 2.0), 2.0);
        assert_eq!((mode(&outs[0]), outs[0].clip.as_str()), (Mode::Toss, "punch"));
        let live: Vec<usize> = (0..outs.len()).filter(|&i| outs[i].attack.is_some()).collect();
        let first = *live.first().expect("it tossed");
        let windup = (brain.toss_windup / DT).round() as usize;
        assert!(first.abs_diff(windup) <= 1, "after the wind-up: tick {first}");
        let strike = &live[..live.iter().take_while(|&&i| i < first + 20).count()];
        assert!(strike.windows(2).all(|w| w[1] == w[0] + 1), "one unbroken strike");
        assert!(strike.len().abs_diff((PUNCH_UP / DT) as usize) <= 1, "{} ticks", strike.len());
        let toss = outs[first].attack.as_ref().unwrap();
        assert_eq!((toss.damage, toss.lift), (brain.toss_damage, brain.toss_lift));
        assert!(outs[..first].iter().all(|o| o.walk == Vec2::ZERO), "stands for the toss");
        let after = first + strike.len();
        assert!(outs[after..after + 10].iter().all(|o| o.attack.is_none()), "then a gap");
    }

    #[test]
    fn from_far_off_it_lowers_its_head_then_charges_at_the_player() {
        let brain = CowBrain::default();
        let outs = run(&brain, fighting(Mode::Angry, 0.0, 10.0, 9.0), brain.charge_windup + 0.5);
        assert_eq!((mode(&outs[0]), outs[0].clip.as_str()), (Mode::ChargeUp, "eat"));
        let windup = (brain.charge_windup / DT) as usize;
        assert!(outs[..windup - 1].iter().all(|o| o.attack.is_none() && o.walk == Vec2::ZERO), "a still wind-up");
        let charging = outs.last().unwrap();
        assert_eq!((mode(charging), charging.clip.as_str()), (Mode::Charge, "run"));
        assert!((charging.walk - Vec2::X * brain.charge_speed).length() < 1e-3, "{}", charging.walk);
        let attack = charging.attack.as_ref().expect("live while charging");
        assert_eq!(attack.damage, brain.charge_damage);
    }

    #[test]
    fn a_charge_bends_only_a_little_toward_a_dodging_player() {
        let brain = CowBrain::default();
        let start = fighting(Mode::Charge, brain.charge_time, 10.0, 5.0);
        // The player steps well off to the side.
        let dodged = BrainContext { player: Some(seen(Vec3::new(3.0, 3.0, 0.0))), ..start };
        let outs = run(&brain, dodged, 0.5);
        let turned = outs.last().unwrap().memory[Slot::Heading as usize];
        assert!((turned - brain.charge_turn * 0.5).abs() < 0.05, "turned {turned} rad in half a second");
    }

    #[test]
    fn a_charge_ends_on_a_hit_and_a_blocked_one_stuns() {
        let brain = CowBrain::default();
        let charging = || fighting(Mode::Charge, brain.charge_time, 10.0, 5.0);
        let outs = run_with(&brain, charging(), 0.2, |tick, c| c.attack_landed = tick == 5);
        assert_eq!(mode(&outs[5]), Mode::Angry, "hit: catches its breath");
        assert!(outs[5..].iter().all(|o| o.attack.is_none() && o.walk == Vec2::ZERO));
        let outs = run_with(&brain, charging(), 0.5, |tick, c| c.blocked = tick == 1 || tick == 20);
        assert_eq!(mode(&outs[1]), Mode::Charge, "blocked as it sets off doesn't count");
        assert_eq!(mode(&outs[20]), Mode::Stunned);
        let outs = run(&brain, BrainContext { memory: outs[20].memory.clone(), ..charging() }, brain.stun_time + 0.1);
        let stunned = (brain.stun_time / DT) as usize - 2;
        assert!(outs[..stunned].iter().all(|o| mode(o) == Mode::Stunned && o.walk == Vec2::ZERO && o.attack.is_none()));
        assert!(mode(outs.last().unwrap()).fighting() && mode(outs.last().unwrap()) != Mode::Stunned, "back to it");
    }

    #[test]
    fn a_charge_that_overshoots_stops() {
        let brain = CowBrain::default();
        // The player is already behind it.
        let out = brain.think(&fighting(Mode::Charge, brain.charge_time, 10.0, -2.0));
        assert_eq!(mode(&out), Mode::Angry);
    }

    #[test]
    fn it_gives_up_on_a_player_it_cannot_get_closer_to() {
        let brain = CowBrain::default();
        // Up on a block 4 m off: too high to toss, too near to charge, and walking gets it no nearer.
        let stuck = BrainContext { player: player_at(4.0, 2.0), ..fighting(Mode::Angry, 0.0, 15.0, 4.0) };
        let outs = run(&brain, stuck, brain.give_up_time + 0.5);
        let trying = (brain.give_up_time / DT) as usize - 2;
        assert!(outs[..trying].iter().all(|o| mode(o) == Mode::Angry && o.walk.length() > 0.0), "walks at them");
        let last = outs.last().unwrap();
        assert_eq!(mode(last), Mode::Flee, "gave up, and runs");
        assert!(near_angle(heading(last), PI, 1e-3), "west, away from them: {}", heading(last));
        // Then calms down for a while.
        let c = BrainContext { memory: last.memory.clone(), yaw: last.yaw, ..stuck_far(&brain) };
        let outs = run(&brain, c, brain.flee_time + 0.5);
        let last = outs.last().unwrap();
        assert!(!mode(last).fighting() && mode(last) != Mode::Flee, "{:?}", mode(last));
        assert!(last.memory[Slot::Calm as usize] > brain.calm_time - 1.0, "calm for a while");
    }

    /// The player well off, east, with no clear run to them.
    fn stuck_far(brain: &CowBrain) -> BrainContext {
        let mut p = seen(Vec3::new(brain.grudge_range + 3.0, 0.0, 0.0));
        p.clear_run = 1.0;
        BrainContext { player: Some(p), ..ctx() }
    }

    #[test]
    fn getting_closer_keeps_it_going_and_its_anger_fades() {
        let brain = CowBrain { give_up_time: 1.0, ..CowBrain::default() };
        // Each tick a little closer, from 5 m.
        let outs = run_with(&brain, fighting(Mode::Angry, 0.0, 3.0, 5.0), 2.9, |tick, c| {
            c.player = player_at(5.0 - tick as f32 * 0.01, 0.0);
        });
        let modes: Vec<Mode> = outs.iter().map(mode).collect();
        assert!(modes.iter().all(|&m| m == Mode::Angry), "never gives up while gaining: {modes:?}");
        let outs = run(&brain, fighting(Mode::Angry, 0.0, 0.2, 4.0), 0.3);
        assert!(!mode(outs.last().unwrap()).fighting(), "calmed down");
    }

    /// A route whose next point is `next` and whose end is `end`.
    fn route(next: Vec3, end: Vec3, reaches: bool) -> Route {
        Route { next, end, length: level(end).length() + 2.0, reaches, hop: false }
    }

    #[test]
    fn with_no_clear_run_it_closes_in_by_its_route_and_hops_when_told() {
        let brain = CowBrain::default();
        // Behind a wall 2 m off; the way round goes north first.
        let walled = Seen { clear_run: 2.0, ..seen(Vec3::new(10.0, 0.0, 0.0)) };
        let round = route(Vec3::new(0.0, 4.0, 0.0), Vec3::new(9.0, 0.0, 0.0), true);
        let c = BrainContext { player: Some(walled.clone()), route: Some(round.clone()), ..fighting(Mode::Angry, 0.0, 15.0, 10.0) };
        let outs = run(&brain, c.clone(), 1.0);
        let last = outs.last().unwrap();
        assert_eq!(mode(last), Mode::Angry);
        assert_eq!(last.go_to, Some(walled.offset), "asks the way to the player");
        assert!(last.walk.y > last.walk.x.abs() * 3.0, "goes north, round the wall: {}", last.walk);
        assert!(last.edge_guard.is_some());
        // Against a ledge its route hops.
        let ledge = Route { hop: true, ..round };
        let out = brain.think(&BrainContext { route: Some(ledge.clone()), blocked: true, memory: last.memory.clone(), ..c.clone() });
        assert!(out.hop);
        let out = brain.think(&BrainContext { route: Some(ledge), blocked: false, memory: last.memory.clone(), ..c });
        assert!(!out.hop, "only up against it");
    }

    #[test]
    fn with_no_path_all_the_way_it_waits_as_near_as_it_gets_until_its_anger_runs_out() {
        let brain = CowBrain::default();
        // Up on a pillar 3 high, a meter off: its route ends at the foot of it.
        let up = Seen { clear_run: 0.5, ..seen(Vec3::new(1.0, 0.0, 3.0)) };
        let foot = route(Vec3::new(0.3, 0.0, 0.0), Vec3::new(0.3, 0.0, 0.0), false);
        let anger = brain.give_up_time + 2.0;
        let c = BrainContext { player: Some(up), route: Some(foot), ..fighting(Mode::Angry, 0.0, anger, 1.0) };
        let outs = run(&brain, c.clone(), anger - 0.2);
        assert!(outs.iter().all(|o| mode(o) == Mode::Angry && o.walk == Vec2::ZERO), "waits below, past its give-up time");
        assert!(outs.iter().all(|o| o.go_to.is_some()), "still looking for a way up");
        let outs = run(&brain, BrainContext { memory: outs.last().unwrap().memory.clone(), ..c }, 0.5);
        assert!(!mode(outs.last().unwrap()).fighting(), "calmed down");
    }

    #[test]
    fn a_charge_has_no_edge_guard_so_it_can_run_off_a_cliff() {
        let brain = CowBrain::default();
        let out = brain.think(&fighting(Mode::Charge, brain.charge_time, 10.0, 5.0));
        assert_eq!(mode(&out), Mode::Charge);
        assert_eq!(out.edge_guard, None);
        let out = brain.think(&fighting(Mode::Angry, 0.0, 10.0, 4.0));
        assert_eq!(out.edge_guard, Some(brain.edge_drop), "closing in, it keeps away from edges");
        // Its route going down a block, it lets itself step down.
        let down = route(Vec3::new(2.0, 0.0, -1.0), Vec3::new(5.0, 0.0, -1.0), true);
        let walled = Seen { clear_run: 1.0, ..seen(Vec3::new(4.0, 0.0, -1.0)) };
        let out = brain.think(&BrainContext { player: Some(walled), route: Some(down), ..fighting(Mode::Angry, 0.0, 10.0, 4.0) });
        assert!(out.edge_guard.is_some_and(|g| g > 1.0), "{:?}", out.edge_guard);
    }

    #[test]
    fn running_off_and_going_back_to_the_herd_go_by_path() {
        let brain = CowBrain::default();
        // Running east: it asks the way to a point ahead, and follows the route north round
        // whatever is in the way.
        let out = brain.think(&remembering(Mode::Flee, 2.0, 0.0));
        let ahead = out.go_to.expect("a goal ahead");
        assert!(ahead.x > 5.0 && ahead.y.abs() < 1e-3, "{ahead}");
        let c = BrainContext { route: Some(route(Vec3::new(0.0, 3.0, 0.0), ahead, true)), ..remembering(Mode::Flee, 2.0, 0.0) };
        let outs = run(&brain, c, 0.5);
        assert!(outs.last().unwrap().walk.y > outs.last().unwrap().walk.x.abs(), "north by the route");
        assert!(near_angle(outs.last().unwrap().memory[Slot::Heading as usize], 0.0, 1e-3), "still running east");
        // Strayed, it asks the way to the middle of the herd.
        let herd = vec![cow(-1.0, 8.0, 1, ""), cow(1.0, 9.0, 2, ""), cow(0.0, 10.0, 3, "")];
        let out = brain.think(&seeing(herd, remembering(Mode::Wander, 5.0, 0.0)));
        let middle = out.go_to.expect("the way home");
        assert!((middle - Vec3::new(0.0, 9.0, 0.0)).length() < 1e-3, "{middle}");
        // Otherwise wandering asks nothing.
        assert_eq!(brain.think(&remembering(Mode::Wander, 5.0, 0.0)).go_to, None);
    }

    #[test]
    fn it_only_charges_with_a_clear_run_at_the_player() {
        let brain = CowBrain::default();
        let far = 10.0;
        let charges = |player: Seen| {
            let c = BrainContext { player: Some(player), ..fighting(Mode::Angry, 0.0, 15.0, far) };
            run(&brain, c, 0.5).iter().any(|o| mode(o) == Mode::ChargeUp)
        };
        assert!(charges(seen(Vec3::new(far, 0.0, 0.0))), "open ground");
        let walled = Seen { clear_run: 4.0, ..seen(Vec3::new(far, 0.0, 0.0)) };
        assert!(!charges(walled.clone()), "a wall in the way");
        let c = BrainContext { player: Some(walled), ..fighting(Mode::Angry, 0.0, 15.0, far) };
        assert!(run(&brain, c, 0.5).iter().all(|o| mode(o) == Mode::Angry && o.walk.length() > 0.0), "walks at them instead");
        // A wall right behind them stops its wider body a little short: still a run at them.
        assert!(charges(Seen { clear_run: far - 0.2, ..seen(Vec3::new(far, 0.0, 0.0)) }));
        // Up on a block 2 high, the run reaches the block but they're out of the charge's way.
        assert!(!charges(seen(Vec3::new(far, 0.0, 2.0))), "too high");
    }

    #[test]
    fn a_charge_runs_past_the_player_by_its_share_of_the_start_distance() {
        let brain = CowBrain::default();
        let charge = |behind: f32| {
            let mut c = fighting(Mode::Charge, brain.charge_time, 10.0, -behind);
            c.memory[Slot::Start as usize] = 10.0;
            mode(&brain.think(&c))
        };
        assert_eq!(charge(10.0 * brain.charge_overshoot - 1.0), Mode::Charge, "still running past");
        assert_eq!(charge(10.0 * brain.charge_overshoot + 1.0), Mode::Angry, "far enough past");
        // The start is measured when the run begins.
        let mut c = fighting(Mode::ChargeUp, 0.0, 10.0, 8.0);
        c.memory[Slot::Start as usize] = 0.0;
        let out = brain.think(&c);
        assert_eq!(mode(&out), Mode::Charge);
        assert!((out.memory[Slot::Start as usize] - 8.0).abs() < 1e-4);
    }

    /// A cow standing about, calm, holding a grudge, with the player `x` meters east.
    fn grudging(x: f32) -> BrainContext {
        let mut c = remembering(Mode::Idle, 3.0, 0.0);
        c.memory[Slot::Grudge as usize] = 1.0;
        BrainContext { player: player_at(x, 0.0), ..c }
    }

    /// Of 400 tries, how many times a grudging cow fights rather than runs.
    fn grudge_fights(brain: &CowBrain, start: &BrainContext) -> usize {
        (0..400u64)
            .filter(|&r| {
                let out = brain.think(&BrainContext { random: r * 7919 + 3, ..start.clone() });
                assert!(matches!(mode(&out), Mode::Angry | Mode::Flee), "fights or runs: {:?}", mode(&out));
                mode(&out) == Mode::Angry
            })
            .count()
    }

    #[test]
    fn a_hurt_cow_holds_a_grudge_for_good() {
        let brain = CowBrain { fight_share: 0.0, ..CowBrain::default() };
        let hit = BrainContext { hurt: Some(Hurt { from: Vec3::X, damage: 1.0 }), ..ctx() };
        let out = brain.think(&hit);
        assert_eq!(out.memory[Slot::Grudge as usize], 1.0);
        // A long while later, with the player gone, it still remembers.
        let later = run(&brain, BrainContext { memory: out.memory, ..ctx() }, 120.0);
        assert!(later.iter().all(|o| o.memory[Slot::Grudge as usize] == 1.0));
        // A cow never hurt doesn't.
        assert!(run(&brain, ctx(), 5.0).iter().all(|o| o.memory[Slot::Grudge as usize] == 0.0));
    }

    #[test]
    fn a_grudging_cow_fights_or_runs_when_the_player_comes_near_likelier_to_fight_when_healthy() {
        let brain = CowBrain::default();
        let whole = grudge_fights(&brain, &grudging(5.0));
        let hurt = grudge_fights(&brain, &BrainContext { health: 10.0, ..grudging(5.0) });
        assert!((whole as f32 / 400.0 - brain.grudge_fight).abs() < 0.08, "whole: {whole}");
        assert!((hurt as f32 / 400.0 - brain.grudge_fight * 0.5).abs() < 0.08, "half health: {hurt}");
        // With no clear run at them, it always runs, away from them.
        let walled = BrainContext { player: Some(Seen { clear_run: 1.0, ..seen(Vec3::new(5.0, 0.0, 0.0)) }), ..grudging(5.0) };
        assert_eq!(grudge_fights(&brain, &walled), 0);
        assert!(near_angle(heading(&brain.think(&walled)), PI, 1e-3));
        // Further off, it carries on; just after a scare, it ignores them.
        assert_eq!(mode(&brain.think(&grudging(brain.grudge_range + 3.0))), Mode::Idle);
        let mut calm = grudging(5.0);
        calm.memory[Slot::Calm as usize] = 2.0;
        assert_eq!(mode(&brain.think(&calm)), Mode::Watch);
        // Without a grudge, it only watches.
        let mut never = grudging(5.0);
        never.memory[Slot::Grudge as usize] = 0.0;
        assert_eq!(mode(&brain.think(&never)), Mode::Watch);
    }

    #[test]
    fn seeing_a_herdmate_hurt_or_fighting_it_holds_a_grudge_too() {
        let brain = CowBrain::default();
        let grudge = |others: Vec<Neighbor>| brain.think(&seeing(others, ctx())).memory[Slot::Grudge as usize];
        let hurt = Neighbor { health: 0.7, ..cow(5.0, 0.0, 1, "") };
        assert_eq!(grudge(vec![hurt]), 1.0, "a hurt one");
        assert_eq!(grudge(vec![cow(5.0, 0.0, 1, ANGRY)]), 1.0, "one fighting");
        assert_eq!(grudge(vec![cow(5.0, 0.0, 1, ""), cow(3.0, 0.0, 2, GRAZING), cow(4.0, 0.0, 3, ALARMED)]), 0.0, "calm, or only scared");
        // From the next tick, it reacts to the player like one that was hurt.
        let mut c = seeing(vec![Neighbor { health: 0.7, ..cow(5.0, 0.0, 1, "") }], remembering(Mode::Idle, 3.0, 0.0));
        c.player = player_at(5.0, 0.0);
        let first = brain.think(&c);
        assert_eq!(mode(&first), Mode::Watch, "not yet");
        let next = brain.think(&BrainContext { memory: first.memory, ..c });
        assert!(matches!(mode(&next), Mode::Angry | Mode::Flee));
    }

    /// A cow holding a grudge, running west from the player `x` meters east, with `health`.
    fn running_from(x: f32, health: f32) -> BrainContext {
        let mut c = remembering(Mode::Flee, 5.0, PI);
        c.memory[Slot::Grudge as usize] = 1.0;
        BrainContext { player: player_at(x, 0.0), health, yaw: PI, ..c }
    }

    #[test]
    fn run_down_while_running_it_turns_and_fights_however_hurt() {
        let brain = CowBrain::default();
        // The player keeps up, 2 m behind, every tick.
        let outs = run(&brain, running_from(2.0, 2.0), brain.corner_time + 3.0);
        let turned = outs.iter().position(|o| mode(o) == Mode::Angry).expect("turned to fight");
        assert!((turned as f32 * DT - brain.corner_time).abs() < 0.05, "after {} s", turned as f32 * DT);
        assert!(outs[turned..].iter().all(|o| mode(o).fighting()), "and doesn't break off, badly hurt and alone");
        // Kept further off, it keeps running.
        let outs = run(&brain, running_from(brain.corner_range + 1.0, 2.0), brain.corner_time + 1.0);
        assert!(outs.iter().all(|o| mode(o) == Mode::Flee));
    }

    #[test]
    fn a_grudging_cow_steers_away_from_the_player_as_it_runs() {
        let brain = CowBrain::default();
        // Running east, with the player to the north.
        let mut c = running_from(0.0, 20.0);
        c.memory[Slot::Heading as usize] = 0.0;
        c.player = Some(seen(Vec3::new(0.0, 6.0, 0.0)));
        let out = brain.think(&c);
        assert!(near_angle(heading(&out), -PI / 2.0, 1e-3), "south: {}", heading(&out));
    }

    #[test]
    fn hitting_a_fighting_cow_keeps_it_angry() {
        let brain = CowBrain::default();
        let out = brain.think(&BrainContext { hurt: Some(Hurt { from: Vec3::X, damage: 1.0 }), ..fighting(Mode::Charge, 1.0, 0.1, 5.0) });
        assert_eq!(mode(&out), Mode::Charge, "carries on");
        assert!((out.memory[Slot::Anger as usize] - brain.anger_time).abs() < 1e-5);
    }

    /// A cow ID with boldness in `range`.
    fn id_with(range: std::ops::Range<f32>) -> u64 {
        (0..10_000).find(|&id| range.contains(&boldness(id))).expect("an ID that bold")
    }

    /// Of 400 tries with different random numbers, how many times a cow standing about stands
    /// up for `other`, rather than running.
    fn defends(brain: &CowBrain, id: u64, health: f32, others: Vec<Neighbor>) -> usize {
        (0..400u64)
            .filter(|&r| {
                let c = BrainContext { id, health, random: r * 7919 + 3, player: player_at(4.0, 0.0), ..remembering(Mode::Idle, 3.0, 0.0) };
                let out = brain.think(&seeing(others.clone(), c));
                assert!(matches!(mode(&out), Mode::Angry | Mode::Flee), "fights or runs: {:?}", mode(&out));
                mode(&out) == Mode::Angry
            })
            .count()
    }

    /// A cow 3 m off, hurt to `health` of its full and alarmed.
    fn hurt_cow(health: f32) -> Neighbor {
        Neighbor { health, ..cow(3.0, 0.0, 1, ALARMED) }
    }

    #[test]
    fn bold_cows_stand_up_for_a_hurt_one_and_timid_ones_run() {
        let brain = CowBrain::default();
        let (bold, timid) = (id_with(0.95..1.0), id_with(0.0..0.05));
        let (b, t) = (defends(&brain, bold, 20.0, vec![hurt_cow(0.8)]), defends(&brain, timid, 20.0, vec![hurt_cow(0.8)]));
        assert!(b > 200 && t < 80, "bold {b}, timid {t} of 400");
        // An unhurt cow running from a scare isn't trouble: it only scares.
        assert_eq!(defends(&brain, bold, 20.0, vec![cow(3.0, 0.0, 1, ALARMED)]), 0);
        // Too far off to help, it doesn't choose at all.
        let far = Neighbor { offset: Vec3::new(brain.defend_range + 1.0, 0.0, 0.0), ..hurt_cow(0.8) };
        let c = BrainContext { id: bold, player: player_at(4.0, 0.0), ..remembering(Mode::Idle, 3.0, 0.0) };
        assert!(!mode(&brain.think(&seeing(vec![far], c))).fighting());
    }

    #[test]
    fn more_fighters_a_worse_hurt_one_and_its_own_health_sway_the_choice() {
        let brain = CowBrain::default();
        let middling = id_with(0.45..0.55);
        let alone = defends(&brain, middling, 20.0, vec![hurt_cow(0.9)]);
        let angry = |i: u64| Neighbor { id: 10 + i, ..cow(2.0 + i as f32, 1.0, 10 + i, ANGRY) };
        let backed = defends(&brain, middling, 20.0, vec![hurt_cow(0.9), angry(0), angry(1), angry(2)]);
        assert!(backed > alone + 100, "courage in numbers: {alone} then {backed}");
        let badly = defends(&brain, middling, 20.0, vec![hurt_cow(0.2)]);
        assert!(badly > alone + 60, "a badly hurt one pulls harder: {alone} then {badly}");
        let hurt_itself = defends(&brain, middling, 8.0, vec![hurt_cow(0.9)]);
        assert!(hurt_itself + 60 < alone, "hurt itself, it holds back: {alone} then {hurt_itself}");
    }

    #[test]
    fn a_cow_that_comes_to_help_is_angry_from_the_start() {
        let brain = CowBrain { defend_chance: 1.0, ..CowBrain::default() };
        let c = BrainContext { id: id_with(0.95..1.0), player: player_at(4.0, 0.0), ..remembering(Mode::Idle, 3.0, 0.0) };
        let out = brain.think(&seeing(vec![hurt_cow(0.5)], c));
        assert_eq!((mode(&out), out.signal.as_str()), (Mode::Angry, ANGRY), "no alarm, so it doesn't scatter the herd");
    }

    #[test]
    fn badly_hurt_and_alone_it_breaks_off_but_not_with_others_beside_it() {
        let brain = CowBrain::default();
        let low = brain.flee_health * 20.0 - 1.0;
        let out = brain.think(&BrainContext { health: low, ..fighting(Mode::Angry, 0.0, 10.0, 4.0) });
        assert_eq!(mode(&out), Mode::Flee, "alone");
        assert!(out.walk.x < 0.0 || libm::cosf(out.memory[Slot::Heading as usize]) < -0.9, "away from the player");
        let beside = seeing(vec![cow(2.0, 1.0, 3, ANGRY)], BrainContext { health: low, ..fighting(Mode::Angry, 0.0, 10.0, 4.0) });
        assert!(mode(&brain.think(&beside)).fighting(), "keeps going with another fighting");
        // Hit while that hurt, it runs rather than turning to fight.
        let hit = BrainContext { hurt: Some(Hurt { from: Vec3::X, damage: 1.0 }), health: low, player: player_at(3.0, 0.0), ..ctx() };
        assert_eq!(mode(&brain.think(&hit)), Mode::Flee);
    }

    #[test]
    fn it_loses_interest_in_a_player_far_off() {
        let brain = CowBrain::default();
        let out = brain.think(&fighting(Mode::Angry, 0.0, 10.0, brain.chase_range + 1.0));
        assert!(!mode(&out).fighting());
    }

    #[test]
    fn charging_a_fence_again_and_again_it_gives_up() {
        let brain = CowBrain::default();
        // The player stays 8 m off, past a fence the cow keeps running into.
        let start = fighting(Mode::Angry, 0.0, brain.anger_time, 8.0);
        let mut charging = 0;
        let outs = run_with(&brain, start, 20.0, |_, c| {
            let m = Mode::from_memory(c.memory.first().copied().unwrap_or(0.0));
            charging = if m == Mode::Charge { charging + 1 } else { 0 };
            c.blocked = charging > 12;
        });
        let stuns = outs.windows(2).filter(|w| mode(&w[0]) != Mode::Stunned && mode(&w[1]) == Mode::Stunned).count();
        assert!(stuns >= 1, "ran into it");
        let gave_up = outs.iter().position(|o| !mode(o).fighting()).expect("gave up");
        assert!((gave_up as f32 * DT) < brain.give_up_time + 4.0, "within a couple of charges: {} s, {stuns} stuns", gave_up as f32 * DT);
        assert!(outs.iter().any(|o| mode(o) == Mode::Stunned && o.clip == "die"), "crumples while stunned");
    }
}
