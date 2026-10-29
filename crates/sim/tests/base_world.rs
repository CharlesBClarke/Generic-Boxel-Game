//! The engine with the base mod loaded, run headless.

use std::collections::HashSet;

use api::{ChunkPos, Vec3, WorldPos};
use glam::Vec2;
use sim::{PlayerInput, SimError, Simulation};

fn base_sim() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mods: [&dyn api::Mod; 3] = [&data, &base::BaseMod, &base_movement::BaseMovementMod];
    let mut sim = Simulation::with_mods(&mods).expect("base mod loads");
    sim.spawn_player().expect("base registers a player");
    sim
}

#[test]
fn engine_alone_has_no_content() {
    let mut sim = Simulation::new();
    assert!(sim.blocks().is_empty());
    assert!(sim.player_def().is_none());
    assert!(sim.interaction_def().is_none());
    assert_eq!(sim.universe().main().chunks().count(), 0);
    assert_eq!(sim.spawn_player(), Err(SimError::NoPlayerDef));
}

#[test]
fn base_registers_blocks_by_name() {
    let sim = base_sim();
    for name in [base::STONE, base::DIRT, base::GRASS, base::BRICK, base::MARKER, base::ICE] {
        assert!(sim.blocks().id(name).is_some(), "{name} missing");
    }
}

#[test]
fn base_blocks_come_from_its_data_files() {
    let sim = base_sim();
    let def = |name| sim.blocks().get(sim.blocks().id(name).unwrap()).unwrap();
    // The blocks the code half used to register, now with textures, then the partial ones.
    let names: Vec<_> = sim.blocks().iter().map(|(_, d)| d.name.as_str()).collect();
    assert_eq!(
        names,
        [
            base::STONE,
            base::DIRT,
            base::GRASS,
            base::BRICK,
            base::MARKER,
            base::ICE,
            base::STONE_SLAB,
            base::STONE_STAIRS,
            base::BRICK_SLAB,
            base::BRICK_STAIRS,
            base::WOOD_FENCE,
            base::LOG,
        ]
    );
    for (_, d) in sim.blocks().iter() {
        assert!(d.solid && d.placeable, "{}", d.name);
        let friction = if d.name == base::ICE { 0.1 } else { 1.0 };
        assert_eq!(d.friction, friction, "{}", d.name);
    }
    let grass = def(base::GRASS).textures.as_ref().unwrap();
    let faces = (grass.top.as_str(), grass.bottom.as_str(), grass.side.as_str());
    assert_eq!(faces, ("base:grass", "base:dirt", "base:grass_side"));
    assert_eq!(def(base::MARKER).textures, None, "the marker stays a flat color");
    assert_eq!(def(base::MARKER).color, [240, 180, 40]);
    use api::{BlockBox, Placement};
    let slab = [BlockBox::new([0, 0, 0], [16, 16, 8])];
    for (name, boxes, placement) in [
        (base::STONE_SLAB, 1, Placement::Half),
        (base::BRICK_SLAB, 1, Placement::Half),
        (base::STONE_STAIRS, 2, Placement::FacingAway),
        (base::BRICK_STAIRS, 2, Placement::FacingAway),
        (base::WOOD_FENCE, 1, Placement::Fixed),
        (base::LOG, 1, Placement::AlongFace),
        (base::STONE, 1, Placement::Fixed),
    ] {
        assert_eq!((def(name).shape.len(), def(name).placement), (boxes, placement), "{name}");
    }
    assert_eq!(def(base::STONE_SLAB).shape, slab);
    assert!(def(base::LOG).is_full() && !def(base::WOOD_FENCE).is_full());
    for name in [
        "base:stone",
        "base:dirt",
        "base:grass",
        "base:grass_side",
        "base:brick",
        "base:ice",
        "base:wood",
        "base:log",
        "base:log_top",
    ] {
        let t = sim.textures().get(name).unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!((t.width, t.height), (16, 16), "{name}");
        assert!(t.rgba.as_chunks::<4>().0.iter().all(|px| px[3] == 255), "{name} should be opaque");
    }
}

#[test]
fn test_area_spans_several_chunks() {
    let sim = base_sim();
    let near: HashSet<ChunkPos> = sim
        .universe()
        .main()
        .chunks()
        .map(|(pos, _)| pos)
        .filter(|pos| pos.0.x.abs() < 100 && pos.0.y.abs() < 100)
        .collect();
    let columns: HashSet<_> = near.iter().map(|p| (p.0.x, p.0.y)).collect();
    let layers: HashSet<_> = near.iter().map(|p| p.0.z).collect();
    assert!(columns.len() >= 16, "only {} chunk columns", columns.len());
    assert!(layers.len() >= 3, "only {} chunk layers", layers.len());
}

#[test]
fn player_spawns_and_settles_on_the_floor() {
    let mut sim = base_sim();
    for _ in 0..60 {
        sim.step(&PlayerInput::default());
    }
    let player = sim.player().unwrap();
    assert!(player.on_ground());
    assert!(sim.spawn().delta_to(player.pos()).length() < 1e-3);
    assert_eq!(sim.tick(), 60);
}

/// A scripted walk: north to line up with the steps, stop, then hop east up them, then turn.
/// Movement keeps momentum, so the walk has to stop before changing direction.
fn script() -> Vec<PlayerInput> {
    let mut inputs = Vec::new();
    let walk = |yaw: f32, jump: bool| PlayerInput { movement: Vec2::Y, yaw, jump_held: jump, ..Default::default() };
    inputs.extend(std::iter::repeat_n(PlayerInput::default(), 30));
    inputs.extend(std::iter::repeat_n(walk(std::f32::consts::FRAC_PI_2, false), 55));
    inputs.extend(std::iter::repeat_n(PlayerInput::default(), 30));
    inputs.extend(std::iter::repeat_n(walk(0.0, true), 300));
    inputs.extend(std::iter::repeat_n(walk(2.5, false), 120));
    inputs
}

fn run_from(start: WorldPos) -> Vec<(api::I64Vec3, Vec3)> {
    let mut sim = base_sim();
    sim.teleport_player(start);
    script()
        .iter()
        .map(|input| {
            sim.step(input);
            let pos = sim.player().unwrap().pos();
            (pos.cell() - start.cell(), pos.offset())
        })
        .collect()
}

#[test]
fn walking_in_the_millions_matches_walking_near_zero() {
    let sim = base_sim();
    let waypoint = |name: &str| sim.waypoints().iter().find(|(n, _)| n == name).unwrap().1;
    let near = run_from(waypoint("origin"));
    let far = run_from(waypoint("far"));
    assert!(waypoint("far").cell().x >= 1_000_000);
    // The walk has to actually climb, or it proves little.
    assert!(near.iter().any(|(cell, _)| cell.z >= 3), "the script never climbed the steps");
    assert_eq!(near, far);
}

#[test]
fn holding_jump_climbs_the_steps() {
    // Like Minecraft: hold forward and jump at the foot of the steps and keep climbing,
    // without stalling against each riser.
    let mut sim = base_sim();
    sim.teleport_player(WorldPos::new(api::I64Vec3::new(4, 5, 0), Vec3::new(0.5, 0.5, 0.0)));
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
    let climb = PlayerInput { movement: Vec2::Y, yaw: 0.0, jump_held: true, ..Default::default() };
    let mut highest = 0.0f32;
    for _ in 0..300 {
        sim.step(&climb);
        highest = highest.max(sim.spawn().delta_to(sim.player().unwrap().pos()).z);
    }
    assert!(highest >= 9.9, "only climbed to {highest}");
}

/// Runs up for `run_up` ticks facing east from `start`, jumps, then keeps going for 90 ticks.
/// `crouch_run_up` crouches during the run-up (charging the jump); `tuck` crouches after the
/// jump. Returns whether the player ever stood on top of the ledge `ledge_height` high.
fn ledge_attempt(start: WorldPos, run_up: usize, crouch_run_up: bool, tuck: bool, ledge_height: i64) -> bool {
    let mut sim = base_sim();
    sim.teleport_player(start);
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
    let forward = |crouch: bool| PlayerInput { movement: Vec2::Y, crouch, ..Default::default() };
    for _ in 0..run_up {
        sim.step(&forward(crouch_run_up));
    }
    sim.step(&PlayerInput { jump_pressed: true, jump_held: true, ..forward(crouch_run_up) });
    (0..90).any(|_| {
        sim.step(&forward(tuck));
        let player = sim.player().unwrap();
        let cell = player.pos().cell();
        player.on_ground() && cell.z == ledge_height && (26..=29).contains(&cell.x)
    })
}

/// Which run-up lengths, every `step` ticks up to `max`, get the player onto the ledge.
fn clearing_run_ups(start: WorldPos, max: usize, crouch_run_up: bool, tuck: bool, ledge_height: i64) -> Vec<usize> {
    (0..=max).step_by(3).filter(|&n| ledge_attempt(start, n, crouch_run_up, tuck, ledge_height)).collect()
}

#[test]
fn a_tuck_clears_a_ledge_a_plain_jump_cannot() {
    let start = WorldPos::new(api::I64Vec3::new(20, -29, 0), Vec3::new(0.5, 0.5, 0.0));
    let plain = clearing_run_ups(start, 75, false, false, 2);
    assert!(plain.is_empty(), "a plain jump made it with run-ups {plain:?}");
    let tucked = clearing_run_ups(start, 75, false, true, 2);
    assert!(!tucked.is_empty(), "no run-up let a tuck clear the 2-high ledge");
}

#[test]
fn a_charged_tuck_clears_a_higher_ledge() {
    let start = WorldPos::new(api::I64Vec3::new(22, -35, 0), Vec3::new(0.5, 0.5, 0.0));
    let tuck_only = clearing_run_ups(start, 150, false, true, 3);
    assert!(tuck_only.is_empty(), "an uncharged tuck made it with run-ups {tuck_only:?}");
    let charged = clearing_run_ups(start, 150, true, true, 3);
    assert!(!charged.is_empty(), "no charged run-up cleared the 3-high ledge");
}

#[test]
fn landmarks_are_standing_spots() {
    // Every landmark should be an empty two-boxel space over something solid, in both copies.
    let sim = base_sim();
    let grid = sim.universe().main();
    let landmarks: Vec<_> = sim.waypoints().iter().filter(|(name, _)| name.starts_with("landmark:")).collect();
    assert_eq!(landmarks.len(), 2 * base::LANDMARKS.len());
    for (name, pos) in landmarks {
        let feet = pos.boxel();
        assert!(grid.block(feet).is_empty() && grid.block(feet.offset(api::I64Vec3::Z)).is_empty(), "{name} is buried");
        assert!(!grid.block(feet.offset(api::I64Vec3::NEG_Z)).is_empty(), "{name} floats");
    }
}
