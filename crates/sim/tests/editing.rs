//! Breaking and placing blocks through the simulation, with the base mod loaded.

use api::{
    ApiError, BlockDef, BoxelPos, I64Vec3, InteractionDef, Mod, PlayerDef, Registrar, Vec3, WorldEdit, WorldPos,
};
use sim::{BlockId, Hit, PlayerInput, Simulation};

mod common;

fn base_sim() -> Simulation {
    let data = common::AllPlaceable::load();
    let mods: [&dyn api::Mod; 3] = [&data, &base::BaseMod, &base_movement::BaseMovementMod];
    let mut sim = Simulation::with_mods(&mods).expect("base mod loads");
    sim.spawn_player().expect("base registers a player");
    sim
}

fn settle(sim: &mut Simulation) {
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
}

fn block(sim: &Simulation, x: i64, y: i64, z: i64) -> BlockId {
    sim.universe().main().block(BoxelPos::new(x, y, z))
}

fn id(sim: &Simulation, name: &str) -> BlockId {
    sim.blocks().id(name).unwrap()
}

/// One tick with the break or place button freshly pressed.
fn break_at(yaw: f32, pitch: f32) -> PlayerInput {
    PlayerInput { yaw, pitch, use_pressed: true, use_held: true, ..Default::default() }
}

fn place_at(yaw: f32, pitch: f32, block: BlockId) -> PlayerInput {
    PlayerInput { yaw, pitch, alt_pressed: true, alt_held: true, place_block: block, ..Default::default() }
}

const DOWN: f32 = -89.0 * std::f32::consts::PI / 180.0;

/// A hit on the middle of a full boxel's face.
fn face(pos: BoxelPos, normal: I64Vec3, distance: f32) -> Hit {
    let point = Vec3::splat(0.5) + normal.as_vec3() * 0.5;
    Hit { pos, occupant: 0, normal, distance, point }
}

/// Where placing `block` on `hit` would put it, if that's allowed.
fn place_pos(sim: &Simulation, hit: &Hit, block: BlockId) -> Option<BoxelPos> {
    sim.placement(hit, block, 0.0, 0.0, false).filter(|t| t.allowed).map(|t| t.pos)
}

#[test]
fn break_then_place_puts_the_block_back() {
    // From spawn the marker post is 2.5 boxels east, 2 high.
    let mut sim = base_sim();
    settle(&mut sim);
    let marker = id(&sim, base::MARKER);
    assert_eq!(block(&sim, 3, 0, 1), marker);

    let hit = sim.target(0.0, 0.0).expect("the post is in reach");
    assert_eq!(hit.pos, BoxelPos::new(3, 0, 1));
    assert_eq!(hit.normal, I64Vec3::NEG_X);
    sim.step(&break_at(0.0, 0.0));
    assert!(block(&sim, 3, 0, 1).is_empty());
    assert_eq!(sim.target(0.0, 0.0), None, "nothing else in reach that way");

    // Aim at the top of the post's lower block and place stone on it.
    let eye = sim.player().unwrap().eye();
    let to_top = WorldPos::new(I64Vec3::new(3, 0, 1), Vec3::new(0.5, 0.5, 0.0));
    let aim = eye.delta_to(to_top);
    let pitch = aim.z.atan2(aim.truncate().length());
    let stone = id(&sim, base::STONE);
    let hit = sim.target(0.0, pitch).unwrap();
    assert_eq!((hit.pos, hit.normal), (BoxelPos::new(3, 0, 0), I64Vec3::Z));
    sim.step(&place_at(0.0, pitch, stone));
    assert_eq!(block(&sim, 3, 0, 1), stone);
}

#[test]
fn placing_at_the_feet_launches_the_player() {
    let mut sim = base_sim();
    settle(&mut sim);
    let hit = sim.target(0.0, DOWN).unwrap();
    assert_eq!((hit.pos, hit.normal), (BoxelPos::new(0, 0, -1), I64Vec3::Z));
    let stone = id(&sim, base::STONE);
    assert_eq!(place_pos(&sim, &hit, stone), Some(BoxelPos::new(0, 0, 0)));
    sim.step(&place_at(0.0, DOWN, stone));
    assert_eq!(block(&sim, 0, 0, 0), stone);
    let player = sim.player().unwrap();
    assert_eq!(player.pos().cell().z, 1, "lifted onto the block");
    assert!(player.velocity().z > 3.0, "and launched, at {}", player.velocity().z);
    assert!(player.velocity().truncate().length() < 1e-3, "straight up from the middle of the cell");
}

#[test]
fn placing_at_the_far_edge_launches_forward() {
    let mut sim = base_sim();
    sim.teleport_player(WorldPos::new(I64Vec3::new(10, -3, 0), Vec3::new(0.8, 0.5, 0.0)));
    settle(&mut sim);
    let stone = id(&sim, base::STONE);
    sim.step(&place_at(0.0, DOWN, stone));
    assert_eq!(block(&sim, 10, -3, 0), stone);
    let v = sim.player().unwrap().velocity();
    assert!(v.x > 1.0 && v.z > 3.0, "launched {v}");
}

#[test]
fn placing_into_a_filled_cell_is_refused() {
    let sim = base_sim();
    let stone = id(&sim, base::STONE);
    // The floor block below another floor block: its top neighbor is filled.
    let hit = face(BoxelPos::new(10, -3, -2), I64Vec3::Z, 1.0);
    assert_eq!(place_pos(&sim, &hit, stone), None);
    // A hit from inside a block has no face to place on.
    let inside = Hit { normal: I64Vec3::ZERO, distance: 0.0, ..face(BoxelPos::new(10, -3, -1), I64Vec3::Z, 0.0) };
    assert_eq!(place_pos(&sim, &inside, stone), None);
    let open = face(BoxelPos::new(10, -3, -1), I64Vec3::Z, 1.0);
    assert_eq!(place_pos(&sim, &open, stone), Some(BoxelPos::new(10, -3, 0)));
    assert_eq!(place_pos(&sim, &open, BlockId::EMPTY), None, "placing nothing does nothing");
}

#[test]
fn out_of_reach_does_nothing() {
    let mut sim = base_sim();
    settle(&mut sim);
    sim.interaction_def_mut().unwrap().reach = 2.0;
    assert_eq!(sim.target(0.0, 0.0), None);
    sim.step(&break_at(0.0, 0.0));
    assert_eq!(block(&sim, 3, 0, 1), id(&sim, base::MARKER));
    assert!(sim.take_changed_chunks().is_empty());
}

#[test]
fn holding_break_repeats_at_the_repeat_rate() {
    // Fly so breaking the floor doesn't drop the player, then dig straight down.
    let mut sim = base_sim();
    sim.step(&PlayerInput { toggle_fly: true, ..Default::default() });
    let mut input = break_at(0.0, DOWN);
    sim.step(&input);
    input.use_pressed = false;
    for _ in 0..30 {
        sim.step(&input);
    }
    // At 4 per second: the press, then after 15 and 30 ticks.
    assert!((-3..=-1).all(|z| block(&sim, 0, 0, z).is_empty()));
    assert!(!block(&sim, 0, 0, -4).is_empty());
}

#[test]
fn edits_report_changed_chunks() {
    let mut sim = base_sim();
    settle(&mut sim);
    assert!(sim.take_changed_chunks().is_empty(), "world building isn't reported");
    sim.step(&break_at(0.0, 0.0));
    // The post at y = 0 is on the chunk's south border, so the chunk south of it is told too.
    let mut changed = sim.take_changed_chunks();
    changed.sort_by_key(|c| c.0.to_array());
    assert_eq!(changed, [api::ChunkPos::new(0, -1, 0), api::ChunkPos::new(0, 0, 0)]);
    assert!(sim.take_changed_chunks().is_empty());
}

/// A mod with one block players may place and one they may not, a floor and a one-boxel shaft
/// two high at (10, 10). Registers no push-out rule.
struct Bedrock;

impl Mod for Bedrock {
    fn name(&self) -> &str {
        "test"
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        r.register_block(BlockDef::new("test:stone", true, [0, 0, 0]))?;
        r.register_block(BlockDef::new("test:bedrock", true, [0, 0, 0]).with_placeable(false))?;
        r.register_movement_controller(base_movement::QUAKE, Box::new(base_movement::QuakeController::default()))?;
        r.register_player(PlayerDef {
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            step_height: 0.0,
            controller: base_movement::QUAKE.into(),
            sounds: Default::default(),
        })?;
        r.register_interaction(InteractionDef { reach: 5.0, repeat_rate: 4.0 })
    }

    fn build_world(&self, w: &mut dyn WorldEdit) -> Result<(), ApiError> {
        w.fill(BoxelPos::new(-5, -5, -1), BoxelPos::new(15, 15, -1), "test:stone")?;
        w.fill(BoxelPos::new(9, 9, 0), BoxelPos::new(11, 11, 2), "test:stone")?;
        w.clear_block(BoxelPos::new(10, 10, 0));
        w.clear_block(BoxelPos::new(10, 10, 1));
        Ok(())
    }
}

/// `Bedrock` plus a push-out rule.
struct BedrockPush;

impl Mod for BedrockPush {
    fn name(&self) -> &str {
        "test"
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        Bedrock.register(r)?;
        r.register_push_out(api::PushOutDef { strength: 6.0, upward_bias: 1.0 })
    }

    fn build_world(&self, w: &mut dyn WorldEdit) -> Result<(), ApiError> {
        Bedrock.build_world(w)
    }
}

#[test]
fn placing_into_the_player_needs_a_push_out_rule_and_room() {
    let feet = face(BoxelPos::new(0, 0, -1), I64Vec3::Z, 1.6);
    let shaft = face(BoxelPos::new(10, 10, -1), I64Vec3::Z, 1.6);
    let open_spot = WorldPos::new(I64Vec3::ZERO, Vec3::new(0.5, 0.5, 0.0));
    let shaft_spot = WorldPos::new(I64Vec3::new(10, 10, 0), Vec3::new(0.5, 0.5, 0.0));

    for (m, rule) in [(&Bedrock as &dyn Mod, false), (&BedrockPush, true)] {
        let mut sim = Simulation::with_mods(&[m]).unwrap();
        sim.spawn_player().unwrap();
        let stone = sim.blocks().id("test:stone").unwrap();
        sim.teleport_player(open_spot);
        assert_eq!(place_pos(&sim, &feet, stone).is_some(), rule, "open ground, rule {rule}");
        sim.teleport_player(shaft_spot);
        assert_eq!(place_pos(&sim, &shaft, stone), None, "no room in the shaft, rule {rule}");
    }
}

#[test]
fn unplaceable_blocks_are_refused() {
    let sim = Simulation::with_mods(&[&Bedrock]).unwrap();
    let open = face(BoxelPos::new(3, 3, -1), I64Vec3::Z, 1.0);
    assert_eq!(place_pos(&sim, &open, sim.blocks().id("test:bedrock").unwrap()), None);
    assert!(place_pos(&sim, &open, sim.blocks().id("test:stone").unwrap()).is_some());
}

#[test]
fn a_second_interaction_def_is_an_error() {
    struct Twice;
    impl Mod for Twice {
        fn name(&self) -> &str {
            "twice"
        }
        fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
            r.register_interaction(InteractionDef { reach: 5.0, repeat_rate: 4.0 })?;
            r.register_interaction(InteractionDef { reach: 5.0, repeat_rate: 4.0 })
        }
    }
    assert!(matches!(
        Simulation::with_mods(&[&Twice]),
        Err(sim::SimError::Api(ApiError::DuplicateInteractionDef))
    ));

    struct Bad;
    impl Mod for Bad {
        fn name(&self) -> &str {
            "bad"
        }
        fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
            r.register_interaction(InteractionDef { reach: 0.0, repeat_rate: 4.0 })
        }
    }
    assert!(matches!(Simulation::with_mods(&[&Bad]), Err(sim::SimError::Api(ApiError::InvalidDef(_)))));
}
