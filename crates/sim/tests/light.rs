//! Stored light in the base game: the lab's lamp lighting the floor in front of it when its
//! shutter opens, light coming back the same after a reload, and what lighting costs.

use api::{BoxelPos, CHUNK_VOLUME, Mod, Vec3, WorldPos};
use sim::{Light, PlayerInput, Simulation};

fn base_sim() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mods: [&dyn Mod; 4] = [&data, &base::BaseMod, &base_cow::BaseCowMod, &base_movement::BaseMovementMod];
    let mut sim = Simulation::with_mods(&mods).expect("base mod loads");
    sim.spawn_player().expect("base registers a player");
    sim
}

/// The base game with a small terrain, generated.
fn small_world() -> Simulation {
    let mut sim = base_sim();
    sim.set_view_distance(32.0, 0.0);
    sim.generate_terrain().unwrap();
    sim
}

/// A boxel of the lab, from its spot.
fn lab(dx: i64, dy: i64, dz: i64) -> BoxelPos {
    let (x, y, z) = base::LAB_SPOT;
    BoxelPos::new(x + dx, y + dy, z + dz)
}

/// Stands the feet in the lab's boxel `(dx, dy, 0)` and lets the body settle.
fn stand(sim: &mut Simulation, dx: i64, dy: i64) {
    sim.teleport_player(WorldPos::new(lab(dx, dy, 0).0, Vec3::new(0.5, 0.5, 0.0)));
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
}

/// Presses interact looking at `point` in boxel `cell`.
fn interact(sim: &mut Simulation, cell: BoxelPos, point: [f32; 3]) {
    let to = sim.player().unwrap().eye().delta_to(WorldPos::new(cell.0, Vec3::from_array(point)));
    let (yaw, pitch) = (to.y.atan2(to.x), to.z.atan2(to.truncate().length()));
    assert_eq!(sim.target(yaw, pitch).expect("in reach").pos, cell, "aimed at the right boxel");
    sim.step(&PlayerInput { yaw, pitch, interact_pressed: true, ..Default::default() });
}

/// Block light at `level`, `steps` boxels on.
fn away(level: u8, steps: usize) -> u8 {
    (0..steps).fold(level, |l, _| Light::passed_on(0, l))
}

fn light(sim: &Simulation, pos: BoxelPos) -> Light {
    sim.universe().main().light(pos)
}

/// Opens the lab lamp's shutter with the lever beside it.
fn open_shutter(sim: &mut Simulation) {
    interact(sim, lab(1, 4, 1), [0.5, 0.0, 0.5]);
}

#[test]
fn the_lab_lamp_lights_the_floor_in_front_only_with_its_shutter_open() {
    let mut sim = base_sim();
    stand(&mut sim, 1, 2);
    let front = lab(0, 3, 1);
    let shut = light(&sim, front).channel(0);
    // The lamp at the back of its niche, two boxels from here.
    open_shutter(&mut sim);
    assert_eq!(light(&sim, front).rgb(), [255, 200, 130].map(|l| away(l, 2)), "the lamp's own warm light");
    // Shut, some still gets out sideways through the lever's boxel beside the shutter, a hole
    // in the wall: four steps instead of two.
    assert_eq!(shut, away(255, 4), "shut, only what gets round");
    open_shutter(&mut sim);
    assert_eq!(light(&sim, front).channel(0), shut, "and shut again");
}

#[test]
fn the_open_ground_has_the_sky_and_the_lamps_light_the_lab() {
    let sim = small_world();
    let (x, y, _) = base::LAB_SPOT;
    // High above the test area nothing is in the way.
    assert_eq!(light(&sim, BoxelPos::new(x, y, 60)), Light::SKY);
    // The lumite on top of the lab's wall lights the boxel beside it.
    assert_eq!(light(&sim, lab(-2, 4, 3)).channel(0), away(255, 1));
}

#[test]
fn light_comes_back_the_same_after_a_reload() {
    let mut sim = small_world();
    stand(&mut sim, 1, 2);
    open_shutter(&mut sim);
    let save = sim.save().unwrap();
    let mut other = small_world();
    other.set_chunk_store(std::sync::Arc::clone(sim.chunk_store()));
    other.load(&save).unwrap();
    let (a, b) = (sim.universe().main(), other.universe().main());
    let mut chunks = 0;
    for (pos, chunk) in a.chunks() {
        let theirs = b.chunk(pos).expect("same chunks");
        for i in 0..CHUNK_VOLUME {
            assert_eq!(chunk.light.at(i), theirs.light.at(i), "chunk {pos:?}, boxel {i}");
        }
        chunks += 1;
    }
    assert!(chunks > 0);
    assert_eq!(light(&other, lab(0, 3, 1)).channel(0), away(255, 2), "the shutter stayed open, lit");
}

/// What lighting the default world costs: run with `--ignored --nocapture` to see.
#[test]
#[ignore]
fn lighting_cost() {
    let mut sim = base_sim();
    let at = WorldPos::at_boxel(base::TERRAIN_CENTER.offset(api::I64Vec3::new(0, 0, 100)));
    sim.teleport_player(at);
    let start = std::time::Instant::now();
    sim.generate_terrain().unwrap();
    sim.stream_all();
    let stats = sim.stream_stats();
    println!(
        "terrain: {} chunks in {:.2} s, {:.2} s on the workers (lit there), seams {:.2} s",
        stats.chunks,
        start.elapsed().as_secs_f32(),
        stats.seconds,
        stats.join_seconds
    );
    sim.teleport_player(sim.spawn());
    sim.stream_all();
    stand(&mut sim, 1, 2);
    sim.take_changed_chunks();
    open_shutter(&mut sim);
    let changed = sim.take_changed_chunks().len();
    println!("opening the shutter relit in {:.2} ms, {changed} chunks changed", sim.relight_seconds() * 1000.0);
    open_shutter(&mut sim);
    let changed = sim.take_changed_chunks().len();
    println!("closing it relit in {:.2} ms, {changed} chunks changed", sim.relight_seconds() * 1000.0);
}
