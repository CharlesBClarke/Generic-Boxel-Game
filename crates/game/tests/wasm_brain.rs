//! The base game's cow brain run as WebAssembly, checked against the same code run natively:
//! every decision must match bit for bit. Also checks that a brain that misbehaves is stopped
//! and the game carries on.

use api::{
    ApiError, Brain, BrainContext, BrainOutput, Hurt, Mod, Neighbor, Registrar, Route, Seen, Tunables, Vec3, WorldPos,
};
use base_cow::{ALARMED, ANGRY, COW, COW_BRAIN, CowBrain, GRAZING};
use game::{BASE_COW_WASM, Code, Where};
use sim::{PlayerInput, Simulation};
use wasm_host::{Limits, Stop, WasmBrain, load_brain};

const HOSTILE_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/hostile_brain.wasm"));
const DT: f32 = 1.0 / 60.0;

/// A small seeded generator, so the contexts are the same every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn range(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * ((self.next() >> 40) as f32 / (1u64 << 24) as f32)
    }

    fn chance(&mut self, p: f32) -> bool {
        self.range(0.0, 1.0) < p
    }

    fn vec(&mut self, size: f32, up: f32) -> Vec3 {
        Vec3::new(self.range(-size, size), self.range(-size, size), self.range(-up, up))
    }
}

fn wasm_cow() -> WasmBrain {
    let (name, brain) = load_brain(BASE_COW_WASM).expect("the component loads");
    assert_eq!(name, COW_BRAIN);
    brain
}

/// A context for creature `id` with its `memory`, the world around it made up from `rng`: the
/// player near or far, hurts, neighbors with every signal, and routes that hop or fall short.
fn context(rng: &mut Rng, id: u64, memory: Vec<f32>) -> BrainContext {
    let signals = ["", ALARMED, GRAZING, ANGRY];
    let others = (0..rng.next() % 9)
        .map(|_| Neighbor {
            offset: rng.vec(12.0, 0.5),
            velocity: rng.vec(4.0, 0.0),
            kind: COW.into(),
            id: rng.next() % 12,
            signal: signals[(rng.next() % 4) as usize].into(),
            health: rng.range(0.0, 1.0),
        })
        .collect();
    BrainContext {
        dt: DT,
        id,
        velocity: rng.vec(5.0, 2.0),
        yaw: rng.range(-7.0, 7.0),
        on_ground: rng.chance(0.9),
        blocked: rng.chance(0.1),
        health: rng.range(0.0, 20.0),
        full_health: 20.0,
        player: rng.chance(0.9).then(|| Seen {
            offset: rng.vec(25.0, 3.0),
            velocity: rng.vec(8.0, 3.0),
            clear_run: rng.range(0.0, 30.0),
        }),
        hurt: rng.chance(0.03).then(|| Hurt { from: rng.vec(1.0, 0.0).normalize_or_zero(), damage: rng.range(1.0, 8.0) }),
        attack_landed: rng.chance(0.05),
        others,
        random: rng.next(),
        route: rng.chance(0.5).then(|| Route {
            next: rng.vec(6.0, 1.0),
            end: rng.vec(20.0, 3.0),
            length: rng.range(0.0, 40.0),
            reaches: rng.chance(0.7),
            hop: rng.chance(0.1),
        }),
        memory,
    }
}

/// An output with every float spelled out exactly: Rust prints the shortest text that reads
/// back as the same bits, so -0.0 and 0.0 differ.
fn exact(o: &BrainOutput) -> String {
    format!("{o:?}")
}

#[test]
fn wasm_matches_native_bit_for_bit() {
    let mut native = CowBrain::default();
    let mut wasm = wasm_cow();
    assert_eq!(native.tunables(), wasm.tunables());
    let mut rng = Rng(0x5eed_c0de);
    // A dozen cows, each carrying its own memory from think to think, as the engine does.
    let mut memories = vec![Vec::new(); 12];
    let (mut heaviest, mut total) = (0, 0);
    let (mut native_time, mut wasm_time) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
    let thinks = 20_000;
    for i in 0..thinks {
        if rng.chance(0.002) {
            let tunables = native.tunables();
            let t = &tunables[rng.next() as usize % tunables.len()];
            let value = rng.range(t.min, t.max);
            assert!(native.set_tunable(&t.name, value));
            assert!(wasm.set_tunable(&t.name, value));
        }
        let id = rng.next() % 12;
        let ctx = context(&mut rng, id, std::mem::take(&mut memories[id as usize]));
        let start = std::time::Instant::now();
        let a = native.think(&ctx);
        native_time += start.elapsed();
        let start = std::time::Instant::now();
        let b = wasm.think(&ctx);
        wasm_time += start.elapsed();
        assert_eq!(exact(&a), exact(&b), "think {i}");
        heaviest = heaviest.max(wasm.fuel_used());
        total += wasm.fuel_used();
        memories[id as usize] = a.memory;
    }
    assert_eq!(native.tunables(), wasm.tunables());
    // The memories got somewhere: cows changed mode along the way.
    assert!(memories.iter().all(|m| !m.is_empty()));

    let budget = Limits::default().fuel;
    println!("think fuel: heaviest {heaviest}, mean {} of {budget}", total / thinks);
    println!("think time: native {:?}, wasm {:?} each", native_time / thinks as u32, wasm_time / thinks as u32);
    assert!(heaviest > 0 && heaviest < budget / 10, "heaviest think used {heaviest} fuel of {budget}");
}

/// Runs the whole game with the cow brain in `brain` for a scripted minute: the player walks
/// into the pasture's herd and swings at the nearest cow now and then. Returns every cow's
/// position, facing, signal and memory after every tick.
fn play(brain: Where) -> Vec<String> {
    let mut sim = game::load(Code { movement: Where::Native, brain }).expect("the game loads");
    let tool = sim.tool_ids().next();
    let mut rng = Rng(0x00b0_c5e1);
    (0..3_600)
        .map(|tick| {
            let me = sim.player().unwrap().eye();
            let nearest = sim
                .targets()
                .iter()
                .map(|t| t.pos)
                .min_by(|a, b| me.delta_to(*a).length().total_cmp(&me.delta_to(*b).length()));
            let (yaw, pitch) = nearest.map_or((0.0, 0.0), |pos: WorldPos| {
                let d = me.delta_to(pos.translate(Vec3::Z));
                (d.y.atan2(d.x), d.z.atan2(d.truncate().length()))
            });
            let swing = tick % 90 == 0;
            sim.step(&PlayerInput {
                movement: api::Vec2::new(rng.range(-0.3, 0.3), 1.0),
                yaw,
                pitch,
                tool,
                use_pressed: swing,
                use_held: swing,
                jump_pressed: rng.chance(0.01),
                ..Default::default()
            });
            sim.targets()
                .iter()
                .map(|t| {
                    let (cell, offset) = (t.pos.cell(), t.pos.offset().to_array().map(f32::to_bits));
                    format!("{} {cell} {offset:?} {} {} {:?}", t.id, t.yaw.to_bits(), t.signal, t.memory)
                })
                .collect::<Vec<_>>()
                .join("; ")
        })
        .collect()
}

#[test]
fn the_herd_moves_the_same() {
    let (native, wasm) = (play(Where::Native), play(Where::Wasm));
    for (tick, (a, b)) in native.iter().zip(&wasm).enumerate() {
        assert_eq!(a, b, "tick {tick}");
    }
    // The script stirred the herd up, so more than grazing got compared.
    assert!(native.iter().any(|s| s.contains(ALARMED) || s.contains(ANGRY)), "nothing alarmed a cow");
}

#[test]
fn tunables_round_trip_through_wasm() {
    let mut brain = wasm_cow();
    for t in brain.tunables() {
        assert!(brain.set_tunable(&t.name, t.min), "{}", t.name);
    }
    assert!(brain.tunables().iter().all(|t| t.value == t.min));
    assert!(!brain.set_tunable("nonsense", 1.0));
}

fn hostile(behavior: f32) -> WasmBrain {
    let (name, mut brain) = load_brain(HOSTILE_WASM).expect("the component loads");
    assert_eq!(name, "test:hostile");
    assert!(brain.set_tunable("behavior", behavior));
    brain
}

/// Thinks three times with a brain that misbehaves and checks it stops for the right reason:
/// the creature stands still, facing as it was, and keeps its memory.
fn stops(behavior: f32, reason: Stop) {
    let brain = hostile(behavior);
    let ctx = context(&mut Rng(7), 3, vec![4.0, 5.0]);
    for _ in 0..3 {
        let out = brain.think(&ctx);
        assert_eq!(out, BrainOutput { yaw: ctx.yaw, clip_speed: 1.0, memory: ctx.memory.clone(), ..Default::default() });
    }
    assert_eq!(brain.stopped(), Some(reason));
    assert!(brain.tunables().is_empty());
}

#[test]
fn a_well_behaved_brain_keeps_running() {
    let brain = hostile(0.0);
    let mut memory = Vec::new();
    for _ in 0..3 {
        memory = brain.think(&context(&mut Rng(7), 3, memory)).memory;
    }
    assert_eq!(memory, vec![3.0], "the engine's memory carried the count");
    assert_eq!(brain.stopped(), None);
}

#[test]
fn a_trapping_brain_stops() {
    stops(2.0, Stop::Trap);
}

#[test]
fn a_looping_brain_runs_out_of_fuel() {
    stops(1.0, Stop::OutOfFuel);
}

/// The hostile brain registered in place of the cow's, so the base game's cows run on it.
struct HostileCow(WasmBrain);

impl Mod for HostileCow {
    fn name(&self) -> &str {
        "test"
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        r.register_brain(COW_BRAIN, Box::new(self.0.clone()))
    }
}

#[test]
fn the_game_carries_on_when_a_brain_stops() {
    let brain = hostile(0.0);
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mut sim =
        Simulation::with_mods(&[&data, &base::BaseMod, &HostileCow(brain.clone()), &base_movement::BaseMovementMod]).unwrap();
    sim.spawn_player().unwrap();
    let near = sim.spawn();
    sim.keep_targets(|pos| near.delta_to(pos).length() < 1000.0);
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
    assert!(sim.targets().iter().any(|t| t.memory == vec![30.0]), "cows think through the component");

    let mut handle = brain.clone();
    assert!(handle.set_tunable("behavior", 2.0));
    let before: Vec<_> = sim.targets().iter().map(|t| t.memory.clone()).collect();
    for _ in 0..120 {
        sim.step(&PlayerInput { movement: api::Vec2::Y, ..Default::default() });
    }
    assert_eq!(brain.stopped(), Some(Stop::Trap));
    assert!(sim.player().is_some(), "the game kept going");
    let after: Vec<_> = sim.targets().iter().map(|t| t.memory.clone()).collect();
    assert_eq!(before, after, "stopped cows keep their memory");
    assert!(sim.targets().iter().all(|t| t.walk == api::Vec2::ZERO), "and stand still");
}
