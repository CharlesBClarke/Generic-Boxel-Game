//! The cow's brain. A cow stands about, grazes and wanders; it stops to watch the player when
//! they come near, and runs off when hit.
//!
//! Cows keep together as a herd. One that strays wanders back toward the others, one that's
//! crowded steps away, and they loosely follow a leader: the oldest cow each one can see, which
//! is the one with the lowest ID. A hit cow shows it's alarmed, and cows near an alarmed one run
//! too, so a scare spreads; once calm, a cow ignores alarms for a while, so it dies out.
//!
//! Everything it remembers between ticks is in its memory (see [`Slot`]), which the engine
//! keeps, so one brain serves every cow. Trig goes through `libm`, so it decides the same
//! natively and in WebAssembly.

use std::f32::consts::{PI, TAU};

use api::{Brain, BrainContext, BrainOutput, Neighbor, Tunable, Tunables, Vec2};

use crate::COW;

/// The registered name `targets.toml` refers to.
pub const COW_BRAIN: &str = "base:cow";

/// What a fleeing cow shows other cows.
pub const ALARMED: &str = "alarmed";
/// What a grazing cow shows other cows.
pub const GRAZING: &str = "grazing";

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
}

impl Mode {
    const ALL: [Mode; 5] = [Mode::Idle, Mode::Wander, Mode::Graze, Mode::Watch, Mode::Flee];

    fn from_memory(value: f32) -> Mode {
        Self::ALL.get(value as usize).copied().unwrap_or(Mode::Idle)
    }

    fn clip(self) -> &'static str {
        match self {
            Mode::Idle | Mode::Watch => "stand",
            Mode::Wander => "walk",
            Mode::Graze => "eat",
            Mode::Flee => "run",
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
}

const MEMORY: usize = 4;

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
        if fresh {
            // Newly spawned: stand a random while first, so cows don't all move at once.
            mode = Mode::Idle;
            left = self.idle_min * ctx.random_unit(1) + self.idle_spread * ctx.random_unit(2);
            calm = 0.0;
        }
        let herd = Herd::seen(ctx);

        let player = ctx.player.as_ref().map(|p| Vec2::new(p.offset.x, p.offset.y));
        let near = player.map(|p| p.length());
        let alarm = herd.alarm(self.alarm_range).filter(|_| calm <= 0.0 && mode != Mode::Flee);
        if let Some(hurt) = &ctx.hurt {
            mode = Mode::Flee;
            left = self.flee_time;
            calm = 0.0;
            // Away from the hit; straight up hits give no direction, so away from the player.
            let away = if hurt.from.truncate() != Vec2::ZERO { -hurt.from.truncate() } else { -player.unwrap_or(Vec2::X) };
            heading = angle(away);
        } else if let Some(other) = alarm {
            // Run the way it runs, and away from it.
            mode = Mode::Flee;
            left = self.spook_time;
            let at = level(other.offset);
            let away = level(other.velocity).normalize_or_zero() - at.normalize_or_zero() * 0.5;
            heading = if away != Vec2::ZERO { angle(away) } else { TAU * ctx.random_unit(8) };
        } else if mode != Mode::Flee {
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
            Mode::Idle | Mode::Graze => (None, 0.0, 0.0),
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
        BrainOutput {
            walk,
            yaw,
            edge_guard: Some(self.edge_drop),
            clip: mode.clip().into(),
            clip_speed,
            signal: signal.into(),
            memory,
        }
    }
}

impl CowBrain {
    fn idle_time(&self, ctx: &BrainContext) -> f32 {
        self.idle_min + self.idle_spread * ctx.random_unit(7)
    }
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
            others: Vec::new(),
            random: 1,
            memory: Vec::new(),
        }
    }

    /// A cow in `mode` with `left` seconds of it and the given heading.
    fn remembering(mode: Mode, left: f32, heading: f32) -> BrainContext {
        BrainContext { memory: vec![mode as usize as f32, left, heading, 0.0], ..ctx() }
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
        let brain = CowBrain::default();
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
        Neighbor { offset: Vec3::new(x, y, 0.0), velocity: Vec3::ZERO, kind: COW.into(), id, signal: signal.into() }
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
        let calm = BrainContext { memory: vec![Mode::Idle as usize as f32, 2.0, 0.0, 3.0], ..ctx() };
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
}
