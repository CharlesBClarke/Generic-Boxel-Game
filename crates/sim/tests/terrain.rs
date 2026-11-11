//! Generated terrain with the base game's generator: streaming it around the player, generating
//! it again after tuning, keeping edits in dropped columns, lifting bodies out of it, and holding
//! still what's beyond the simulation distance or over ground still to come.

use api::{BoxelPos, ChunkPos, I64Vec3, Vec2, Vec3, WorldPos};
use sim::{Grid, PlayerInput, Simulation};

fn base() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mut sim = Simulation::with_mods(&[&data, &base::BaseMod, &base_cow::BaseCowMod, &base_movement::BaseMovementMod]).unwrap();
    sim.spawn_player().unwrap();
    sim
}

/// A base game that keeps little terrain loaded, so tests stay quick: 64 m around the player.
fn small() -> Simulation {
    let mut sim = base();
    sim.set_view_distance(32.0);
    sim.world_settings_mut().terrain_height = 16;
    sim
}

fn tune(sim: &mut Simulation, name: &str, value: f32) {
    let i = sim.generator_names().position(|n| n == base::terrain::HILLS).unwrap();
    assert!(sim.tune_generator(i, name, value));
}

fn block_name(sim: &Simulation, x: i64, y: i64, z: i64) -> Option<String> {
    let pos = base::TERRAIN_CENTER.offset(I64Vec3::new(x, y, z));
    let id = sim.universe().main().block(pos);
    sim.blocks().get(id).map(|d| d.name.clone())
}

/// A spot `(x, y)` from the terrain's center, `up` boxels over where the default tuning puts
/// the ground.
fn over(x: i64, y: i64, up: i64) -> WorldPos {
    let ground = base::terrain::Hills::default().surface(x, y);
    WorldPos::new(base::TERRAIN_CENTER.0 + I64Vec3::new(x, y, ground + up), Vec3::new(0.5, 0.5, 0.0))
}

/// Puts the player over the terrain's center and loads everything around them.
fn on_terrain(sim: &mut Simulation) {
    sim.teleport_player(over(0, 0, 2));
    sim.generate_terrain().unwrap();
    sim.stream_all();
}

fn terrain_chunks(sim: &Simulation) -> Vec<ChunkPos> {
    let mut chunks: Vec<ChunkPos> = sim.universe().main().chunks().map(|(p, _)| p).filter(|&p| sim.is_terrain(p)).collect();
    chunks.sort_by_key(|c| c.0.to_array());
    chunks
}

#[test]
fn the_terrain_loads_around_the_player_where_the_base_game_put_it() {
    let mut sim = small();
    on_terrain(&mut sim);
    let stats = sim.stream_stats();
    assert!(stats.columns >= 9, "the columns within 64 m: {}", stats.columns);
    assert_eq!(stats.waiting, 0);
    assert_eq!(terrain_chunks(&sim).len(), stats.columns * 16, "whole columns");
    let hills = base::terrain::Hills::default();
    let ground = hills.ground(5, 7);
    assert!(block_name(&sim, 5, 7, ground).is_some(), "the ground at {ground}");
    let above = hills.ground(5, 7).max(hills.ice_level as i64) + 1;
    // Clear above, unless a boulder sits there.
    let top = (above..above + 20).find(|&z| block_name(&sim, 5, 7, z).is_none()).unwrap();
    assert!(top - above < 10, "open sky over the ground");
    // The test area is untouched, and no terrain comes near it.
    assert_eq!(block_name(&sim, 5, 5 - base::TERRAIN_CENTER.0.y, -1).as_deref(), Some(base::GRASS));
    sim.teleport_player(sim.spawn());
    sim.stream_all();
    assert!(terrain_chunks(&sim).is_empty(), "the test area keeps terrain away");
}

#[test]
fn streamed_terrain_is_what_the_generator_makes_lit_as_if_made_at_once() {
    let mut sim = small();
    on_terrain(&mut sim);
    // Walk a little, so some columns came in later, beside ones already there.
    sim.teleport_player(over(70, -40, 30));
    sim.stream_all();
    let grid = sim.universe().main();
    let chunks = terrain_chunks(&sim);
    let def = api::TerrainDef { generator: base::terrain::HILLS.into(), center: base::TERRAIN_CENTER };
    let made = sim::terrain::fill_chunks(&base::terrain::Hills::default(), &def, &chunks, sim.blocks()).unwrap();
    let mut whole = Grid::default();
    for (pos, data) in chunks.iter().zip(&made) {
        let streamed = grid.export_chunk(*pos);
        assert_eq!(occupants(&streamed), occupants(data), "blocks in {pos:?}");
        whole.import_chunk(&streamed);
    }
    // Lighting what's loaded all at once, the built world beside it included, gives the same light.
    for (pos, _) in grid.chunks().filter(|(p, _)| !sim.is_terrain(*p)) {
        whole.import_chunk(&grid.export_chunk(pos));
    }
    whole.relight_all(sim.blocks());
    for pos in &chunks {
        assert_eq!(grid.chunk(*pos).unwrap().light, whole.chunk(*pos).unwrap().light, "light in {pos:?}");
    }
}

/// Each boxel's occupants in `data`, however its lists are numbered.
fn occupants(data: &sim::ChunkData) -> Vec<&[sim::Occupant]> {
    match &data.cells {
        sim::Layer::Uniform(i) => vec![&data.lists[*i as usize]],
        sim::Layer::Dense(cells) => cells.iter().map(|&i| data.lists[i as usize].as_slice()).collect(),
    }
}

/// The top of the ground at a few spots near the terrain's center.
fn tops(sim: &Simulation) -> Vec<Option<i64>> {
    (0..12).map(|i| (-200..200).rev().find(|&z| block_name(sim, i * 4 - 20, 10 - i * 3, z).is_some())).collect()
}

#[test]
fn columns_left_behind_are_dropped_and_ones_ahead_load() {
    let mut sim = small();
    on_terrain(&mut sim);
    sim.take_unloaded_chunks();
    let first = sim.take_loaded_chunks();
    assert_eq!(first.len(), terrain_chunks(&sim).len());
    let start = terrain_chunks(&sim);
    sim.teleport_player(over(300, 0, 20));
    sim.stream_all();
    let dropped = sim.take_unloaded_chunks();
    let loaded = sim.take_loaded_chunks();
    assert_eq!(dropped.len(), start.len(), "nothing near the start is within reach");
    assert!(dropped.iter().all(|c| start.contains(c)));
    assert!(dropped.iter().all(|c| sim.universe().main().chunk(*c).is_none()), "dropped from the world");
    assert_eq!(loaded.len(), terrain_chunks(&sim).len());
    assert!(block_name(&sim, 300, 0, base::terrain::Hills::default().ground(300, 0)).is_some());
}

#[test]
fn the_load_distance_follows_the_view_distance() {
    let mut sim = small();
    on_terrain(&mut sim);
    let near = sim.stream_stats().columns;
    sim.set_view_distance(96.0);
    assert_eq!(sim.load_distance(), 128.0);
    sim.stream_all();
    let far = sim.stream_stats().columns;
    assert!(far > 3 * near, "{near} columns at 64 m, {far} at 128 m");
    sim.set_view_distance(32.0);
    sim.stream_all();
    // Down to the radius and its margin, so a little more than at first.
    let back = sim.stream_stats().columns;
    assert!(back < far && back >= near, "{back}");
}

#[test]
fn an_edit_survives_its_column_being_dropped() {
    let mut sim = small();
    on_terrain(&mut sim);
    settle(&mut sim);
    let below = BoxelPos(sim.player().unwrap().pos().cell() - I64Vec3::Z);
    assert!(!sim.universe().main().cell(below).is_empty(), "standing on the ground");
    let pitch = -89.0f32.to_radians();
    sim.step(&PlayerInput { pitch, use_pressed: true, use_held: true, ..Default::default() });
    assert!(sim.universe().main().cell(below).is_empty(), "dug out");
    let dug = below.split().0;
    sim.teleport_player(over(400, 0, 20));
    sim.stream_all();
    assert!(sim.universe().main().chunk(dug).is_none(), "dropped");
    assert!(sim.save().chunks.iter().any(|c| c.pos == dug), "still saved while dropped");
    sim.teleport_player(WorldPos::at_boxel(below.offset(I64Vec3::new(3, 3, 40))));
    sim.stream_all();
    assert!(sim.universe().main().cell(below).is_empty(), "comes back dug");
    assert!(sim.edited_chunks().contains(&dug), "still edited");
    // Made again from scratch, it's filled in.
    sim.generate_terrain().unwrap();
    sim.stream_all();
    assert!(!sim.universe().main().cell(below).is_empty(), "regenerated");
}

fn settle(sim: &mut Simulation) {
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
}

#[test]
fn generating_again_follows_the_tuning() {
    let mut sim = small();
    on_terrain(&mut sim);
    sim.take_loaded_chunks();
    let before = tops(&sim);
    assert!(before.iter().all(Option::is_some), "{before:?}");
    let loaded = terrain_chunks(&sim);
    tune(&mut sim, "seed", 7.0);
    sim.generate_terrain().unwrap();
    sim.stream_all();
    let after = tops(&sim);
    assert_ne!(before, after, "another seed, other land");
    // Every chunk is dropped and comes in again.
    let mut dropped = sim.take_unloaded_chunks();
    dropped.sort_by_key(|c| c.0.to_array());
    dropped.dedup();
    assert!(loaded.iter().all(|c| dropped.contains(c)));
    assert_eq!(sim.take_loaded_chunks().len(), terrain_chunks(&sim).len());
}

#[test]
fn the_player_waits_over_ground_still_to_come_and_lands_on_top() {
    let mut sim = small();
    let deep = WorldPos::at_boxel(base::TERRAIN_CENTER.offset(I64Vec3::new(3, 3, -100)));
    sim.teleport_player(deep);
    assert!(sim.awaiting_ground(deep));
    sim.generate_terrain().unwrap();
    assert!(!sim.awaiting_ground(sim.player().unwrap().pos()));
    let feet = sim.player().unwrap().pos();
    let hills = base::terrain::Hills::default();
    assert!(feet.cell().z - base::TERRAIN_CENTER.0.z > hills.ground(3, 3), "on top, at {:?}", feet.cell());
    sim.stream_all();
    // Every creature on loaded ground stands in the open.
    let grid = sim.universe().main();
    for t in sim.targets().iter().filter(|t| !sim.awaiting_ground(t.pos)) {
        assert!(grid.cell(t.pos.boxel()).iter().all(|o| !sim.blocks().get(o.block).unwrap().is_full()), "{:?}", t.pos.cell());
    }
    // Held still over ground still to come, however long it takes.
    let far = over(5000, 0, 10);
    sim.teleport_player(far);
    for _ in 0..3 {
        sim.step(&PlayerInput::default());
    }
    if sim.awaiting_ground(sim.player().unwrap().pos()) {
        assert_eq!(sim.player().unwrap().pos(), far, "held");
    }
}

#[test]
fn teleporting_into_the_ground_comes_out_on_top() {
    let mut sim = small();
    on_terrain(&mut sim);
    sim.teleport_player(WorldPos::at_boxel(base::TERRAIN_CENTER.offset(I64Vec3::new(0, 0, -200))));
    let z = sim.player().unwrap().pos().cell().z - base::TERRAIN_CENTER.0.z;
    let hills = base::terrain::Hills::default();
    assert!(z > hills.ground(0, 0), "out at {z}");
    // A teleport into the open isn't moved.
    let open = WorldPos::new(base::TERRAIN_CENTER.0 + I64Vec3::new(0, 0, 300), Vec3::new(0.5, 0.5, 0.25));
    sim.teleport_player(open);
    assert_eq!(sim.player().unwrap().pos(), open);
}

#[test]
fn creatures_and_items_beyond_the_simulation_distance_or_over_ground_to_come_hold_still() {
    let mut sim = small();
    // The test area's cows are near the player at spawn; the terrain's herds are 4 km off.
    sim.world_settings_mut().sim_distance = 200.0;
    let near = |p: WorldPos, s: &Simulation| s.player().unwrap().pos().delta_to(p).length() <= 200.0;
    // Give every creature a push, so the ones awake are sure to move.
    let start: Vec<(u64, WorldPos)> = sim.targets().iter().map(|t| (t.id, t.pos)).collect();
    let material = sim.blocks().material(base::DIRT).unwrap();
    let far = over(0, 0, 20);
    sim.spawn_item(material, 1, far);
    let input = PlayerInput { movement: Vec2::ZERO, ..Default::default() };
    for _ in 0..60 {
        sim.step(&input);
    }
    let mut asleep = 0;
    for (id, pos) in &start {
        let t = sim.targets().iter().find(|t| t.id == *id).unwrap();
        if !near(*pos, &sim) {
            asleep += 1;
            assert_eq!(t.pos, *pos, "cow {id} far off held still");
            assert_eq!(t.prev_pos, t.pos, "with no blend");
        }
    }
    assert!(asleep >= 10, "the herds on the terrain are far: {asleep}");
    let item = sim.items().iter().find(|i| i.pos.cell().y > 3000).expect("the far item");
    assert_eq!(item.pos, far, "the far item hangs where it was dropped");
    // Within reach, but over ground that isn't there yet, it still hangs.
    sim.world_settings_mut().sim_distance = 10_000.0;
    for _ in 0..10 {
        sim.step(&input);
    }
    let item = sim.items().iter().find(|i| i.pos.cell().y > 3000).unwrap();
    assert_eq!(item.pos, far, "waits for its ground");
    // Once the ground under it is in, it falls.
    sim.teleport_player(over(10, 10, 5));
    sim.stream_all();
    for _ in 0..10 {
        sim.step(&input);
    }
    let item = sim.items().iter().find(|i| i.pos.cell().y > 3000).unwrap();
    assert!(far.delta_to(item.pos).z < 0.0, "falls once its ground is in");
}

/// Times streaming in the default load distance of terrain. Run with `--ignored --nocapture`.
#[test]
#[ignore]
fn time_the_full_terrain() {
    let mut sim = base();
    sim.teleport_player(over(0, 0, 2));
    let start = std::time::Instant::now();
    sim.generate_terrain().unwrap();
    let near = start.elapsed().as_secs_f32();
    sim.stream_all();
    let stats = sim.stream_stats();
    println!(
        "{} columns in {:.2} s (the ground near in {near:.2} s); {:.2} s generating on the workers, {:.2} s joining",
        stats.columns,
        start.elapsed().as_secs_f32(),
        stats.seconds,
        stats.join_seconds
    );
}

#[test]
fn keeping_only_some_creatures_drops_the_rest_for_good() {
    let mut sim = base();
    let all = sim.targets().len();
    let herds: usize = base::TERRAIN_HERDS.iter().map(|h| h.2).sum();
    assert_eq!(all, base::COWS.len() + herds);
    let spawn = sim.spawn();
    sim.keep_targets(|pos| spawn.delta_to(pos).length() < 1000.0);
    assert_eq!(sim.targets().len(), base::COWS.len());
    sim.respawn_targets();
    assert_eq!(sim.targets().len(), base::COWS.len(), "respawning doesn't bring them back");
    assert!(sim.targets().iter().all(|t| spawn.delta_to(t.pos).length() < 1000.0));
}
