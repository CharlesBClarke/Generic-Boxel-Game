//! The block-jump trainer: a temporary tool for learning block-jumping, not a feature. It
//! watches each attempt at placing a block into your feet while jumping, so the push-out
//! launches you, and says what went wrong. Like the chase drill, it lives here on its own
//! rather than in the API (see the API debt list in `SLICES.md`).
//!
//! It only reads what the simulation reports about each tick ([`sim::StepReport`] and
//! [`sim::PlaceReport`]); it never changes how block-jumping works.

use std::collections::VecDeque;

use api::{Vec3 as ZVec3, WorldPos};
use bevy::prelude::*;
use sim::{PlaceReport, PlayerInput, Simulation, StepReport};

use crate::controls::{Controls, label};

/// Ticks per second, for showing timings in milliseconds.
const TICK_MS: f32 = 1000.0 / Simulation::TICK_RATE as f32;
/// An attempt that never leaves the ground ends after this many ticks.
const GROUNDED_TICKS: u64 = 10;
/// An attempt ends after this many ticks however it's going.
const MAX_TICKS: u64 = 5 * Simulation::TICK_RATE as u64;
/// Rising faster than this when a jump replaces the speed counts as a lost kick, m/s.
const LOST_SPEED: f32 = 0.5;
/// A kick further than this off vertical is worth mentioning, degrees.
const TILT_DEGREES: f32 = 10.0;
/// How many verdicts the HUD keeps.
const HISTORY: usize = 5;

/// What the trainer sees of one tick, after it ran.
#[derive(Clone, Copy, Debug)]
pub struct Observation {
    /// The tick that just ran.
    pub tick: u64,
    pub jump_pressed: bool,
    pub jump_held: bool,
    /// The placement attempt made on this tick, if any.
    pub place: Option<PlaceReport>,
    pub step: StepReport,
    /// The velocity after collisions.
    pub velocity: ZVec3,
    pub feet: WorldPos,
    pub width: f32,
    pub on_ground: bool,
    pub flying: bool,
}

impl Observation {
    /// What `sim` did on the tick it just ran with `input`. `None` without a player.
    pub fn of(sim: &Simulation, input: &PlayerInput) -> Option<Self> {
        let player = sim.player()?;
        let tick = sim.tick().checked_sub(1)?;
        Some(Self {
            tick,
            jump_pressed: input.jump_pressed,
            jump_held: input.jump_held,
            place: sim.last_place().copied().filter(|p| p.tick == tick),
            step: player.last_step(),
            velocity: player.velocity(),
            feet: player.pos(),
            width: player.shape().width,
            on_ground: player.on_ground(),
            flying: player.flying(),
        })
    }
}

/// How an attempt went.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// The kick came on the jump's tick or `delay` ticks after it, and added to it.
    Stacked { delay: u32 },
    /// A kick with no jump.
    KickOnly,
    /// A jump `after` ticks after the kick replaced `lost` m/s of upward speed. `unpressed`
    /// when jump wasn't pressed on that tick: holding jump or an early press did it.
    KickLost { after: u32, lost: f32, unpressed: bool },
    /// The block went under feet already `clearance` meters above its boxel.
    TooLate { clearance: f32 },
    /// The block went in, but not into the body.
    Missed,
    /// Placing was refused.
    Refused,
    /// Nothing to place on in reach, or nothing to place.
    NothingInReach,
}

/// Something that cut the kick short on the tick it happened.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Blocked {
    Wall,
    Ceiling,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Verdict {
    pub outcome: Outcome,
    /// How far the feet rose above where the attempt started, meters.
    pub height: f32,
    /// How far the first kick pointed off vertical, degrees.
    pub tilt: Option<f32>,
    /// The first push-out moved the body sideways or down instead of up.
    pub sideways: bool,
    pub blocked: Option<Blocked>,
    /// Push-outs in the attempt.
    pub kicks: u32,
}

impl Verdict {
    /// Whether this is the clean version of the move.
    pub fn clean(&self) -> bool {
        matches!(self.outcome, Outcome::Stacked { delay } if delay <= 2)
    }

    pub fn text(&self) -> String {
        let ms = |ticks: u32| (ticks as f32 * TICK_MS).round();
        let ticks = |n: u32| if n == 1 { "1 tick".to_string() } else { format!("{n} ticks") };
        let mut line = match self.outcome {
            Outcome::Stacked { delay: 0 } => "Same tick: the kick stacked on the full jump.".to_string(),
            Outcome::Stacked { delay } if delay <= 2 => format!(
                "Placed {} ({} ms) after the jump: stacked, a little height lost.",
                ticks(delay),
                ms(delay)
            ),
            Outcome::Stacked { delay } => format!(
                "Late: placed {} ({} ms) after the jump, which had already slowed. Place sooner.",
                ticks(delay),
                ms(delay)
            ),
            Outcome::KickOnly => "Kick only, no jump. Jump on the same tick to stack them.".to_string(),
            Outcome::KickLost { after, lost, unpressed } => format!(
                "Kick lost: {} {} ({} ms) after placing and threw away {lost:.1} m/s. Press both together, or jump first.",
                if unpressed { "a held or early jump fired" } else { "you jumped" },
                ticks(after),
                ms(after)
            ),
            Outcome::TooLate { clearance } => format!(
                "Too late: your feet were {clearance:.2} m above that boxel, so it went under you with no kick."
            ),
            Outcome::Missed => "Missed: the block didn't go into your body. Look further down.".to_string(),
            Outcome::Refused => "Refused: no room to push you out, or the spot is taken.".to_string(),
            Outcome::NothingInReach => "Nothing to place on in reach, or no block picked.".to_string(),
        };
        if self.sideways {
            line += " Pushed out sideways, not up (crouched, or no headroom).";
        } else if let Some(tilt) = self.tilt.filter(|&t| t > TILT_DEGREES) {
            line += &format!(" Kick tilted {tilt:.0}° off vertical: you were off the boxel's center.");
        }
        match self.blocked {
            Some(Blocked::Wall) => line += " The kick ran into a wall.",
            Some(Blocked::Ceiling) => line += " The kick hit a ceiling.",
            None => {}
        }
        if self.kicks > 1 {
            line += &format!(" ({} kicks)", self.kicks);
        }
        format!("{:.2} m  {line}", self.height)
    }
}

/// A push-out seen during an attempt.
#[derive(Clone, Copy, Debug)]
struct Kick {
    tick: u64,
    direction: ZVec3,
    shift: ZVec3,
    blocked: Option<Blocked>,
}

#[derive(Clone, Copy, Debug)]
struct Attempt {
    start_tick: u64,
    start_feet: WorldPos,
    peak: f32,
    airborne: bool,
    jump_tick: Option<u64>,
    kick: Option<Kick>,
    kicks: u32,
    lost: Option<Outcome>,
    /// What the first placement that didn't kick would have meant, if there was no kick.
    miss: Option<Outcome>,
}

#[derive(Resource, Default)]
pub struct Trainer {
    pub active: bool,
    attempt: Option<Attempt>,
    /// Where the feet were at the end of the last tick, which is where placements aim from.
    last_feet: Option<WorldPos>,
    last_jump_press: Option<u64>,
    history: VecDeque<Verdict>,
    attempts: u32,
    clean: u32,
    lost: u32,
    best: f32,
}

impl Trainer {
    pub fn history(&self) -> impl DoubleEndedIterator<Item = &Verdict> {
        self.history.iter()
    }

    /// Feeds one tick. Returns a verdict when an attempt that involved placing ends.
    pub fn observe(&mut self, obs: &Observation) -> Option<Verdict> {
        let before = self.last_feet.replace(obs.feet).unwrap_or(obs.feet);
        if obs.jump_pressed {
            self.last_jump_press = Some(obs.tick);
        }
        if obs.flying {
            self.attempt = None;
            return None;
        }
        let jumped = obs.step.controller_impulse.z > 0.0;
        if self.attempt.is_none() && (jumped || obs.place.is_some()) {
            self.attempt = Some(Attempt {
                start_tick: obs.tick,
                start_feet: before,
                peak: 0.0,
                airborne: false,
                jump_tick: None,
                kick: None,
                kicks: 0,
                lost: None,
                miss: None,
            });
        }
        let attempt = self.attempt.as_mut()?;

        if let Some(place) = &obs.place {
            match place.push_out {
                Some((push, _)) => {
                    attempt.kicks += 1;
                    if attempt.kick.is_none() {
                        attempt.kick = Some(Kick {
                            tick: obs.tick,
                            direction: push.direction,
                            shift: push.shift,
                            blocked: blocked(obs.step.velocity_wanted, obs.velocity),
                        });
                    }
                }
                None if attempt.miss.is_none() => attempt.miss = Some(classify_miss(place, before, obs.width)),
                None => {}
            }
        }
        if jumped {
            match attempt.kick {
                Some(kick) if kick.tick < obs.tick && attempt.lost.is_none() && obs.step.velocity_in.z > LOST_SPEED => {
                    attempt.lost = Some(Outcome::KickLost {
                        after: (obs.tick - kick.tick) as u32,
                        lost: obs.step.velocity_in.z,
                        unpressed: !obs.jump_pressed,
                    });
                }
                _ => {}
            }
            attempt.jump_tick.get_or_insert(obs.tick);
        }
        attempt.peak = attempt.peak.max(attempt.start_feet.delta_to(obs.feet).z);
        attempt.airborne |= !obs.on_ground;

        let age = obs.tick - attempt.start_tick;
        let done = (obs.on_ground && attempt.airborne) || (!attempt.airborne && age >= GROUNDED_TICKS) || age >= MAX_TICKS;
        if !done {
            return None;
        }
        let attempt = self.attempt.take()?;
        let verdict = judge(&attempt)?;
        self.attempts += 1;
        self.clean += verdict.clean() as u32;
        self.lost += matches!(verdict.outcome, Outcome::KickLost { .. }) as u32;
        if attempt.kick.is_some() {
            self.best = self.best.max(verdict.height);
        }
        if self.history.len() == HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(verdict);
        Some(verdict)
    }

    /// Clears the history and counts.
    pub fn reset(&mut self) {
        *self = Self { active: self.active, ..Self::default() };
    }

    pub fn summary(&self) -> String {
        format!(
            "{} attempts, {} clean, {} kicks lost, best {:.2} m",
            self.attempts, self.clean, self.lost, self.best
        )
    }
}

/// Why a placement that went somewhere didn't kick, judged from where the feet were when it
/// was placed.
fn classify_miss(place: &PlaceReport, feet: WorldPos, width: f32) -> Outcome {
    let Some(target) = place.target else {
        return Outcome::NothingInReach;
    };
    if !target.allowed {
        return Outcome::Refused;
    }
    let block = feet.delta_to(WorldPos::at_boxel(target.pos));
    let half = width * 0.5;
    let under = (0..2).all(|axis| block[axis] < half && block[axis] + 1.0 > -half);
    let clearance = -(block.z + 1.0);
    if under && (0.0..3.0).contains(&clearance) {
        Outcome::TooLate { clearance }
    } else {
        Outcome::Missed
    }
}

/// Whether collisions cut a kick short: the speed wanted against the speed after colliding.
fn blocked(wanted: ZVec3, actual: ZVec3) -> Option<Blocked> {
    if wanted.z > 1.0 && actual.z <= 0.01 {
        return Some(Blocked::Ceiling);
    }
    let (wanted, actual) = (wanted.truncate(), actual.truncate());
    (wanted.length() > 1.0 && actual.dot(wanted.normalize()) < wanted.length() * 0.5).then_some(Blocked::Wall)
}

/// The verdict on a finished attempt, or `None` if it didn't involve placing.
fn judge(attempt: &Attempt) -> Option<Verdict> {
    let outcome = match (attempt.kick, attempt.lost, attempt.jump_tick) {
        (Some(_), Some(lost), _) => lost,
        (Some(kick), None, Some(jump)) if jump <= kick.tick => Outcome::Stacked { delay: (kick.tick - jump) as u32 },
        (Some(_), None, Some(_)) => Outcome::KickOnly,
        (Some(_), None, None) => Outcome::KickOnly,
        (None, _, _) => attempt.miss?,
    };
    let kick = attempt.kick;
    Some(Verdict {
        outcome,
        height: attempt.peak,
        tilt: kick.map(|k| k.direction.z.clamp(-1.0, 1.0).acos().to_degrees()),
        sideways: kick.is_some_and(|k| k.shift.z <= 0.0),
        blocked: kick.and_then(|k| k.blocked),
        kicks: attempt.kicks,
    })
}

#[derive(Component)]
pub(crate) struct TrainerHud;

pub(crate) fn setup_trainer(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont { font_size: 16.0, ..default() },
        Node { position_type: PositionType::Absolute, bottom: px(64), left: px(8), max_width: px(720), ..default() },
        TrainerHud,
        crate::dev::DevText,
    ));
}

pub(crate) fn trainer_keys(keys: Res<ButtonInput<KeyCode>>, controls: Res<Controls>, mut trainer: ResMut<Trainer>) {
    if keys.just_pressed(controls.trainer) {
        trainer.active = !trainer.active;
        trainer.reset();
    }
}

/// Called right after each simulation tick with the input it ran on.
pub(crate) fn observe_tick(sim: &Simulation, input: &PlayerInput, trainer: &mut Trainer) {
    if !trainer.active {
        return;
    }
    let Some(obs) = Observation::of(sim, input) else {
        return;
    };
    if let Some(verdict) = trainer.observe(&obs) {
        info!("block-jump trainer: {}", verdict.text());
    }
}

pub(crate) fn draw_trainer(trainer: Res<Trainer>, controls: Res<Controls>, mut text: Single<&mut Text, With<TrainerHud>>) {
    if !trainer.active {
        text.0 = format!("{} block-jump trainer", label(controls.trainer));
        return;
    }
    let mut lines = format!("BLOCK-JUMP TRAINER  {}  ({} stop)", trainer.summary(), label(controls.trainer));
    if trainer.history.is_empty() {
        lines += "\nLook down, then jump and place together.";
    }
    for verdict in trainer.history().rev() {
        lines += "\n";
        lines += &verdict.text();
    }
    text.0 = lines;
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::BlockId;

    const DOWN: f32 = -89.0 * std::f32::consts::PI / 180.0;

    fn base_sim() -> Simulation {
        let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
        let mods: [&dyn api::Mod; 3] = [&data, &base::BaseMod, &base_movement::BaseMovementMod];
        let mut sim = Simulation::with_mods(&mods).expect("base mod loads");
        sim.spawn_player().expect("base registers a player");
        sim
    }

    /// One scripted tick: which buttons go down on it, and whether jump is held.
    #[derive(Clone, Copy, Default)]
    struct Press {
        jump: bool,
        hold_jump: bool,
        place: bool,
    }

    struct Rig {
        sim: Simulation,
        trainer: Trainer,
        /// A placeable full block.
        brick: BlockId,
        pitch: f32,
        yaw: f32,
    }

    impl Rig {
        fn new() -> Self {
            let mut sim = base_sim();
            for _ in 0..30 {
                sim.step(&PlayerInput::default());
            }
            let brick = sim.blocks().id("base:brick").unwrap();
            let trainer = Trainer { active: true, ..Trainer::default() };
            Self { sim, trainer, brick, pitch: DOWN, yaw: 0.0 }
        }

        fn tick(&mut self, press: Press) -> Option<Verdict> {
            let input = PlayerInput {
                yaw: self.yaw,
                pitch: self.pitch,
                jump_pressed: press.jump,
                jump_held: press.jump || press.hold_jump,
                alt_pressed: press.place,
                alt_held: press.place,
                place_block: self.brick,
                ..Default::default()
            };
            self.sim.step(&input);
            self.trainer.observe(&Observation::of(&self.sim, &input).unwrap())
        }

        /// Runs `presses`, then idle ticks until a verdict comes, for at most six seconds.
        fn run(&mut self, presses: &[Press]) -> Option<Verdict> {
            let mut verdict = None;
            for &press in presses {
                verdict = verdict.or(self.tick(press));
            }
            for _ in 0..360 {
                if verdict.is_some() {
                    break;
                }
                verdict = self.tick(Press::default());
            }
            verdict
        }
    }

    const JUMP: Press = Press { jump: true, hold_jump: false, place: false };
    const PLACE: Press = Press { jump: false, hold_jump: false, place: true };
    const BOTH: Press = Press { jump: true, hold_jump: false, place: true };
    const IDLE: Press = Press { jump: false, hold_jump: false, place: false };

    fn idle(n: usize) -> Vec<Press> {
        vec![IDLE; n]
    }

    #[test]
    fn jump_and_place_on_the_same_tick_stacks() {
        let mut rig = Rig::new();
        let v = rig.run(&[BOTH]).expect("a verdict");
        assert_eq!(v.outcome, Outcome::Stacked { delay: 0 });
        assert!(v.clean());
        assert!(v.height > 3.0, "stacked height {}", v.height);
        assert!(v.tilt.unwrap() < TILT_DEGREES, "centered, so straight up");
        assert!(!v.sideways);
    }

    #[test]
    fn jumping_after_placing_loses_the_kick() {
        let mut rig = Rig::new();
        let mut presses = vec![PLACE];
        presses.extend(idle(2));
        presses.push(JUMP);
        let v = rig.run(&presses).expect("a verdict");
        let Outcome::KickLost { after, lost, unpressed } = v.outcome else {
            panic!("expected a lost kick, got {:?}", v.outcome);
        };
        assert_eq!(after, 3);
        assert!(lost > 3.0, "lost {lost} m/s");
        assert!(!unpressed);

        let mut stacked = Rig::new();
        let best = stacked.run(&[BOTH]).unwrap().height;
        assert!(v.height < best - 1.0, "lost {} against stacked {best}", v.height);
    }

    #[test]
    fn holding_jump_after_a_kick_hops_and_loses_it() {
        // With hold-to-hop, a held jump fires by itself inside the coyote window the kick
        // leaves behind.
        let mut rig = Rig::new();
        let mut presses = vec![PLACE];
        presses.push(Press { jump: false, hold_jump: true, place: false });
        let v = rig.run(&presses).expect("a verdict");
        let Outcome::KickLost { after, unpressed, .. } = v.outcome else {
            panic!("expected a lost kick, got {:?}", v.outcome);
        };
        assert_eq!(after, 1);
        assert!(unpressed, "jump was only held");
    }

    #[test]
    fn placing_late_in_the_jump_still_stacks_but_lower() {
        let mut rig = Rig::new();
        let mut presses = vec![JUMP];
        presses.extend(idle(7));
        presses.push(PLACE);
        let v = rig.run(&presses).expect("a verdict");
        assert_eq!(v.outcome, Outcome::Stacked { delay: 8 });
        assert!(!v.clean());

        let best = Rig::new().run(&[BOTH]).unwrap().height;
        assert!(v.height < best, "late {} against stacked {best}", v.height);
    }

    #[test]
    fn placing_once_the_feet_are_above_the_boxel_gives_no_kick() {
        let mut rig = Rig::new();
        let mut presses = vec![JUMP];
        presses.extend(idle(19));
        presses.push(PLACE);
        let v = rig.run(&presses).expect("a verdict");
        let Outcome::TooLate { clearance } = v.outcome else {
            panic!("expected too late, got {:?}", v.outcome);
        };
        assert!(clearance > 0.0 && clearance < 0.5, "clearance {clearance}");
        assert_eq!(v.kicks, 0);
    }

    #[test]
    fn a_kick_without_a_jump_says_so() {
        let mut rig = Rig::new();
        let v = rig.run(&[PLACE]).expect("a verdict");
        assert_eq!(v.outcome, Outcome::KickOnly);
    }

    #[test]
    fn aiming_ahead_misses_and_aiming_up_finds_nothing() {
        let mut rig = Rig::new();
        rig.yaw = std::f32::consts::PI;
        rig.pitch = -0.6;
        assert_eq!(rig.run(&[PLACE]).expect("a verdict").outcome, Outcome::Missed);

        let mut rig = Rig::new();
        rig.pitch = 1.2;
        assert_eq!(rig.run(&[PLACE]).expect("a verdict").outcome, Outcome::NothingInReach);
    }

    #[test]
    fn plain_jumps_give_no_verdict() {
        let mut rig = Rig::new();
        assert_eq!(rig.run(&[JUMP]), None);
        assert_eq!(rig.trainer.attempts, 0);
    }

    #[test]
    fn refused_placements_and_blocked_kicks_are_named() {
        let place = |allowed| PlaceReport {
            tick: 0,
            target: Some(sim::PlaceTarget {
                pos: api::BoxelPos::new(0, 0, 0),
                occupant: sim::Occupant { block: BlockId::EMPTY, orientation: api::Orientation::default() },
                allowed,
            }),
            push_out: None,
        };
        let feet = WorldPos::new(api::I64Vec3::ZERO, ZVec3::new(0.5, 0.5, 0.0));
        assert_eq!(classify_miss(&place(false), feet, 0.6), Outcome::Refused);
        let Outcome::TooLate { clearance } = classify_miss(&place(true), feet.translate(ZVec3::Z * 1.3), 0.6) else {
            panic!("feet above the boxel should be too late");
        };
        assert!((clearance - 0.3).abs() < 1e-4);
        assert_eq!(classify_miss(&place(true), feet.translate(ZVec3::X * 2.0), 0.6), Outcome::Missed);

        assert_eq!(blocked(ZVec3::new(0.0, 0.0, 6.0), ZVec3::ZERO), Some(Blocked::Ceiling));
        assert_eq!(blocked(ZVec3::new(4.0, 0.0, 4.0), ZVec3::new(0.0, 0.0, 4.0)), Some(Blocked::Wall));
        assert_eq!(blocked(ZVec3::new(4.0, 0.0, 4.0), ZVec3::new(4.0, 0.0, 4.0)), None);
    }
}
