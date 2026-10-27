//! The base game's movement: Quake-style. Speed you build up is kept, air strafing can add
//! to it, and jumping on the landing tick skips friction, so bunny hopping works.
//!
//! Crouching is a robot's spring. Crouching mid-air tucks the feet up (the engine keeps the
//! head put), so jump-then-crouch clears higher ledges. Crouching on the ground compresses
//! the spring: the charge builds over `charge_time`, sinking the body a little, and a jump
//! from a full charge reaches `charged_jump_height`. The charge survives letting go of crouch
//! for `charge_grace`, so sliding a thumb from crouch to jump still gets it. The body springs
//! to standing height as it jumps, so keeping crouch held tucks again in the air and the two
//! stack.
//!
//! The game runs this crate compiled to WebAssembly, as a component exporting the controller.
//! It also builds natively, for its tests and the `--native-movement` flag.

use api::{ApiError, Mod, MoveContext, MoveOutput, MovementController, Registrar, Tunable, Tunables, Vec3};

/// The controller's registered name.
pub const QUAKE: &str = "base:quake";

#[cfg(target_arch = "wasm32")]
api::export_movement_controller!(QUAKE, QuakeController);

/// Registers the controller natively, for running without WebAssembly.
pub struct BaseMovementMod;

impl Mod for BaseMovementMod {
    fn name(&self) -> &str {
        "base"
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        r.register_movement_controller(QUAKE, Box::new(QuakeController::default()))
    }
}

/// Every number here is live-tunable; the defaults are the values found by playing.
#[derive(Clone, Debug, PartialEq)]
pub struct QuakeController {
    /// Top ground speed, meters per second.
    pub walk_speed: f32,
    pub sprint_speed: f32,
    /// How quickly ground speed is reached, as a fraction of top speed per second.
    pub ground_accel: f32,
    /// How quickly ground speed bleeds off when not accelerating.
    pub ground_friction: f32,
    /// Below this speed friction acts as if moving at it, so stopping finishes cleanly.
    pub stop_speed: f32,
    /// Air acceleration. High values with a low air speed cap give Quake's air strafing.
    pub air_accel: f32,
    /// The most speed air control can add along the wish direction.
    pub air_speed_cap: f32,
    /// How high a jump lifts the feet, in meters.
    pub jump_height: f32,
    /// Gravity while rising, meters per second squared.
    pub gravity: f32,
    /// Gravity multiplier while falling, so jumps feel less floaty.
    pub fall_gravity_scale: f32,
    pub terminal_velocity: f32,
    /// A jump pressed up to this many seconds before landing still happens on landing.
    pub jump_buffer: f32,
    /// A jump up to this many seconds after walking off an edge still works.
    pub coyote_time: f32,
    /// 1 to re-jump automatically on landing while jump is held, 0 to need a fresh press.
    pub hold_to_hop: f32,
    /// Body height while crouched, meters.
    pub crouch_height: f32,
    /// Ground speed while crouched, as a fraction of walk speed.
    pub crouch_speed: f32,
    /// Seconds crouched on the ground to fully charge a jump.
    pub charge_time: f32,
    /// How high a fully charged jump lifts the feet, in meters.
    pub charged_jump_height: f32,
    /// How much lower the body sinks at full charge, meters.
    pub charge_sink: f32,
    /// Seconds after letting go of crouch that a jump still gets the charge.
    pub charge_grace: f32,
    /// While crouched on the ground, the biggest drop the body may walk off, meters. Below 1
    /// stops at every boxel edge.
    pub edge_guard_drop: f32,

    since_ground: f32,
    since_jump_press: f32,
    /// How charged the crouch jump is, 0 to 1.
    charge: f32,
    /// Seconds since crouch was let go on the ground.
    since_crouch_release: f32,
}

impl Default for QuakeController {
    fn default() -> Self {
        Self {
            walk_speed: 5.0,
            sprint_speed: 7.5,
            // Found by playing. 10 (Quake's value) left held-jump stair climbs stalling
            // against risers.
            ground_accel: 15.0,
            ground_friction: 6.0,
            stop_speed: 2.0,
            air_accel: 15.0,
            air_speed_cap: 0.8,
            jump_height: 1.25,
            gravity: 25.0,
            fall_gravity_scale: 1.4,
            terminal_velocity: 60.0,
            jump_buffer: 0.1,
            coyote_time: 0.1,
            hold_to_hop: 1.0,
            // A crouch-jump tucks the feet 0.8 higher, enough to clear a 2-boxel ledge.
            crouch_height: 1.0,
            crouch_speed: 0.4,
            charge_time: 0.6,
            charged_jump_height: 2.5,
            charge_sink: 0.15,
            // Covers a thumb sliding from crouch to jump.
            charge_grace: 0.2,
            // Minecraft's value: stops at every boxel edge.
            edge_guard_drop: 0.6,
            since_ground: f32::INFINITY,
            since_jump_press: f32::INFINITY,
            charge: 0.0,
            since_crouch_release: 0.0,
        }
    }
}

/// Quake's acceleration: adds speed along `dir` up to `wish_speed`, never removing speed
/// the body already has in other directions. `cap` limits how much speed along `dir` is
/// allowed, which is what makes air strafing gain speed.
fn accelerate(velocity: Vec3, dir: Vec3, wish_speed: f32, cap: f32, accel: f32, dt: f32) -> Vec3 {
    let current = velocity.dot(dir);
    let add = cap.min(wish_speed) - current;
    if add <= 0.0 {
        return velocity;
    }
    velocity + dir * (accel * wish_speed * dt).min(add)
}

impl QuakeController {
    /// Takeoff speed for a jump with the current charge.
    fn jump_speed(&self) -> f32 {
        let height = self.jump_height + (self.charged_jump_height - self.jump_height).max(0.0) * self.charge;
        (2.0 * self.gravity * height).sqrt()
    }

    fn apply_friction(&self, velocity: Vec3, surface: f32, dt: f32) -> Vec3 {
        let speed = velocity.truncate().length();
        if speed < 1e-4 {
            return Vec3::new(0.0, 0.0, velocity.z);
        }
        let drop = speed.max(self.stop_speed) * self.ground_friction * surface * dt;
        let scale = (speed - drop).max(0.0) / speed;
        Vec3::new(velocity.x * scale, velocity.y * scale, velocity.z)
    }
}

impl MovementController for QuakeController {
    fn spawn(&self) -> Box<dyn MovementController> {
        Box::new(Self {
            since_ground: f32::INFINITY,
            since_jump_press: f32::INFINITY,
            charge: 0.0,
            since_crouch_release: 0.0,
            ..self.clone()
        })
    }

    fn step(&mut self, ctx: &MoveContext) -> MoveOutput {
        let dt = ctx.dt;
        let input = &ctx.input;
        self.since_ground = if ctx.ground.is_some() { 0.0 } else { self.since_ground + dt };
        self.since_jump_press = if input.jump_pressed { 0.0 } else { self.since_jump_press + dt };

        let wants_jump = self.since_jump_press <= self.jump_buffer || (input.jump_held && self.hold_to_hop >= 0.5);
        let can_jump = self.since_ground <= self.coyote_time;
        let jumping = wants_jump && can_jump;

        // The spring only charges while crouched on the ground. Letting go of crouch keeps the
        // charge for the grace period, so a jump just after still gets it; leaving the ground
        // without jumping lets it go.
        if input.crouch && ctx.ground.is_some() {
            if !jumping {
                self.charge = (self.charge + dt / self.charge_time.max(1e-3)).min(1.0);
            }
            self.since_crouch_release = 0.0;
        } else if ctx.ground.is_some() {
            self.since_crouch_release += dt;
            if self.since_crouch_release > self.charge_grace + 1e-5 {
                self.charge = 0.0;
            }
        } else {
            self.charge = 0.0;
        }

        let crouched = ctx.height < ctx.standing_height - 1e-3;
        let wish = input.wish_direction();
        let wish_dir = wish.normalize_or_zero();
        let top_speed = match (crouched && ctx.ground.is_some(), input.sprint) {
            (true, _) => self.walk_speed * self.crouch_speed,
            (false, true) => self.sprint_speed,
            (false, false) => self.walk_speed,
        };
        let wish_speed = wish.length() * top_speed;

        let mut velocity = ctx.velocity;
        match &ctx.ground {
            Some(ground) => {
                // Jumping on the landing tick skips friction; that's what keeps bunny hops
                // fast. It still gets ground acceleration, or hopping from a standstill would
                // never get past the air speed cap.
                if !jumping {
                    velocity = self.apply_friction(velocity, ground.friction, dt);
                }
                let accel = self.ground_accel * ground.friction.max(0.05);
                velocity = accelerate(velocity, wish_dir, wish_speed, wish_speed, accel, dt);
            }
            None => {
                velocity = accelerate(velocity, wish_dir, wish_speed, self.air_speed_cap, self.air_accel, dt);
            }
        }

        let gravity = if velocity.z > 0.0 { self.gravity } else { self.gravity * self.fall_gravity_scale };
        velocity.z = (velocity.z - gravity * dt).max(-self.terminal_velocity);

        let mut impulse = Vec3::ZERO;
        if jumping {
            // The jump replaces any downward speed instead of fighting it.
            velocity.z = 0.0;
            impulse.z = self.jump_speed();
            self.since_jump_press = f32::INFINITY;
            self.since_ground = f32::INFINITY;
            self.charge = 0.0;
        }
        // Springing up on the jump tick, then tucking again next tick if crouch is still held.
        let height = (input.crouch && !jumping).then_some(self.crouch_height - self.charge * self.charge_sink);
        // Like Minecraft's sneaking: crouched on the ground, the body won't walk off edges.
        let guarded = input.crouch && ctx.ground.is_some() && !jumping;
        MoveOutput { velocity, impulse, height, edge_guard: guarded.then_some(self.edge_guard_drop) }
    }
}

impl Tunables for QuakeController {
    fn tunables(&self) -> Vec<Tunable> {
        vec![
            Tunable::new("walk_speed", self.walk_speed, 0.5, 20.0),
            Tunable::new("sprint_speed", self.sprint_speed, 0.5, 30.0),
            Tunable::new("ground_accel", self.ground_accel, 0.5, 50.0),
            Tunable::new("ground_friction", self.ground_friction, 0.0, 20.0),
            Tunable::new("stop_speed", self.stop_speed, 0.0, 10.0),
            Tunable::new("air_accel", self.air_accel, 0.0, 100.0),
            Tunable::new("air_speed_cap", self.air_speed_cap, 0.0, 10.0),
            Tunable::new("jump_height", self.jump_height, 0.1, 5.0),
            Tunable::new("gravity", self.gravity, 1.0, 80.0),
            Tunable::new("fall_gravity_scale", self.fall_gravity_scale, 0.5, 4.0),
            Tunable::new("terminal_velocity", self.terminal_velocity, 5.0, 150.0),
            Tunable::new("jump_buffer", self.jump_buffer, 0.0, 0.5),
            Tunable::new("coyote_time", self.coyote_time, 0.0, 0.5),
            Tunable::new("hold_to_hop", self.hold_to_hop, 0.0, 1.0),
            Tunable::new("crouch_height", self.crouch_height, 0.3, 1.8),
            Tunable::new("crouch_speed", self.crouch_speed, 0.05, 1.0),
            Tunable::new("charge_time", self.charge_time, 0.05, 3.0),
            Tunable::new("charged_jump_height", self.charged_jump_height, 0.1, 8.0),
            Tunable::new("charge_sink", self.charge_sink, 0.0, 0.5),
            Tunable::new("charge_grace", self.charge_grace, 0.0, 1.0),
            Tunable::new("edge_guard_drop", self.edge_guard_drop, 0.0, 5.0),
        ]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        let field = match name {
            "walk_speed" => &mut self.walk_speed,
            "sprint_speed" => &mut self.sprint_speed,
            "ground_accel" => &mut self.ground_accel,
            "ground_friction" => &mut self.ground_friction,
            "stop_speed" => &mut self.stop_speed,
            "air_accel" => &mut self.air_accel,
            "air_speed_cap" => &mut self.air_speed_cap,
            "jump_height" => &mut self.jump_height,
            "gravity" => &mut self.gravity,
            "fall_gravity_scale" => &mut self.fall_gravity_scale,
            "terminal_velocity" => &mut self.terminal_velocity,
            "jump_buffer" => &mut self.jump_buffer,
            "coyote_time" => &mut self.coyote_time,
            "hold_to_hop" => &mut self.hold_to_hop,
            "crouch_height" => &mut self.crouch_height,
            "crouch_speed" => &mut self.crouch_speed,
            "charge_time" => &mut self.charge_time,
            "charged_jump_height" => &mut self.charged_jump_height,
            "charge_sink" => &mut self.charge_sink,
            "charge_grace" => &mut self.charge_grace,
            "edge_guard_drop" => &mut self.edge_guard_drop,
            _ => return false,
        };
        *field = value;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{Ground, MoveInput, Vec2};

    const DT: f32 = 1.0 / 60.0;

    fn ctx(velocity: Vec3, input: MoveInput, friction: Option<f32>) -> MoveContext {
        MoveContext {
            dt: DT,
            velocity,
            height: 1.8,
            standing_height: 1.8,
            input,
            ground: friction.map(|friction| Ground { friction }),
        }
    }

    fn forward() -> MoveInput {
        MoveInput { movement: Vec2::Y, ..Default::default() }
    }

    /// Runs on flat ground, applying the floor the way the engine would.
    fn on_ground(c: &mut QuakeController, mut v: Vec3, input: &MoveInput, ticks: usize, friction: f32) -> Vec3 {
        for _ in 0..ticks {
            let out = c.step(&ctx(v, input.clone(), Some(friction)));
            v = out.velocity + out.impulse;
            v.z = v.z.max(0.0);
        }
        v
    }

    #[test]
    fn reaches_walk_speed_quickly_and_no_faster() {
        let mut c = QuakeController::default();
        let v = on_ground(&mut c, Vec3::ZERO, &forward(), 15, 1.0);
        assert!(v.x > 0.8 * c.walk_speed, "only {} after a quarter second", v.x);
        let v = on_ground(&mut c, v, &forward(), 120, 1.0);
        assert!((v.x - c.walk_speed).abs() < 0.3, "settled at {}", v.x);
    }

    #[test]
    fn sprint_is_faster() {
        let mut c = QuakeController::default();
        let sprint = MoveInput { sprint: true, ..forward() };
        let v = on_ground(&mut c, Vec3::ZERO, &sprint, 120, 1.0);
        assert!((v.x - c.sprint_speed).abs() < 0.3, "settled at {}", v.x);
    }

    #[test]
    fn friction_stops_you() {
        let mut c = QuakeController::default();
        let v = on_ground(&mut c, Vec3::new(5.0, 0.0, 0.0), &MoveInput::default(), 30, 1.0);
        assert!(v.length() < 1e-3, "still moving at {v}");
    }

    #[test]
    fn ice_slides_much_farther() {
        let distance = |friction: f32| {
            let mut c = QuakeController::default();
            let mut v = Vec3::new(5.0, 0.0, 0.0);
            let mut total = 0.0;
            for _ in 0..600 {
                v = on_ground(&mut c, v, &MoveInput::default(), 1, friction);
                total += v.x * DT;
            }
            total
        };
        let (grass, ice) = (distance(1.0), distance(0.1));
        assert!(ice > 5.0 * grass, "grass {grass} m, ice {ice} m");
    }

    #[test]
    fn air_strafing_gains_speed() {
        // Holding strafe right while turning to keep facing along the velocity: the classic
        // Quake air strafe. Speed should climb past anything the ground allows.
        let mut c = QuakeController::default();
        let mut v = Vec3::new(c.walk_speed, 0.0, 0.0);
        for _ in 0..60 {
            let input = MoveInput { movement: Vec2::X, yaw: v.y.atan2(v.x), ..Default::default() };
            v = c.step(&ctx(v, input, None)).velocity;
        }
        assert!(v.truncate().length() > c.walk_speed + 1.0, "only reached {}", v.truncate().length());
    }

    #[test]
    fn air_control_alone_cannot_exceed_the_cap_from_rest() {
        let mut c = QuakeController::default();
        let mut v = Vec3::ZERO;
        for _ in 0..120 {
            v = c.step(&ctx(v, forward(), None)).velocity;
        }
        assert!(v.truncate().length() <= c.air_speed_cap + 1e-4);
    }

    #[test]
    fn bunny_hops_keep_speed() {
        let mut c = QuakeController::default();
        let fast = Vec3::new(9.0, 0.0, -3.0);
        let hop = MoveInput { jump_held: true, ..forward() };
        let out = c.step(&ctx(fast, hop, Some(1.0)));
        assert!(out.impulse.z > 0.0, "should jump");
        assert!(out.velocity.x >= 9.0 - 1e-4, "landing jump lost speed to friction: {}", out.velocity.x);
    }

    #[test]
    fn jump_reaches_the_configured_height() {
        let mut c = QuakeController::default();
        let jump = MoveInput { jump_pressed: true, jump_held: true, ..Default::default() };
        let out = c.step(&ctx(Vec3::ZERO, jump, Some(1.0)));
        let mut v = out.velocity + out.impulse;
        let (mut z, mut peak) = (v.z * DT, 0.0f32);
        for _ in 0..120 {
            v = c.step(&ctx(v, MoveInput::default(), None)).velocity;
            z += v.z * DT;
            peak = peak.max(z);
        }
        assert!((peak - c.jump_height).abs() < 0.08, "peak {peak}");
    }

    #[test]
    fn falling_is_faster_than_rising() {
        let mut c = QuakeController::default();
        let rising = c.step(&ctx(Vec3::new(0.0, 0.0, 5.0), MoveInput::default(), None)).velocity.z;
        let falling = c.step(&ctx(Vec3::new(0.0, 0.0, -5.0), MoveInput::default(), None)).velocity.z;
        assert!((5.0 - rising) < (-5.0 - falling).abs());
    }

    #[test]
    fn terminal_velocity_caps_falls() {
        let mut c = QuakeController::default();
        let v = c.step(&ctx(Vec3::new(0.0, 0.0, -1000.0), MoveInput::default(), None)).velocity;
        assert_eq!(v.z, -c.terminal_velocity);
    }

    #[test]
    fn a_buffered_press_jumps_on_landing() {
        let mut c = QuakeController { hold_to_hop: 0.0, ..Default::default() };
        let press = MoveInput { jump_pressed: true, jump_held: true, ..Default::default() };
        c.step(&ctx(Vec3::new(0.0, 0.0, -5.0), press, None));
        // Just inside the buffer window: released key, land 5 ticks later (~0.083 s).
        for _ in 0..4 {
            c.step(&ctx(Vec3::new(0.0, 0.0, -5.0), MoveInput::default(), None));
        }
        assert!(c.step(&ctx(Vec3::ZERO, MoveInput::default(), Some(1.0))).impulse.z > 0.0);

        // Outside it: land 8 ticks later (~0.13 s).
        let mut c = QuakeController { hold_to_hop: 0.0, ..Default::default() };
        c.step(&ctx(Vec3::new(0.0, 0.0, -5.0), MoveInput { jump_pressed: true, ..Default::default() }, None));
        for _ in 0..7 {
            c.step(&ctx(Vec3::new(0.0, 0.0, -5.0), MoveInput::default(), None));
        }
        assert_eq!(c.step(&ctx(Vec3::ZERO, MoveInput::default(), Some(1.0))).impulse.z, 0.0);
    }

    #[test]
    fn coyote_time_allows_late_jumps_once() {
        let press = MoveInput { jump_pressed: true, jump_held: true, ..Default::default() };
        let mut c = QuakeController { hold_to_hop: 0.0, ..Default::default() };
        c.step(&ctx(Vec3::ZERO, MoveInput::default(), Some(1.0)));
        for _ in 0..4 {
            c.step(&ctx(Vec3::ZERO, MoveInput::default(), None));
        }
        assert!(c.step(&ctx(Vec3::ZERO, press.clone(), None)).impulse.z > 0.0, "late jump inside the window");
        assert_eq!(c.step(&ctx(Vec3::ZERO, press.clone(), None)).impulse.z, 0.0, "no second coyote jump");

        let mut c = QuakeController { hold_to_hop: 0.0, ..Default::default() };
        c.step(&ctx(Vec3::ZERO, MoveInput::default(), Some(1.0)));
        for _ in 0..8 {
            c.step(&ctx(Vec3::ZERO, MoveInput::default(), None));
        }
        assert_eq!(c.step(&ctx(Vec3::ZERO, press, None)).impulse.z, 0.0, "too late");
    }

    #[test]
    fn tunables_round_trip() {
        let mut c = QuakeController::default();
        for t in c.tunables() {
            assert!(t.min <= t.value && t.value <= t.max, "{} default {} outside its range", t.name, t.value);
            assert!(c.set_tunable(&t.name, t.min));
        }
        assert!(c.tunables().iter().all(|t| t.value == t.min));
        assert!(!c.set_tunable("nonsense", 1.0));
    }

    fn crouched_ctx(velocity: Vec3, input: MoveInput, friction: Option<f32>) -> MoveContext {
        MoveContext { height: 1.0, ..ctx(velocity, input, friction) }
    }

    #[test]
    fn crouching_walks_slower() {
        let mut c = QuakeController::default();
        let input = MoveInput { crouch: true, sprint: true, ..forward() };
        let mut v = Vec3::ZERO;
        for _ in 0..120 {
            let out = c.step(&crouched_ctx(v, input.clone(), Some(1.0)));
            v = out.velocity + out.impulse;
            v.z = v.z.max(0.0);
        }
        let expected = c.walk_speed * c.crouch_speed;
        assert!((v.x - expected).abs() < 0.2, "settled at {}, expected {expected}", v.x);
    }

    #[test]
    fn guards_edges_only_while_crouched_on_the_ground() {
        let mut c = QuakeController::default();
        let crouch = MoveInput { crouch: true, ..Default::default() };
        assert_eq!(c.step(&crouched_ctx(Vec3::ZERO, crouch.clone(), Some(1.0))).edge_guard, Some(0.6));
        assert_eq!(c.step(&ctx(Vec3::ZERO, MoveInput::default(), Some(1.0))).edge_guard, None);
        assert_eq!(c.step(&crouched_ctx(Vec3::ZERO, crouch.clone(), None)).edge_guard, None, "not in the air");
        let jump = MoveInput { jump_pressed: true, jump_held: true, ..crouch };
        assert_eq!(c.step(&crouched_ctx(Vec3::ZERO, jump, Some(1.0))).edge_guard, None, "not when jumping");
    }

    #[test]
    fn asks_for_crouch_height_only_while_crouching() {
        let mut c = QuakeController::default();
        assert_eq!(c.step(&ctx(Vec3::ZERO, MoveInput::default(), Some(1.0))).height, None);
        let out = c.step(&ctx(Vec3::ZERO, MoveInput { crouch: true, ..Default::default() }, None));
        assert_eq!(out.height, Some(c.crouch_height), "a mid-air tuck, no charge");
    }

    /// The jump impulse after crouching on the ground for `ticks`, with crouch still held.
    fn jump_after_crouching(c: &mut QuakeController, ticks: usize) -> MoveOutput {
        let crouch = MoveInput { crouch: true, ..Default::default() };
        for _ in 0..ticks {
            c.step(&crouched_ctx(Vec3::ZERO, crouch.clone(), Some(1.0)));
        }
        c.step(&crouched_ctx(Vec3::ZERO, MoveInput { jump_pressed: true, jump_held: true, ..crouch }, Some(1.0)))
    }

    fn rise(c: &QuakeController, impulse: f32) -> f32 {
        impulse * impulse / (2.0 * c.gravity)
    }

    #[test]
    fn charging_raises_the_jump_up_to_the_charged_height() {
        let mut c = QuakeController::default();
        let instant = jump_after_crouching(&mut c, 0);
        assert!((rise(&c, instant.impulse.z) - c.jump_height).abs() < 0.02, "no charge, normal jump");

        let half = (c.charge_time / DT / 2.0).round() as usize;
        let impulse = jump_after_crouching(&mut c, half).impulse.z;
        let mid = rise(&c, impulse);
        let midway = (c.jump_height + c.charged_jump_height) / 2.0;
        assert!((mid - midway).abs() < 0.1, "half charge rose {mid}, expected about {midway}");

        let full = jump_after_crouching(&mut c, 120);
        assert!((rise(&c, full.impulse.z) - c.charged_jump_height).abs() < 0.02);
        assert_eq!(full.height, None, "the body springs up on the jump tick");
    }

    #[test]
    fn the_charge_sinks_the_body() {
        let mut c = QuakeController::default();
        let crouch = MoveInput { crouch: true, ..Default::default() };
        let mut out = MoveOutput::default();
        for _ in 0..120 {
            out = c.step(&crouched_ctx(Vec3::ZERO, crouch.clone(), Some(1.0)));
        }
        let sunk = out.height.unwrap();
        assert!((sunk - (c.crouch_height - c.charge_sink)).abs() < 1e-5, "at {sunk}");
    }

    /// The jump impulse after a full charge, letting go of crouch, then waiting `gap` ticks.
    /// A gap of 0 lets go of crouch on the jump tick itself.
    fn jump_after_release(c: &mut QuakeController, gap: usize) -> f32 {
        let crouch = MoveInput { crouch: true, ..Default::default() };
        for _ in 0..120 {
            c.step(&crouched_ctx(Vec3::ZERO, crouch.clone(), Some(1.0)));
        }
        for _ in 0..gap {
            c.step(&ctx(Vec3::ZERO, MoveInput::default(), Some(1.0)));
        }
        let jump = MoveInput { jump_pressed: true, jump_held: true, ..Default::default() };
        let out = c.step(&ctx(Vec3::ZERO, jump, Some(1.0)));
        rise(c, out.impulse.z)
    }

    #[test]
    fn letting_go_of_crouch_keeps_the_charge_briefly() {
        let mut c = QuakeController::default();
        assert!((jump_after_release(&mut c, 0) - c.charged_jump_height).abs() < 0.02, "same tick");
        let grace = (c.charge_grace / DT).round() as usize;
        assert!((jump_after_release(&mut c, grace - 1) - c.charged_jump_height).abs() < 0.02, "inside the grace");
        assert!((jump_after_release(&mut c, grace + 1) - c.jump_height).abs() < 0.02, "after the grace");
    }

    #[test]
    fn standing_up_loses_the_charge() {
        let mut c = QuakeController::default();
        let crouch = MoveInput { crouch: true, ..Default::default() };
        for _ in 0..120 {
            c.step(&crouched_ctx(Vec3::ZERO, crouch.clone(), Some(1.0)));
        }
        for _ in 0..30 {
            c.step(&ctx(Vec3::ZERO, MoveInput::default(), Some(1.0)));
        }
        let out = jump_after_crouching(&mut c, 0);
        assert!((rise(&c, out.impulse.z) - c.jump_height).abs() < 0.02);
    }
}
