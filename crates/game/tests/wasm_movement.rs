//! The base game's movement controller run as WebAssembly, checked against the same code run
//! natively: every result must match bit for bit. Also checks that the sandbox limits stop
//! mods that misbehave.

use api::{Ground, MoveContext, MoveInput, MoveOutput, MovementController, Vec2, Vec3, WorldPos};
use base_movement::QuakeController;
use game::{BASE_MOVEMENT_WASM, Movement};
use sim::{PlayerInput, Simulation};
use wasm_host::{Limits, Stop, WasmController, load_movement, load_movement_with};

const PANICKY_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/panicky_movement.wasm"));
const HOSTILE_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/hostile_movement.wasm"));
const STUCK_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/stuck_movement.wasm"));
const DT: f32 = 1.0 / 60.0;

/// A small seeded generator, so the scripted input is the same every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// In `[0, 1)`.
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.unit()
    }

    fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }
}

/// Every float in an output as raw bits, so -0.0 and 0.0 count as different.
fn bits(o: &MoveOutput) -> [Option<u32>; 8] {
    let v = |x: f32| Some(x.to_bits());
    [
        v(o.velocity.x),
        v(o.velocity.y),
        v(o.velocity.z),
        v(o.impulse.x),
        v(o.impulse.y),
        v(o.impulse.z),
        o.height.map(f32::to_bits),
        o.edge_guard.map(f32::to_bits),
    ]
}

fn wasm_quake() -> WasmController {
    let (name, controller) = load_movement(BASE_MOVEMENT_WASM).expect("the component loads");
    assert_eq!(name, base_movement::QUAKE);
    controller
}

/// Scripted held keys that change now and then, like a player's: hopping, charging, crouching,
/// strafing and turning, on grass, ice and in the air.
struct Script {
    rng: Rng,
    input: MoveInput,
    ground: Option<f32>,
}

impl Script {
    fn new(seed: u64) -> Self {
        Self { rng: Rng(seed), input: MoveInput::default(), ground: Some(1.0) }
    }

    fn next(&mut self) -> MoveInput {
        let r = &mut self.rng;
        if r.chance(0.05) {
            self.input.movement = Vec2::new(r.range(-1.0, 1.0), r.range(-1.0, 1.0));
        }
        self.input.yaw = if r.chance(0.02) { r.range(-10.0, 10.0) } else { self.input.yaw + r.range(-0.1, 0.1) };
        let was_held = self.input.jump_held;
        if r.chance(0.04) {
            self.input.jump_held = !self.input.jump_held;
        }
        self.input.jump_pressed = self.input.jump_held && !was_held;
        if r.chance(0.02) {
            self.input.sprint = !self.input.sprint;
        }
        if r.chance(0.03) {
            self.input.crouch = !self.input.crouch;
        }
        self.input.clone()
    }
}

#[test]
fn wasm_matches_native_bit_for_bit() {
    let mut native: Box<dyn MovementController> = QuakeController::default().spawn();
    let prototype = wasm_quake();
    let mut wasm = prototype.spawn();
    let mut heaviest = 0;
    assert_eq!(native.tunables(), wasm.tunables());

    let mut script = Script::new(0x5eed_cafe);
    let (mut velocity, mut height) = (Vec3::ZERO, 1.8);
    for tick in 0..10_000 {
        // Now and then the ground changes, and a value gets retuned on both.
        if script.rng.chance(0.01) {
            script.ground = match script.rng.next() % 3 {
                0 => None,
                1 => Some(1.0),
                _ => Some(0.1),
            };
        }
        if script.rng.chance(0.002) {
            let tunables = native.tunables();
            let t = &tunables[script.rng.next() as usize % tunables.len()];
            let value = script.rng.range(t.min, t.max);
            assert!(native.set_tunable(&t.name, value));
            assert!(wasm.set_tunable(&t.name, value));
        }
        let ctx = MoveContext {
            dt: DT,
            velocity,
            height,
            standing_height: 1.8,
            input: script.next(),
            ground: script.ground.map(|friction| Ground { friction }),
        };
        let (a, b) = (native.step(&ctx), wasm.step(&ctx));
        assert_eq!(bits(&a), bits(&b), "tick {tick}: native {a:?}, wasm {b:?}");
        heaviest = heaviest.max(prototype.fuel_used());
        // A rough stand-in for the engine: a floor while grounded, and the asked-for height.
        velocity = a.velocity + a.impulse;
        if ctx.ground.is_some() {
            velocity.z = velocity.z.max(0.0);
        }
        height = a.height.unwrap_or(1.8);
    }
    assert_eq!(native.tunables(), wasm.tunables());

    // Well clear of the limit, so the controller can grow without tripping it.
    let budget = Limits::default().fuel;
    println!("heaviest step used {heaviest} fuel of {budget}");
    assert!(heaviest > 0 && heaviest < budget / 10, "heaviest step used {heaviest} fuel of {budget}");
}

/// Runs the whole game for a scripted few minutes from `start`, returning where the player
/// is after every tick.
fn play(movement: Movement, start: WorldPos) -> Vec<WorldPos> {
    let mut sim = game::load(movement).expect("the game loads");
    sim.teleport_player(start);
    let mut script = Script::new(0x00b0_c5e1);
    (0..6_000)
        .map(|_| {
            let held = script.next();
            sim.step(&PlayerInput {
                movement: held.movement,
                yaw: held.yaw,
                jump_pressed: held.jump_pressed,
                jump_held: held.jump_held,
                sprint: held.sprint,
                crouch: held.crouch,
                ..Default::default()
            });
            sim.player().unwrap().pos()
        })
        .collect()
}

#[test]
fn the_whole_game_moves_the_same_near_and_far() {
    let near = Simulation::with_mods(&[&base::BaseMod]).unwrap().spawn();
    let offset = base::FAR_ORIGIN.0;
    let far = WorldPos::new(near.cell() + offset, near.offset());
    for start in [near, far] {
        let (native, wasm) = (play(Movement::Native, start), play(Movement::Wasm, start));
        for (tick, (a, b)) in native.iter().zip(&wasm).enumerate() {
            assert_eq!(a.cell(), b.cell(), "tick {tick}");
            let (a, b) = (a.offset().to_array().map(f32::to_bits), b.offset().to_array().map(f32::to_bits));
            assert_eq!(a, b, "tick {tick}");
        }
        let travelled = native[0].delta_to(*native.last().unwrap()).length();
        assert!(travelled > 5.0, "the script should get the player moving, only went {travelled} m");
    }
}

#[test]
fn spawned_controllers_keep_their_own_state() {
    let prototype = wasm_quake();
    let (mut a, mut b) = (prototype.spawn(), prototype.spawn());
    let grounded = |input| MoveContext {
        dt: DT,
        velocity: Vec3::ZERO,
        height: 1.8,
        standing_height: 1.8,
        input,
        ground: Some(Ground { friction: 1.0 }),
    };
    // `a` charges a crouch jump; `b` just stands. Then both jump.
    let crouch = MoveInput { crouch: true, ..Default::default() };
    for _ in 0..60 {
        a.step(&grounded(crouch.clone()));
        b.step(&grounded(MoveInput::default()));
    }
    let jump = MoveInput { jump_pressed: true, jump_held: true, ..Default::default() };
    let (ja, jb) = (a.step(&grounded(jump.clone())).impulse.z, b.step(&grounded(jump)).impulse.z);
    assert!(ja > jb + 1.0, "charged {ja}, plain {jb}");

    // Settings are copied at spawn, then each controller's own.
    assert!(a.set_tunable("walk_speed", 9.0));
    let walk = |c: &dyn MovementController| c.tunables().into_iter().find(|t| t.name == "walk_speed").unwrap().value;
    assert_eq!(walk(a.as_ref()), 9.0);
    assert_eq!(walk(b.as_ref()), 5.0);
    assert_eq!(walk(a.spawn().as_ref()), 9.0);
}

#[test]
fn tunables_round_trip_through_wasm() {
    let mut c = wasm_quake().spawn();
    for t in c.tunables() {
        assert!(c.set_tunable(&t.name, t.min), "{}", t.name);
    }
    assert!(c.tunables().iter().all(|t| t.value == t.min));
    assert!(!c.set_tunable("nonsense", 1.0));
}

#[test]
fn a_trapping_controller_stops_without_crashing() {
    let (name, prototype) = load_movement(PANICKY_WASM).expect("the component loads");
    assert_eq!(name, "test:panicky");
    let mut c = prototype.spawn();
    assert!(c.set_tunable("speed", 3.0), "works before the trap");

    let ctx = MoveContext {
        dt: DT,
        velocity: Vec3::new(1.0, 2.0, 3.0),
        height: 1.8,
        standing_height: 1.8,
        input: MoveInput::default(),
        ground: None,
    };
    for _ in 0..3 {
        let out = c.step(&ctx);
        assert_eq!(out, MoveOutput { velocity: ctx.velocity, ..Default::default() }, "velocity passes through");
    }
    assert_eq!(prototype.stopped(), Some(Stop::Trap));
    assert!(c.tunables().is_empty());
    assert!(!c.set_tunable("speed", 1.0));
    drop(c.spawn());
}

#[test]
fn a_player_naming_an_unknown_controller_cannot_spawn() {
    // The base mod alone names `base:quake` but doesn't provide it.
    let mut sim = Simulation::with_mods(&[&base::BaseMod]).unwrap();
    let err = sim.spawn_player().unwrap_err();
    assert_eq!(err.to_string(), "no movement controller named \"base:quake\"");
}

fn still(velocity: Vec3) -> MoveContext {
    MoveContext { dt: DT, velocity, height: 1.8, standing_height: 1.8, input: MoveInput::default(), ground: None }
}

/// Loads the hostile fixture with `limits` and spawns a controller set to `behavior`.
fn hostile(limits: Limits, behavior: f32) -> (WasmController, Box<dyn MovementController>) {
    let (name, prototype) = load_movement_with(HOSTILE_WASM, limits).expect("the component loads");
    assert_eq!(name, "test:hostile");
    let mut c = prototype.spawn();
    assert!(c.set_tunable("behavior", behavior));
    (prototype, c)
}

/// Steps a controller that misbehaves in `step` and checks it stops for the right reason,
/// with velocity passing through and the instance never called again.
fn stops_in_step(behavior: f32, reason: Stop) {
    let (prototype, mut c) = hostile(Limits::default(), behavior);
    let ctx = still(Vec3::new(1.0, 2.0, 3.0));
    for _ in 0..3 {
        assert_eq!(c.step(&ctx), MoveOutput { velocity: ctx.velocity, ..Default::default() });
    }
    assert_eq!(prototype.stopped(), Some(reason));
    assert!(c.tunables().is_empty());
    drop(c.spawn());
}

#[test]
fn a_loop_runs_out_of_fuel() {
    stops_in_step(1.0, Stop::OutOfFuel);
}

#[test]
fn endless_allocation_hits_the_memory_limit() {
    stops_in_step(2.0, Stop::OutOfMemory);
}

#[test]
fn endless_recursion_overflows_the_stack() {
    stops_in_step(3.0, Stop::StackOverflow);
}

#[test]
fn a_well_behaved_hostile_controller_keeps_running() {
    let (prototype, mut c) = hostile(Limits::default(), 0.0);
    let ctx = still(Vec3::X);
    assert_eq!(c.step(&ctx).velocity, Vec3::X);
    assert_eq!(prototype.stopped(), None);
}

#[test]
fn loops_outside_step_are_stopped_too() {
    // In `spawn`: the new controller comes back stopped.
    let (prototype, c) = hostile(Limits::default(), 4.0);
    let spawned = c.spawn();
    assert_eq!(prototype.stopped(), Some(Stop::OutOfFuel));
    assert!(spawned.tunables().is_empty());

    // In `tunables`.
    let (prototype, c) = hostile(Limits::default(), 5.0);
    assert!(c.tunables().is_empty());
    assert_eq!(prototype.stopped(), Some(Stop::OutOfFuel));
}

#[test]
fn a_component_stuck_while_loading_fails_to_load() {
    let err = load_movement(STUCK_WASM).err().expect("loading gives up");
    assert_eq!(err.stop, Some(Stop::OutOfFuel), "{err}");
}

#[test]
fn controllers_from_one_mod_share_its_memory_limit() {
    let limits = Limits { memory: 64 << 20, ..Limits::default() };
    let (prototype, mut a) = hostile(limits, 6.0);
    let mut b = a.spawn();
    for c in [&mut a, &mut b] {
        assert!(c.set_tunable("allocate_mib", 40.0));
    }
    // 40 MiB fits under 64 on its own; a second 40 from the same mod doesn't.
    a.step(&still(Vec3::ZERO));
    assert_eq!(prototype.stopped(), None);
    b.step(&still(Vec3::ZERO));
    assert_eq!(prototype.stopped(), Some(Stop::OutOfMemory));
    assert!(a.tunables().is_empty(), "every controller from the mod stops");

    // A separate load of the same mod has its own memory.
    let (other, mut c) = hostile(limits, 6.0);
    assert!(c.set_tunable("allocate_mib", 40.0));
    c.step(&still(Vec3::ZERO));
    assert_eq!(other.stopped(), None);
}

#[test]
fn fuel_use_is_the_same_every_run() {
    let run = || {
        let prototype = wasm_quake();
        let mut c = prototype.spawn();
        let mut script = Script::new(0x0f0e_1000);
        let mut velocity = Vec3::ZERO;
        (0..2_000)
            .map(|_| {
                let ctx =
                    MoveContext { input: script.next(), ground: Some(Ground { friction: 1.0 }), ..still(velocity) };
                let out = c.step(&ctx);
                velocity = out.velocity + out.impulse;
                velocity.z = velocity.z.max(0.0);
                prototype.fuel_used()
            })
            .collect::<Vec<_>>()
    };
    let (a, b) = (run(), run());
    assert!(a.iter().all(|&f| f > 0));
    assert_eq!(a, b);
}
