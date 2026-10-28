//! The base game's movement controller run as WebAssembly, checked against the same code run
//! natively: every result must match bit for bit.

use api::{Ground, MoveContext, MoveInput, MoveOutput, MovementController, Vec2, Vec3, WorldPos};
use base_movement::QuakeController;
use game::{BASE_MOVEMENT_WASM, Movement};
use sim::{PlayerInput, Simulation};
use wasm_host::{WasmController, load_movement};

const PANICKY_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/panicky_movement.wasm"));
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
    let mut wasm = wasm_quake().spawn();
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
        // A rough stand-in for the engine: a floor while grounded, and the asked-for height.
        velocity = a.velocity + a.impulse;
        if ctx.ground.is_some() {
            velocity.z = velocity.z.max(0.0);
        }
        height = a.height.unwrap_or(1.8);
    }
    assert_eq!(native.tunables(), wasm.tunables());
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
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let near = Simulation::with_mods(&[&data, &base::BaseMod]).unwrap().spawn();
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
    assert!(prototype.trapped());
    assert!(c.tunables().is_empty());
    assert!(!c.set_tunable("speed", 1.0));
    drop(c.spawn());
}

#[test]
fn a_player_naming_an_unknown_controller_cannot_spawn() {
    // The base mod alone names `base:quake` but doesn't provide it.
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mut sim = Simulation::with_mods(&[&data, &base::BaseMod]).unwrap();
    let err = sim.spawn_player().unwrap_err();
    assert_eq!(err.to_string(), "no movement controller named \"base:quake\"");
}
