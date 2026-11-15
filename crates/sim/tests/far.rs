//! Far terrain with the base game's generator: built around the player out to the far
//! distance, following them, and with stored edits shrunk in.

use std::collections::{HashMap, HashSet};

use api::{ChunkPos, I64Vec3, LocalPos, Vec3, WorldPos};
use sim::far::{FarBuilt, FarChunk};
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
    sim
}

/// A spot `(x, y)` from the terrain's center, over the ground.
fn over(x: i64, y: i64) -> WorldPos {
    let ground = base::terrain::Hills::default().surface(x, y);
    WorldPos::new(base::TERRAIN_CENTER.0 + I64Vec3::new(x, y, ground + 2), Vec3::new(0.5, 0.5, 0.0))
}

/// Every far chunk sent and not dropped since, from the start.
fn gather(sim: &mut Simulation, have: &mut HashMap<FarChunk, FarBuilt>) {
    for gone in sim.take_far_unloaded() {
        have.remove(&gone);
    }
    for built in sim.take_far_loaded() {
        have.insert(built.chunk, built);
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
    let wanted: HashSet<FarChunk> = sim.reach().wanted(0, pos).into_iter().collect();
    assert!(!wanted.is_empty());
    let got: HashSet<FarChunk> = have.keys().copied().collect();
    assert_eq!(got, wanted, "every far chunk wanted, and only those");
    assert_eq!(sim.far_stats().waiting, 0);
    assert!(got.iter().all(|c| c.level >= 1));
    // Far chunks over and under the player are built too, not only around.
    assert!(got.iter().any(|c| c.nearest(pos) > 0.0 && c.pos.z * c.span() > pos.cell().z + 100), "some far over the player");
    assert!(got.iter().any(|c| (c.pos.z + 1) * c.span() < pos.cell().z - 100), "some far under the player");
    for (chunk, built) in &have {
        if let Some(cells) = &built.cells {
            assert_eq!(cells.data.pos.0, chunk.pos, "cells at their far chunk");
        }
    }
    // Over the highest ground around they're empty, deep under it solid, with ground between.
    let center_z = base::TERRAIN_CENTER.0.z;
    let hills = base::terrain::Hills::default();
    let ground: Vec<i64> = (-700..700).step_by(16).flat_map(|y| (-700..700).step_by(16).map(move |x| (x, y))).map(|(x, y)| hills.surface(x, y)).collect();
    let (peak, low) = (*ground.iter().max().unwrap(), *ground.iter().min().unwrap());
    let high = have.values().filter(|b| b.chunk.pos.z * b.chunk.span() > center_z + peak + 64);
    assert!(high.clone().count() > 0 && high.clone().all(|b| b.cells.is_none()), "sky is empty");
    let deep = have.values().filter(|b| (b.chunk.pos.z + 1) * b.chunk.span() < center_z + low - 64).collect::<Vec<_>>();
    assert!(!deep.is_empty() && deep.iter().all(|b| b.cells.as_ref().is_some_and(|c| c.data.cells.uniform().is_some_and(|e| e != 0))), "deep rock is solid");
    // Some surface cells are of two materials, and say so.
    let mixed = have.values().filter_map(|b| b.cells.as_ref()).filter(|c| (0..api::CHUNK_VOLUME).any(|i| c.mix.at(i).is_some())).count();
    assert!(mixed > 0, "some cells mixed");
}

#[test]
fn far_terrain_follows_the_player() {
    let mut sim = seeing(512.0);
    sim.teleport_player(over(0, 0));
    sim.generate_terrain().unwrap();
    sim.far_all();
    let mut have = HashMap::new();
    gather(&mut sim, &mut have);
    let before: HashSet<FarChunk> = have.keys().copied().collect();
    sim.teleport_player(over(3000, 0));
    sim.stream_all();
    sim.far_all();
    gather(&mut sim, &mut have);
    let pos = sim.player().unwrap().pos();
    let now: HashSet<FarChunk> = have.keys().copied().collect();
    let wanted: HashSet<FarChunk> = sim.reach().wanted(0, pos).into_iter().collect();
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
    // Near the player: the level-1 far chunks around are wanted.
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
    let chunk = FarChunk::holding(0, 1, dug);
    let built = have.get(&chunk).expect("the level-1 far chunk holding it");
    // The dug chunk is its lower half, in the cells over chunk X 3 and Y 128 (its east and
    // south halves).
    let deep = &built.cells.as_ref().expect("stone around it").data;
    assert_eq!(deep.pos.0.z, center.z / 2 - 4);
    let entry = |x: u8, y: u8, z: u8| deep.cells.get(LocalPos::new(x, y, z));
    assert_eq!(entry(16, 0, 0), 0, "dug out");
    assert_eq!(entry(31, 15, 15), 0, "dug out");
    assert_ne!(entry(15, 0, 0), 0, "the chunk beside it is stone");
    assert_ne!(entry(16, 0, 16), 0, "the chunk above it is stone");
}

#[test]
fn far_chunks_holding_an_edit_are_built_again_once_it_drops() {
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
    // Far away, the dug chunk drops and goes to the store.
    sim.teleport_player(over(150, 0));
    sim.stream_all();
    assert!(sim.universe().main().chunk(dug).is_none(), "dropped");
    sim.far_all();
    let again: Vec<FarChunk> = sim.take_far_loaded().into_iter().map(|b| b.chunk).collect();
    let holding: Vec<FarChunk> = again.iter().copied().filter(|c| c.holds(dug)).collect();
    assert!(!holding.is_empty(), "built again: {again:?}");
}

#[test]
fn far_terrain_is_as_tall_as_the_land() {
    let mut sim = seeing(1024.0);
    // Mountains well past where terrain used to stop, 256 boxels over its center.
    let hills = sim.generator_names().position(|n| n == base::terrain::HILLS).unwrap();
    assert!(sim.tune_generator(hills, "mountain_height", 500.0));
    sim.teleport_player(over(0, 0));
    sim.generate_terrain().unwrap();
    sim.far_all();
    assert!(sim.far_error().is_none(), "{:?}", sim.far_error());
    let mut have = HashMap::new();
    gather(&mut sim, &mut have);
    let center_z = base::TERRAIN_CENTER.0.z;
    let mut highest = i64::MIN;
    for built in have.values() {
        let (step, span) = (built.chunk.step(), built.chunk.span());
        if let Some(cells) = &built.cells {
            for i in 0..api::CHUNK_VOLUME {
                if cells.data.cells.at(i) != 0 {
                    highest = highest.max(built.chunk.pos.z * span + LocalPos::from_index(i).z as i64 * step + step / 2 - center_z);
                }
            }
        }
    }
    let tall = base::terrain::Hills { mountain_height: 500.0, ..base::terrain::Hills::default() };
    let peak = (-1000..1000).step_by(25).flat_map(|y| (-1000..1000).step_by(25).map(move |x| (x, y))).map(|(x, y)| tall.ground(x, y)).max().unwrap();
    assert!(peak > 300, "the land rises past the old limit within 1 km: {peak}");
    assert!(highest > 300, "and far terrain with it: {highest}");
}

#[test]
fn a_one_boxel_wall_shows_in_far_terrain_at_every_level() {
    let mut sim = seeing(2048.0);
    sim.teleport_player(over(0, 0));
    sim.generate_terrain().unwrap();
    // A chunk of air well over the ground 900 m east, with a stone wall one boxel thick, 4
    // tall and 8 long in it, stored as if built: far shorter than the cells out there.
    let center = base::TERRAIN_CENTER.0 / api::CHUNK_SIZE;
    let high = (base::terrain::Hills::default().surface(900, 0) + 200).div_euclid(api::CHUNK_SIZE);
    let pos = ChunkPos(center + I64Vec3::new(900 / api::CHUNK_SIZE, 0, high));
    let mut cells = sim::Layer::Uniform(0);
    for i in 0..api::CHUNK_VOLUME {
        let p = LocalPos::from_index(i);
        if p.x == 5 && p.y < 8 && p.z < 4 {
            cells.set_at(i, 1);
        }
    }
    let stone = (base::STONE.to_owned(), api::Orientation::IDENTITY);
    let saved = sim::save::SavedChunk { pos, lists: vec![Vec::new(), vec![stone]], cells, records: Vec::new() };
    sim.chunk_store().put(vec![(pos, saved.to_bytes())]);
    sim.reset_far();
    sim.far_all();
    assert!(sim.far_error().is_none(), "{:?}", sim.far_error());
    let mut have = HashMap::new();
    gather(&mut sim, &mut have);
    let mut levels = Vec::new();
    for (chunk, built) in &have {
        if !chunk.holds(pos) {
            continue;
        }
        let step = chunk.step();
        // The wall's boxel at the chunk's lowest corner, as a cell of this level.
        let boxel = pos.0 * api::CHUNK_SIZE + I64Vec3::new(5, 0, 0);
        let local = (boxel - chunk.pos * chunk.span()) / step;
        let cells = built.cells.as_ref().unwrap_or_else(|| panic!("level {}: the wall's far chunk has cells", chunk.level));
        let filled = cells.data.cells.get(LocalPos::new(local.x as u8, local.y as u8, local.z as u8)) != 0;
        assert!(filled, "level {}: the wall shows", chunk.level);
        levels.push(chunk.level);
    }
    levels.sort_unstable();
    // Built there out to 2 km: levels 3 to 5, each with cells taller than the wall. (The
    // coarsest far chunk holding it holds the player too, so it's always split and never
    // built; the unit tests in `sim::far` hold level 6.)
    assert!(levels.len() >= 3, "seen at levels {levels:?}");
}
