//! Far terrain with the base game's generator: built around the player out to the far
//! distance, following them, and with stored edits shrunk in.

use std::collections::{HashMap, HashSet};

use api::{ChunkPos, I64Vec3, LocalPos, Vec3, WorldPos};
use sim::far::{FarChunks, FarColumn};
use sim::Simulation;

fn base() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mut sim = Simulation::with_mods(&[&data, &base::BaseMod, &base_cow::BaseCowMod, &base_movement::BaseMovementMod]).unwrap();
    sim.spawn_player().unwrap();
    sim
}

/// A base game seeing full detail to 32 m and far terrain to `far`.
fn seeing(far: f32) -> Simulation {
    let mut sim = base();
    sim.set_view_distance(32.0, far);
    sim.world_settings_mut().terrain_height = 16;
    sim
}

/// A spot `(x, y)` from the terrain's center, over the ground.
fn over(x: i64, y: i64) -> WorldPos {
    let ground = base::terrain::Hills::default().surface(x, y);
    WorldPos::new(base::TERRAIN_CENTER.0 + I64Vec3::new(x, y, ground + 2), Vec3::new(0.5, 0.5, 0.0))
}

/// Every far column sent and not dropped since, from the start.
fn gather(sim: &mut Simulation, have: &mut HashMap<FarColumn, FarChunks>) {
    for gone in sim.take_far_unloaded() {
        have.remove(&gone);
    }
    for built in sim.take_far_loaded() {
        have.insert(built.column, built);
    }
}

#[test]
fn far_terrain_is_built_out_to_the_far_distance() {
    let mut sim = seeing(512.0);
    sim.teleport_player(over(0, 0));
    sim.generate_terrain().unwrap();
    sim.stream_all();
    sim.far_all();
    assert!(sim.far_error().is_none(), "{:?}", sim.far_error());
    let mut have = HashMap::new();
    gather(&mut sim, &mut have);
    let pos = sim.player().unwrap().pos();
    let wanted: HashSet<FarColumn> = sim.reach().wanted(0, pos).into_iter().collect();
    assert!(!wanted.is_empty());
    let got: HashSet<FarColumn> = have.keys().copied().collect();
    assert_eq!(got, wanted, "every far column wanted, and only those");
    assert_eq!(sim.far_stats().waiting, 0);
    assert!(got.iter().all(|c| c.level >= 1));
    // The ground shows in every column: each has a chunk with something in it.
    for (column, built) in &have {
        assert!(!built.chunks.is_empty(), "{column:?} has ground");
        for chunk in &built.chunks {
            assert_eq!((chunk.pos.0.x, chunk.pos.0.y), (column.x, column.y));
        }
    }
}

#[test]
fn far_terrain_follows_the_player() {
    let mut sim = seeing(512.0);
    sim.teleport_player(over(0, 0));
    sim.generate_terrain().unwrap();
    sim.far_all();
    let mut have = HashMap::new();
    gather(&mut sim, &mut have);
    let before: HashSet<FarColumn> = have.keys().copied().collect();
    sim.teleport_player(over(3000, 0));
    sim.stream_all();
    sim.far_all();
    gather(&mut sim, &mut have);
    let pos = sim.player().unwrap().pos();
    let now: HashSet<FarColumn> = have.keys().copied().collect();
    let wanted: HashSet<FarColumn> = sim.reach().wanted(0, pos).into_iter().collect();
    assert!(wanted.is_subset(&now), "everything wanted where the player went");
    assert!(before.is_disjoint(&now), "what was left 3 km behind is dropped");
}

#[test]
fn without_far_distance_nothing_is_built() {
    let mut sim = seeing(0.0);
    sim.teleport_player(over(0, 0));
    sim.generate_terrain().unwrap();
    sim.far_all();
    assert!(sim.take_far_loaded().is_empty());
}

#[test]
fn stored_edits_are_shrunk_into_far_terrain() {
    let mut sim = seeing(512.0);
    // In the middle of a level-2 column, which splits: its level-1 columns are wanted.
    sim.teleport_player(over(64, 64));
    sim.generate_terrain().unwrap();
    // A deep chunk of solid stone just past what's loaded, dug out whole and stored as if
    // edited.
    let center = base::TERRAIN_CENTER.0 / api::CHUNK_SIZE;
    let dug = ChunkPos(center + I64Vec3::new(3, 0, -8));
    assert!(sim.universe().main().chunk(dug).is_none(), "not loaded");
    let saved = sim::save::SavedChunk { pos: dug, lists: vec![Vec::new()], cells: sim::Layer::Uniform(0), records: Vec::new() };
    sim.chunk_store().put(vec![(dug, saved.to_bytes())]);
    // Far terrain is built afresh, reading the store.
    sim.reset_far();
    sim.far_all();
    assert!(sim.far_error().is_none(), "{:?}", sim.far_error());
    let mut have = HashMap::new();
    gather(&mut sim, &mut have);
    let column = FarColumn::holding(0, 1, dug.0.x, dug.0.y);
    let built = have.get(&column).expect("the level-1 column over it");
    // Level-1 chunk z -4 holds ordinary rows -8 and -7; the dug chunk is its lower half, in
    // the cells over chunk X 3 and Y 128 (the column's east and south halves).
    let deep = built.chunks.iter().find(|c| c.pos.0.z == center.z / 2 - 4).expect("the deep chunk");
    let entry = |x: u8, y: u8, z: u8| deep.cells.get(LocalPos::new(x, y, z));
    assert_eq!(entry(16, 0, 0), 0, "dug out");
    assert_eq!(entry(31, 15, 15), 0, "dug out");
    assert_ne!(entry(15, 0, 0), 0, "the chunk beside it is stone");
    assert_ne!(entry(16, 0, 16), 0, "the chunk above it is stone");
}

#[test]
fn far_columns_over_an_edit_are_built_again_once_it_drops() {
    let mut sim = seeing(512.0);
    sim.teleport_player(over(0, 0));
    sim.generate_terrain().unwrap();
    sim.stream_all();
    // Let the player land, then dig out the boxel underfoot.
    for _ in 0..120 {
        sim.step(&sim::PlayerInput::default());
    }
    let below = api::BoxelPos(sim.player().unwrap().pos().cell() - I64Vec3::Z);
    let pitch = -89.0f32.to_radians();
    sim.step(&sim::PlayerInput { pitch, use_pressed: true, use_held: true, ..Default::default() });
    assert!(sim.universe().main().cell(below).is_empty(), "dug out");
    let dug = below.split().0;
    sim.far_all();
    sim.take_far_loaded();
    // Far away, the dug chunk's column drops and goes to the store.
    sim.teleport_player(over(150, 0));
    sim.stream_all();
    assert!(sim.universe().main().chunk(dug).is_none(), "dropped");
    sim.far_all();
    let again: Vec<FarColumn> = sim.take_far_loaded().into_iter().map(|b| b.column).collect();
    let over_it: Vec<FarColumn> = again.iter().copied().filter(|c| c.under().0.contains(&dug.0.x) && c.under().1.contains(&dug.0.y)).collect();
    assert!(!over_it.is_empty(), "built again: {again:?}");
}

#[test]
fn far_terrain_stops_at_the_terrain_height_like_full_detail() {
    let mut sim = seeing(1024.0);
    // Four chunks tall: 64 boxels above the terrain's center and 64 below, so the hills'
    // higher ground is cut off.
    sim.world_settings_mut().terrain_height = 4;
    sim.teleport_player(over(0, 0));
    sim.generate_terrain().unwrap();
    sim.far_all();
    let mut have = HashMap::new();
    gather(&mut sim, &mut have);
    let center_z = base::TERRAIN_CENTER.0.z;
    let mut cells = 0;
    for built in have.values() {
        let (step, span) = (built.column.step(), built.column.span());
        for chunk in &built.chunks {
            for i in 0..api::CHUNK_VOLUME {
                let middle = chunk.pos.0.z * span + LocalPos::from_index(i).z as i64 * step + step / 2 - center_z;
                if chunk.cells.at(i) != 0 {
                    cells += 1;
                    assert!((-64..64).contains(&middle), "{:?} cell at {middle}", built.column);
                }
            }
        }
    }
    assert!(cells > 0);
    // And there was ground above the limit to cut.
    let hills = base::terrain::Hills::default();
    let higher = (-1000..1000).step_by(50).flat_map(|y| (-1000..1000).step_by(50).map(move |x| (x, y))).any(|(x, y)| hills.ground(x, y) >= 64);
    assert!(higher, "the hills rise past 64 somewhere within 1 km");
}
