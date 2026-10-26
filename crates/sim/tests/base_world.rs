//! The engine with the base mod loaded, run headless.

use std::collections::HashSet;

use api::{ChunkPos, Vec3, WorldPos};
use glam::Vec2;
use sim::{PlayerInput, SimError, Simulation};

fn base_sim() -> Simulation {
    let mut sim = Simulation::with_mods(&[&base::BaseMod]).expect("base mod loads");
    sim.spawn_player().expect("base registers a player");
    sim
}

#[test]
fn engine_alone_has_no_content() {
    let mut sim = Simulation::new();
    assert!(sim.blocks().is_empty());
    assert!(sim.player_def().is_none());
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
