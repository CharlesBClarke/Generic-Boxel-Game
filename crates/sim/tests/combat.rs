//! Hitting targets with a tool through the simulation: the base mod's cows and club, and a small
//! walled room of this test's own.

use api::{
    ApiError, BoxelPos, I64Vec3, InteractionDef, Mod, ModelDef, PlayerDef, Registrar, TargetDef, Texture, ToolDef, Vec3, WorldEdit,
    WorldPos,
};
use sim::{PlayerInput, Simulation, Strike, SwingStage};

fn base_sim() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mut sim = Simulation::with_mods(&[&data, &base::BaseMod, &base_movement::BaseMovementMod]).expect("base mod loads");
    sim.spawn_player().unwrap();
    sim
}

/// The base game with its cows' brains off, so they stand still as dummies.
fn dummy_sim() -> Simulation {
    let mut sim = base_sim();
    sim.set_brains_on(false);
    sim
}

fn idle(sim: &mut Simulation, ticks: usize) {
    for _ in 0..ticks {
        sim.step(&PlayerInput::default());
    }
}

/// The yaw and pitch that look from the player's eyes at `pos`.
fn aim_at(sim: &Simulation, pos: WorldPos) -> (f32, f32) {
    let d = sim.player().unwrap().eye().delta_to(pos);
    (d.y.atan2(d.x), d.z.atan2(d.truncate().length()))
}

/// Swings the first tool at `pos` until the strike lands, and returns it.
fn swing_at(sim: &mut Simulation, pos: WorldPos) -> Strike {
    let tool = sim.tool_ids().next();
    let (yaw, pitch) = aim_at(sim, pos);
    let mut input = PlayerInput { yaw, pitch, tool, use_pressed: true, use_held: true, ..Default::default() };
    for _ in 0..60 {
        sim.step(&input);
        input.use_pressed = false;
        if let Some(strike) = sim.take_strikes().pop() {
            return strike;
        }
    }
    panic!("no strike within a second");
}

#[test]
fn base_places_its_cows_in_the_pen_where_they_settle() {
    let mut sim = dummy_sim();
    let cows: Vec<_> = sim.targets().iter().filter(|t| sim.target_def(t.kind).name == base::COW).collect();
    assert_eq!(cows.len(), base::COWS.len(), "one cow per spot, in the near test area only");
    let model = &sim.target_def(cows[0].kind).model;
    assert!(sim.models().iter().any(|m| &m.name == model), "the cow's model is registered");
    idle(&mut sim, 120);
    for cow in sim.targets().iter() {
        assert!(cow.on_ground && cow.velocity == Vec3::ZERO, "settled");
        let pen = BoxelPos::new(-14, 14, 0);
        assert!(WorldPos::at_boxel(pen).delta_to(cow.pos).truncate().length() < 8.0, "in the pen");
    }
}

#[test]
fn the_club_strikes_after_its_windup_and_knocks_a_cow_away() {
    let mut sim = base_sim();
    let cow = sim.targets().iter().next().unwrap().clone();
    let def = sim.target_def(cow.kind).clone();
    sim.teleport_player(cow.pos.translate(Vec3::new(-2.0, 0.0, 0.0)));
    idle(&mut sim, 30);
    let club = sim.tool(sim.tool_ids().next().unwrap()).clone();
    let start = sim.tick();
    let strike = swing_at(&mut sim, cow.pos.translate(Vec3::Z * def.height * 0.5));
    let windup_ticks = (club.windup / Simulation::TICK_SECONDS).ceil() as u64;
    assert!(strike.tick - start + 1 >= windup_ticks, "struck after the windup, on tick {}", strike.tick - start);
    let hit = strike.hit.expect("the cow is in reach");
    assert_eq!(hit.target, cow.id);
    assert!(matches!(sim.swing(), Some(s) if s.stage == SwingStage::Recover), "recovering after the strike");
    let after = sim.targets().iter().find(|t| t.id == cow.id).unwrap();
    assert_eq!(hit.zone.map(|z| def.hit_boxes[z].name.as_str()), Some("body"));
    assert_eq!(after.health, def.health - club.damage, "standing still, a body hit does the club's damage");
    assert!(after.velocity.x > 0.0 && after.velocity.z > 0.0, "knocked east and up: {}", after.velocity);
}

#[test]
fn enough_hits_break_a_cow_and_it_comes_back() {
    let mut sim = dummy_sim();
    let cow = sim.targets().iter().next().unwrap().clone();
    let def = sim.target_def(cow.kind).clone();
    let mut broke = false;
    for _ in 0..20 {
        let Some(now) = sim.targets().iter().find(|t| t.id == cow.id).cloned() else {
            break;
        };
        // Step up beside it each time, wherever it was knocked.
        sim.teleport_player(now.pos.translate(Vec3::new(-1.8, 0.0, 0.0)));
        idle(&mut sim, 40);
        let now = sim.targets().iter().find(|t| t.id == cow.id).unwrap().clone();
        if let Some(hit) = swing_at(&mut sim, now.pos.translate(Vec3::Z * def.height * 0.5)).hit {
            broke |= hit.broke && hit.target == cow.id;
        }
    }
    assert!(broke, "the cow broke");
    assert!(sim.targets().iter().all(|t| t.id != cow.id), "and is gone");
    // Swings may have broken the cows next to it too; all come back.
    idle(&mut sim, (def.respawn_time / Simulation::TICK_SECONDS) as usize + 2);
    assert_eq!(sim.targets().len(), base::COWS.len(), "back after its respawn time");
}

#[test]
fn a_cows_head_takes_double_damage() {
    let mut sim = dummy_sim();
    idle(&mut sim, 60);
    let cow = sim.targets().iter().next().unwrap().clone();
    let def = sim.target_def(cow.kind).clone();
    let head = def.hit_boxes.iter().position(|b| b.name == "head").expect("the cow has a head box");
    let b = &def.hit_boxes[head];
    // The middle of the head box, turned with the cow's facing.
    let mid = (api::Vec3::from(b.min) + api::Vec3::from(b.max)) * 0.5;
    let turn = cow.yaw - std::f32::consts::FRAC_PI_2;
    let (sin, cos) = turn.sin_cos();
    let at = cow.pos.translate(Vec3::new(cos * mid.x - sin * mid.y, sin * mid.x + cos * mid.y, mid.z));
    // Stand off to the side of the head, level with it.
    let side = Vec3::new(-sin, cos, 0.0).cross(Vec3::Z) * 1.8;
    sim.teleport_player(at.translate(side - Vec3::Z * 1.0));
    idle(&mut sim, 30);
    let hit = swing_at(&mut sim, at).hit.expect("in reach");
    assert_eq!(hit.zone, Some(head));
    let club = sim.tool(sim.tool_ids().next().unwrap()).clone();
    assert!((hit.damage - club.damage * b.damage).abs() < 1e-4, "{} damage", hit.damage);
}

#[test]
fn holding_a_tool_swings_and_guards_instead_of_editing() {
    let mut sim = base_sim();
    sim.teleport_player(WorldPos::new(I64Vec3::new(10, 10, 0), Vec3::new(0.5, 0.5, 0.0)));
    idle(&mut sim, 30);
    let floor = BoxelPos::new(10, 10, -1);
    let tool = sim.tool_ids().next();
    let down = -89f32.to_radians();
    let stone = sim.universe().main().block(floor);
    sim.step(&PlayerInput { pitch: down, tool, use_pressed: true, use_held: true, ..Default::default() });
    assert_eq!(sim.universe().main().block(floor), stone, "a swing doesn't break the floor");
    assert!(sim.swing().is_some());
    idle(&mut sim, 60);

    let brick = sim.blocks().id(base::BRICK).unwrap();
    let guard = PlayerInput { pitch: down, tool, alt_pressed: true, alt_held: true, place_block: brick, ..Default::default() };
    sim.step(&guard);
    assert!(sim.guarding());
    assert_eq!(sim.universe().main().block(BoxelPos::new(10, 10, 0)), sim::BlockId::EMPTY, "guarding places nothing");
    sim.step(&PlayerInput { pitch: down, ..Default::default() });
    assert!(!sim.guarding(), "letting go, or putting the tool away, drops the guard");

    sim.step(&PlayerInput { pitch: down, use_pressed: true, use_held: true, ..Default::default() });
    assert_ne!(sim.universe().main().block(floor), stone, "without a tool the button breaks blocks again");
}

#[test]
fn a_block_cannot_be_placed_into_a_cow() {
    let mut sim = dummy_sim();
    idle(&mut sim, 60);
    let cow = sim.targets().iter().next().unwrap().clone();
    let inside = cow.pos.translate(Vec3::Z * 0.5).boxel();
    sim.teleport_player(cow.pos.translate(Vec3::new(-2.5, 0.0, 0.0)));
    idle(&mut sim, 30);
    let brick = sim.blocks().id(base::BRICK).unwrap();
    // Aim at the floor right under the cow's middle.
    let (yaw, pitch) = aim_at(&sim, WorldPos::at_boxel(inside).translate(Vec3::new(0.5, 0.5, 0.0)));
    let hit = sim.target(yaw, pitch).expect("the floor is in reach");
    let place = sim.placement(&hit, brick, yaw, pitch, false).unwrap();
    assert_eq!(place.pos, inside);
    assert!(!place.allowed, "the cow is in the way");
}

/// Where each cow is, by ID.
fn cow_spots(sim: &Simulation) -> Vec<(u64, WorldPos)> {
    sim.targets().iter().map(|t| (t.id, t.pos)).collect()
}

#[test]
fn cows_wander_about_but_stay_in_the_pen() {
    let mut sim = base_sim();
    let start = cow_spots(&sim);
    let (lo, hi) = (BoxelPos::new(-19, 9, 0), BoxelPos::new(-9, 19, 0));
    let mut moved = vec![0.0f32; start.len()];
    let mut clips = std::collections::BTreeSet::new();
    for _ in 0..90 * 60 {
        sim.step(&PlayerInput::default());
        for (i, cow) in sim.targets().iter().enumerate() {
            let rel = WorldPos::at_boxel(lo).delta_to(cow.pos);
            let size = WorldPos::at_boxel(lo).delta_to(WorldPos::at_boxel(hi)) + Vec3::ONE;
            assert!(rel.x > 0.0 && rel.y > 0.0 && rel.x < size.x && rel.y < size.y, "cow {} left the pen: {rel}", cow.id);
            assert!(rel.z > -0.01 && rel.z < 2.01, "and stays between the floor and the platform top: {}", rel.z);
            moved[i] = moved[i].max(start[i].1.delta_to(cow.pos).truncate().length());
            clips.insert(cow.clip.clone());
        }
    }
    assert!(moved.iter().all(|&m| m > 1.0), "every cow wandered: {moved:?}");
    for clip in ["stand", "walk", "eat"] {
        assert!(clips.contains(clip), "{clip} played: {clips:?}");
    }
}

#[test]
fn a_cow_watches_the_player_come_near() {
    let mut sim = base_sim();
    let cow = sim.targets().iter().next().unwrap().clone();
    sim.teleport_player(cow.pos.translate(Vec3::new(0.0, 3.0, 0.0)));
    idle(&mut sim, 120);
    let now = sim.targets().iter().find(|t| t.id == cow.id).unwrap();
    let to_player = now.pos.delta_to(sim.player().unwrap().pos());
    let wanted = to_player.y.atan2(to_player.x);
    let off = (now.yaw - wanted + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    assert!(off.abs() < 0.05, "facing the player: off by {off}");
    assert_eq!(now.clip, "stand");
    assert!(now.velocity.truncate().length() < 0.05, "standing still: {}", now.velocity);
}

#[test]
fn a_hit_cow_runs_off_then_calms_down() {
    let mut sim = base_sim();
    let cow = sim.targets().iter().next().unwrap().clone();
    let def = sim.target_def(cow.kind).clone();
    sim.teleport_player(cow.pos.translate(Vec3::new(-2.0, 0.0, 0.0)));
    idle(&mut sim, 30);
    let now = sim.targets().iter().find(|t| t.id == cow.id).unwrap().clone();
    let hit = swing_at(&mut sim, now.pos.translate(Vec3::Z * def.height * 0.5)).hit.expect("in reach");
    assert_eq!(hit.target, cow.id);
    idle(&mut sim, 30);
    let running = sim.targets().iter().find(|t| t.id == cow.id).unwrap().clone();
    assert_eq!(running.clip, "run");
    let away = |sim: &Simulation, pos: WorldPos| sim.player().unwrap().pos().delta_to(pos).truncate().length();
    idle(&mut sim, 30);
    let later = sim.targets().iter().find(|t| t.id == cow.id).unwrap().clone();
    assert!(away(&sim, later.pos) > away(&sim, running.pos), "getting away from the player");
    idle(&mut sim, 5 * 60);
    let calm = sim.targets().iter().find(|t| t.id == cow.id).unwrap();
    assert_ne!(calm.clip, "run", "calmed down");
}

#[test]
fn with_brains_off_cows_stand_as_dummies() {
    let mut sim = base_sim();
    idle(&mut sim, 60 * 10);
    sim.set_brains_on(false);
    idle(&mut sim, 120);
    let still = cow_spots(&sim);
    idle(&mut sim, 120);
    assert_eq!(cow_spots(&sim), still, "nobody moves");
    assert!(sim.targets().iter().all(|t| t.clip.is_empty() && t.walk == api::Vec2::ZERO));
}

#[test]
fn a_target_naming_an_unknown_brain_is_an_error() {
    struct Lost;
    impl Mod for Lost {
        fn name(&self) -> &str {
            "lost"
        }

        fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
            Room.register(r)?;
            r.register_target(TargetDef { name: "lost:cow".into(), brain: Some("lost:nowhere".into()), ..dummy() })
        }
    }
    let err = Simulation::with_mods(&[&Lost]).err().expect("refused");
    assert_eq!(err, sim::SimError::Api(ApiError::UnknownBrain("lost:nowhere".into())));
}

/// The room's target.
fn dummy() -> TargetDef {
    TargetDef {
        name: "room:dummy".into(),
        model: "room:box".into(),
        hit_sound: None,
        brain: None,
        width: 0.8,
        height: 1.2,
        step_height: 0.0,
        hit_boxes: Vec::new(),
        // Enough that the tests never break it, slams included.
        health: 1000.0,
        weight: 1.0,
        gravity: 25.0,
        drag: 5.0,
        air_drag: 2.0,
        bounce: 0.2,
        respawn_time: 1.0,
        slam_speed: 4.0,
        slam_damage: 1.0,
    }
}

/// A stone room with one target and a club: a floor, and a wall across x = 2 to 3.
struct Room;

impl Mod for Room {
    fn name(&self) -> &str {
        "room"
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        r.register_block(api::BlockDef::new("room:stone", true, [128; 3]))?;
        r.register_player(PlayerDef { width: 0.6, height: 1.8, eye_height: 1.6, step_height: 0.0, controller: "base:quake".into() })?;
        r.register_interaction(InteractionDef { reach: 4.5, repeat_rate: 4.0 })?;
        r.register_model(ModelDef {
            name: "room:box".into(),
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            uvs: vec![[0.0; 2]; 3],
            indices: vec![0, 1, 2],
            texture: Texture { width: 1, height: 1, rgba: vec![255; 4] },
            skin: None,
        })?;
        r.register_target(dummy())?;
        r.register_tool(ToolDef {
            name: "room:club".into(),
            reach: 3.0,
            radius: 0.3,
            damage: 2.0,
            knockback: 10.0,
            lift: 3.0,
            momentum: 0.8,
            momentum_damage: 0.5,
            windup: 0.05,
            recover: 0.2,
        })
    }

    fn build_world(&self, w: &mut dyn WorldEdit) -> Result<(), ApiError> {
        w.fill(BoxelPos::new(-10, -10, -1), BoxelPos::new(10, 10, -1), "room:stone")?;
        w.fill(BoxelPos::new(2, -10, 0), BoxelPos::new(2, 10, 3), "room:stone")?;
        w.set_spawn(WorldPos::new(I64Vec3::new(0, 0, 0), Vec3::new(0.5, 0.5, 0.0)));
        w.add_target("room:dummy", WorldPos::new(I64Vec3::new(3, 0, 0), Vec3::new(0.5, 0.5, 0.0)), 0.0)
    }
}

#[test]
fn a_wall_between_stops_the_strike() {
    let mut sim = Simulation::with_mods(&[&Room, &base_movement::BaseMovementMod]).unwrap();
    sim.spawn_player().unwrap();
    idle(&mut sim, 30);
    let dummy = sim.targets().iter().next().unwrap().pos;
    let strike = swing_at(&mut sim, dummy.translate(Vec3::Z * 0.6));
    assert_eq!(strike.hit, None, "the dummy is in reach, but behind the wall");
    assert!(strike.reach < 2.0, "the wall cut the reach: {}", strike.reach);
}

#[test]
fn a_target_knocked_into_a_wall_stops_against_it() {
    let mut sim = Simulation::with_mods(&[&Room, &base_movement::BaseMovementMod]).unwrap();
    sim.spawn_player().unwrap();
    idle(&mut sim, 30);
    // From the far side, knock the dummy west into the wall, again and again.
    for _ in 0..5 {
        let dummy = sim.targets().iter().next().unwrap().pos;
        sim.teleport_player(dummy.translate(Vec3::new(1.5, 0.0, 0.0)));
        idle(&mut sim, 20);
        swing_at(&mut sim, dummy.translate(Vec3::Z * 0.6));
        idle(&mut sim, 60);
        let dummy = sim.targets().iter().next().unwrap();
        let west_side = WorldPos::new(I64Vec3::new(3, 0, 0), Vec3::ZERO).delta_to(dummy.pos).x - 0.4;
        assert!(west_side >= -1e-3, "inside the wall by {}", -west_side);
        assert!(dummy.on_ground, "settled on the floor: {:?} {:?} v {}", dummy.pos.cell(), dummy.pos.offset(), dummy.velocity);
    }
}

#[test]
fn sprinting_into_a_swing_hits_harder() {
    let mut sim = Simulation::with_mods(&[&Room, &base_movement::BaseMovementMod]).unwrap();
    sim.spawn_player().unwrap();
    idle(&mut sim, 30);
    let dummy = sim.targets().iter().next().unwrap().pos;
    let tool = sim.tool_ids().next();
    // Start 7 m east of the dummy and sprint at it, swinging as it comes into reach.
    sim.teleport_player(dummy.translate(Vec3::new(7.0, 0.0, 0.0)));
    idle(&mut sim, 30);
    let target = dummy.translate(Vec3::Z * 0.6);
    let mut strike = None;
    for _ in 0..120 {
        let (yaw, pitch) = aim_at(&sim, target);
        let close = sim.player().unwrap().eye().delta_to(target).truncate().length() < 3.2;
        let input = PlayerInput {
            movement: api::Vec2::new(0.0, 1.0),
            sprint: true,
            yaw,
            pitch,
            tool,
            use_held: close,
            ..Default::default()
        };
        sim.step(&input);
        if let Some(s) = sim.take_strikes().pop() {
            strike = Some(s);
            break;
        }
    }
    let hit = strike.and_then(|s| s.hit).expect("hit on the run");
    assert!(hit.carried > 3.0, "carried {} m/s", hit.carried);
    let club = sim.tool(tool.unwrap()).clone();
    assert!((hit.damage - club.damage - hit.carried * club.momentum_damage).abs() < 1e-4, "{} damage", hit.damage);
}
