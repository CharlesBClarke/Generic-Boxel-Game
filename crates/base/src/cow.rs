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
//! and charges in a line it can only bend a little, for big damage and knockback; dodged into
//! a wall, it's stunned for a moment. It has no pathfinding: if it gets no closer for a while,
//! because the player stands on a block or over a fence, it gives up.
//!
//! Each cow has a boldness, fixed by its ID. A cow that sees another in trouble (fighting, or
//! hurt and alarmed) chooses once whether to stand up for it or run: bold cows, cows seeing
//! others fight, and cows seeing one badly hurt are likelier to fight; hurt ones less. A
//! fighting cow that's badly hurt, with no other cow fighting beside it, breaks off and runs.
//!
//! Everything it remembers between ticks is in its memory (see [`Slot`]), which the engine
//! keeps, so one brain serves every cow. Trig goes through `libm`, so it decides the same
//! natively and in WebAssembly.

use std::f32::consts::{PI, TAU};

use api::{Attack, Brain, BrainContext, BrainOutput, Neighbor, Tunable, Tunables, Vec2};

use crate::COW;

/// The registered name `targets.toml` refers to.
pub const COW_BRAIN: &str = "base:cow";

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
            Mode::Idle | Mode::Watch | Mode::Angry | Mode::Stunned => "stand",
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
}

const MEMORY: usize = 7;

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
/// How far past the player, in meters along its run, a charge goes before giving up.
const OVERSHOOT: f32 = 1.5;
/// Where the toss and the charge hit, in the cow's own space: in front of the head.
const TOSS_BOX: ([f32; 3], [f32; 3]) = ([-0.6, 0.9, 0.0], [0.6, 2.4, 1.9]);
const CHARGE_BOX: ([f32; 3], [f32; 3]) = ([-0.6, 0.5, 0.0], [0.6, 2.1, 1.8]);

/// How much further than `notice_range` the player has to go before a watching cow loses
/// interest, so it doesn't flicker at the edge.
const LOSE_RANGE: f32 = 1.5;

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
    /// It charges with the player at least this far, level, in meters.
    pub charge_range: f32,
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
}

impl Default for CowBrain {
    fn default() -> Self {
        Self {
            walk_speed: 1.0,
            run_speed: 3.0,
            turn_speed: 2.5,
            notice_range: 5.0,
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
            spook_time: 2.0,
            calm_time: 4.0,
            fight_share: 1.0,
            defend_range: 8.0,
            defend_chance: 0.25,
            courage: 0.15,
            injury_pull: 0.5,
            flee_health: 0.3,
            anger_time: 20.0,
            give_up_time: 4.0,
            chase_range: 20.0,
            approach_speed: 2.0,
            toss_range: 2.4,
            toss_height: 1.2,
            toss_windup: 0.5,
            toss_recover: 0.7,
            toss_damage: 2.0,
            toss_push: 6.0,
            toss_lift: 9.0,
            toss_guarded_damage: 0.25,
            toss_guarded_push: 0.5,
            charge_range: 6.0,
            charge_windup: 0.9,
            charge_speed: 7.0,
            charge_turn: 0.8,
            charge_time: 2.5,
            charge_damage: 7.0,
            charge_push: 16.0,
            charge_lift: 6.0,
            charge_guarded_damage: 0.5,
            charge_guarded_push: 0.5,
            stun_time: 2.0,
            charge_recover: 1.2,
        }
    }
}

impl Tunables for CowBrain {
    fn tunables(&self) -> Vec<Tunable> {
        vec![
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
        ]
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
        let set = |m: &mut [f32], s: Slot, v: f32| m[s as usize] = v;
        let mut mode = Mode::from_memory(get(&memory, Slot::Mode));
        let mut left = get(&memory, Slot::Left) - ctx.dt;
        let mut heading = get(&memory, Slot::Heading);
        let mut calm = (get(&memory, Slot::Calm) - ctx.dt).max(0.0);
        let mut fight = Fight {
            anger: (get(&memory, Slot::Anger) - ctx.dt).max(0.0),
            stuck: get(&memory, Slot::Stuck),
            best: get(&memory, Slot::Best),
        };
        if fresh {
            // Newly spawned: stand a random while first, so cows don't all move at once.
            mode = Mode::Idle;
            left = self.idle_min * ctx.random_unit(1) + self.idle_spread * ctx.random_unit(2);
            calm = 0.0;
        }
        let herd = Herd::seen(ctx);

        let player = ctx.player.as_ref().map(|p| Vec2::new(p.offset.x, p.offset.y));
        let near = player.map(|p| p.length());
        let settled = calm <= 0.0 && mode != Mode::Flee && !mode.fighting();
        let alarm = herd.alarm(self.alarm_range).filter(|_| settled);
        let trouble = herd.trouble(self.defend_range).filter(|_| settled && player.is_some());
        // Badly hurt with nobody fighting beside it, it loses its nerve.
        let own = ctx.health / ctx.full_health.max(1e-6);
        let nerve = own >= self.flee_health || herd.fighters(self.defend_range) > 0;
        if ctx.hurt.is_some() && mode.fighting() && nerve {
            fight.anger = self.anger_time;
        } else if ctx.hurt.is_some() && !mode.fighting() && player.is_some() && nerve && self.fights(ctx.id) {
            mode = Mode::Angry;
            left = SQUARE_UP;
            fight = Fight { anger: self.anger_time, stuck: 0.0, best: f32::MAX };
        } else if let Some(hurt) = &ctx.hurt {
            mode = Mode::Flee;
            left = self.flee_time;
            calm = 0.0;
            // Away from the hit; straight up hits give no direction, so away from the player.
            let away = if hurt.from.truncate() != Vec2::ZERO { -hurt.from.truncate() } else { -player.unwrap_or(Vec2::X) };
            heading = angle(away);
        } else if mode.fighting() && !nerve {
            // Breaks off, away from the player.
            mode = Mode::Flee;
            left = self.flee_time;
            fight = Fight { anger: 0.0, stuck: 0.0, best: f32::MAX };
            heading = angle(-player.filter(|p| *p != Vec2::ZERO).unwrap_or(Vec2::X));
        } else if trouble.is_some_and(|t| ctx.random_unit(11) < self.defend_odds(ctx, &herd, t)) {
            // Stands up for it.
            mode = Mode::Angry;
            left = SQUARE_UP;
            fight = Fight { anger: self.anger_time, stuck: 0.0, best: f32::MAX };
        } else if let Some(other) = trouble.or(alarm) {
            // Run the way it runs, and away from it.
            mode = Mode::Flee;
            left = self.spook_time;
            let at = level(other.offset);
            let away = level(other.velocity).normalize_or_zero() - at.normalize_or_zero() * 0.5;
            heading = if away != Vec2::ZERO { angle(away) } else { TAU * ctx.random_unit(8) };
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
                set(&mut memory, Slot::Mode, mode as usize as f32);
                set(&mut memory, Slot::Left, left);
                set(&mut memory, Slot::Heading, heading);
                set(&mut memory, Slot::Calm, calm);
                set(&mut memory, Slot::Anger, fight.anger);
                set(&mut memory, Slot::Stuck, fight.stuck);
                set(&mut memory, Slot::Best, fight.best);
                return BrainOutput { edge_guard: Some(self.edge_drop), memory, ..out };
            }
            // Calmed down, or gave up.
            mode = Mode::Idle;
            left = self.idle_time(ctx);
            calm = self.calm_time;
            fight = Fight { anger: 0.0, stuck: 0.0, best: f32::MAX };
        }

        if matches!(mode, Mode::Wander | Mode::Flee) && ctx.blocked && ctx.on_ground {
            // Turn away from what's in the way, by a quarter to three quarters of a turn.
            heading = ctx.yaw + PI * (0.5 + ctx.random_unit(3));
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

        let (face, speed, turn) = match mode {
            Mode::Wander => (Some(heading), self.walk_speed, self.turn_speed),
            Mode::Flee => (Some(heading), self.run_speed, self.turn_speed * 2.0),
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

        set(&mut memory, Slot::Mode, mode as usize as f32);
        set(&mut memory, Slot::Left, left);
        set(&mut memory, Slot::Heading, heading);
        set(&mut memory, Slot::Calm, calm);
        set(&mut memory, Slot::Anger, fight.anger);
        set(&mut memory, Slot::Stuck, fight.stuck);
        set(&mut memory, Slot::Best, fight.best);
        BrainOutput {
            walk,
            yaw,
            edge_guard: Some(self.edge_drop),
            clip: mode.clip().into(),
            clip_speed,
            signal: signal.into(),
            attack: None,
            memory,
        }
    }
}

impl CowBrain {
    fn idle_time(&self, ctx: &BrainContext) -> f32 {
        self.idle_min + self.idle_spread * ctx.random_unit(7)
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
        if ctx.attack_landed {
            fight.stuck = 0.0;
            fight.best = f32::MAX;
        }
        // A move running out leads back to squaring up.
        match *mode {
            Mode::Toss if *left <= 0.0 => *mode = Mode::Angry,
            Mode::ChargeUp if *left <= 0.0 => {
                *mode = Mode::Charge;
                *left = self.charge_time;
                *heading = toward;
            }
            Mode::Charge => {
                let (sin, cos) = libm::sincosf(*heading);
                let passed = to.dot(Vec2::new(cos, sin)) < -OVERSHOOT;
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
            let reach = seen.offset.length();
            if reach < fight.best - PROGRESS {
                fight.best = reach;
                fight.stuck = 0.0;
            } else {
                fight.stuck += ctx.dt;
            }
            if fight.stuck >= self.give_up_time {
                return None;
            }
            if distance <= self.toss_range && rise <= self.toss_height && rise >= -self.toss_height {
                *mode = Mode::Toss;
                *left = self.toss_windup + PUNCH_UP + self.toss_recover;
            } else if distance >= self.charge_range {
                *mode = Mode::ChargeUp;
                *left = self.charge_windup;
            }
        }

        let face = |turn: f32| libm::remainderf(turn_toward(ctx.yaw, toward, turn * ctx.dt), TAU);
        let (yaw, speed, clip_speed, attack) = match *mode {
            Mode::Angry if *left > 0.0 => (face(self.turn_speed * 2.0), 0.0, 1.0, None),
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
            _ => (ctx.yaw, 0.0, 0.5, None),
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
        Some(BrainOutput {
            walk: Vec2::new(cos, sin) * speed,
            yaw,
            edge_guard: None,
            clip: clip.into(),
            clip_speed,
            signal: signal.into(),
            attack,
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

    /// How many cows within `range` are fighting.
    fn fighters(&self, range: f32) -> usize {
        self.cows.iter().filter(|n| n.signal == ANGRY && level(n.offset).length() <= range).count()
    }

    /// The nearest alarmed cow within `range`.
    fn alarm(&self, range: f32) -> Option<&'a Neighbor> {
        self.cows.iter().copied().find(|n| n.signal == ALARMED && level(n.offset).length() <= range)
    }
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
    use api::{Hurt, Seen, Vec3};

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
        let north = |d: f32| Some(Seen { offset: Vec3::new(0.0, d, 0.0), velocity: Vec3::ZERO });
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
        // Hit from the west, while watching the player there.
        let player = Some(Seen { offset: Vec3::new(-2.0, 0.0, 0.0), velocity: Vec3::ZERO });
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
        Some(Seen { offset: Vec3::new(x, 0.0, z), velocity: Vec3::ZERO })
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
        let dodged = BrainContext { player: Some(Seen { offset: Vec3::new(3.0, 3.0, 0.0), velocity: Vec3::ZERO }), ..start };
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
        assert!(!mode(last).fighting(), "gave up: {:?}", mode(last));
        assert!(last.memory[Slot::Calm as usize] > brain.calm_time - 1.0, "calm for a while");
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
}
