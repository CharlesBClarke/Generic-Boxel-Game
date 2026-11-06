//! Generated terrain with the base game's generator: filling it, generating it again after
//! tuning, lifting bodies out of it, and holding still what's beyond the simulation distance.

use api::{I64Vec3, Vec2, Vec3, WorldPos};
use sim::{PlayerInput, Simulation};

fn base() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mut sim = Simulation::with_mods(&[&data, &base::BaseMod, &base_cow::BaseCowMod, &base_movement::BaseMovementMod]).unwrap();
    sim.spawn_player().unwrap();
    sim
}

/// A base game with a small terrain, so tests stay quick.
fn small() -> Simulation {
    let mut sim = base();
    let w = sim.world_settings_mut();
    w.terrain_width = 4;
    w.terrain_height = 16;
    sim
}

fn generator(sim: &mut Simulation) -> &mut dyn api::Generator {
    let i = sim.generator_names().position(|n| n == base::terrain::HILLS).unwrap();
    sim.generator_mut(i).unwrap()
}

fn block_name(sim: &Simulation, x: i64, y: i64, z: i64) -> Option<String> {
    let pos = base::TERRAIN_CENTER.offset(I64Vec3::new(x, y, z));
    let id = sim.universe().main().block(pos);
    sim.blocks().get(id).map(|d| d.name.clone())
}

#[test]
fn the_terrain_is_generated_where_the_base_game_put_it() {
    let mut sim = small();
    let stats = sim.generate_terrain().unwrap();
    assert_eq!(stats.chunks, 4 * 4 * 16);
    let hills = base::terrain::Hills::default();
    let ground = hills.ground(5, 7);
    assert!(block_name(&sim, 5, 7, ground).is_some(), "the ground at {ground}");
    let above = hills.ground(5, 7).max(hills.ice_level as i64) + 1;
    // Clear above, unless a boulder sits there.
    let top = (above..above + 20).find(|&z| block_name(&sim, 5, 7, z).is_none()).unwrap();
    assert!(top - above < 10, "open sky over the ground");
    // The test area is untouched.
    assert_eq!(block_name(&sim, 5, 5 - base::TERRAIN_CENTER.0.y, -1).as_deref(), Some(base::GRASS));
}

#[test]
fn generating_again_follows_the_tuning_and_drops_chunks_out_of_reach() {
    let mut sim = small();
    sim.generate_terrain().unwrap();
    sim.take_changed_chunks();
    let before: Vec<_> = (0..20).map(|i| block_name(&sim, i * 3, -i * 2, 0)).collect();
    generator(&mut sim).set_tunable("seed", 7.0);
    sim.world_settings_mut().terrain_width = 2;
    let stats = sim.generate_terrain().unwrap();
    assert_eq!(stats.chunks, 2 * 2 * 16);
    let after: Vec<_> = (0..20).map(|i| block_name(&sim, i * 3, -i * 2, 0)).collect();
    assert_ne!(before, after, "another seed, other land");
    // Every chunk generated before, kept or dropped, is sent again.
    let changed = sim.take_changed_chunks();
    assert_eq!(changed.len(), 4 * 4 * 16);
    let far = base::TERRAIN_CENTER.offset(I64Vec3::new(-60, -60, 0)).chunk();
    assert!(sim.universe().main().chunk(far).is_none(), "out of reach now");
}

#[test]
fn generating_lifts_the_player_and_creatures_out_of_the_ground() {
    let mut sim = small();
    let deep = WorldPos::at_boxel(base::TERRAIN_CENTER.offset(I64Vec3::new(3, 3, -100)));
    sim.teleport_player(deep);
    sim.generate_terrain().unwrap();
    let feet = sim.player().unwrap().pos();
    let hills = base::terrain::Hills::default();
    assert!(feet.cell().z - base::TERRAIN_CENTER.0.z > hills.ground(3, 3), "on top, at {:?}", feet.cell());
    // Every creature stands in the open.
    let grid = sim.universe().main();
    for t in sim.targets().iter() {
        assert!(grid.cell(t.pos.boxel()).iter().all(|o| !sim.blocks().get(o.block).unwrap().is_full()), "{:?}", t.pos.cell());
    }
}

#[test]
fn teleporting_into_the_ground_comes_out_on_top() {
    let mut sim = small();
    sim.generate_terrain().unwrap();
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
fn creatures_and_items_beyond_the_simulation_distance_hold_still() {
    let mut sim = small();
    sim.generate_terrain().unwrap();
    // The test area's cows are near the player at spawn; the terrain's herds are 4 km off.
    sim.world_settings_mut().sim_distance = 200.0;
    let near = |p: WorldPos, s: &Simulation| s.player().unwrap().pos().delta_to(p).length() <= 200.0;
    // Give every creature a push, so the ones awake are sure to move.
    let start: Vec<(u64, WorldPos)> = sim.targets().iter().map(|t| (t.id, t.pos)).collect();
    let material = sim.blocks().material(base::DIRT).unwrap();
    let sky = base::terrain::Hills::default().surface(0, 0) + 20;
    let far = WorldPos::at_boxel(base::TERRAIN_CENTER.offset(I64Vec3::new(0, 0, sky)));
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
    // Within reach, it falls.
    sim.world_settings_mut().sim_distance = 10_000.0;
    for _ in 0..10 {
        sim.step(&input);
    }
    let item = sim.items().iter().find(|i| i.pos.cell().y > 3000).unwrap();
    assert!(far.delta_to(item.pos).z < 0.0, "falls once awake");
}

/// Times generating the full default terrain. Run with `--ignored --nocapture`.
#[test]
#[ignore]
fn time_the_full_terrain() {
    let mut sim = base();
    let stats = sim.generate_terrain().unwrap();
    println!("{} chunks, {} uniform, {:.2} s", stats.chunks, stats.uniform, stats.seconds);
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
