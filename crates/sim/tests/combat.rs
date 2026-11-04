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

/// The base game with cows that run when hit, or when another is, instead of fighting.
fn timid_sim() -> Simulation {
    let mut sim = base_sim();
    let cow = sim.brain_names().position(|n| n == base::cow::COW_BRAIN).unwrap();
    let brain = sim.brain_mut(cow).unwrap();
    for name in ["fight_share", "defend_chance", "courage", "injury_pull"] {
        assert!(brain.set_tunable(name, 0.0), "{name}");
    }
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

/// Whether `pos` is inside the pasture's fence, at floor level or just above.
fn in_pasture(pos: WorldPos) -> bool {
    let ((x0, y0), (x1, _)) = base::PASTURE;
    let rel = WorldPos::at_boxel(BoxelPos::new(x0 + 1, y0 + 1, 0)).delta_to(pos);
    let size = (x1 - x0 - 1) as f32;
    rel.x > 0.0 && rel.y > 0.0 && rel.x < size && rel.y < size && rel.z > -0.01 && rel.z < 0.5
}

#[test]
fn base_places_its_cows_in_the_pasture_where_they_settle() {
    let mut sim = dummy_sim();
    let cows: Vec<_> = sim.targets().iter().filter(|t| sim.target_def(t.kind).name == base::COW).collect();
    assert_eq!(cows.len(), base::COWS.len(), "one cow per spot, in the near test area only");
    let model = &sim.target_def(cows[0].kind).model;
    assert!(sim.models().iter().any(|m| &m.name == model), "the cow's model is registered");
    idle(&mut sim, 120);
    for cow in sim.targets().iter() {
        assert!(cow.on_ground && cow.velocity == Vec3::ZERO, "settled");
        assert!(in_pasture(cow.pos), "in the pasture: {:?}", cow.pos);
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
fn a_broken_cow_stays_gone_until_the_player_is_knocked_out() {
    let mut sim = dummy_sim();
    let cow = sim.targets().iter().next().unwrap().clone();
    let def = sim.target_def(cow.kind).clone();
    for _ in 0..80 {
        let Some(now) = sim.targets().iter().find(|t| t.id == cow.id).cloned() else {
            break;
        };
        // Step up beside it each time, wherever it was knocked.
        sim.teleport_player(now.pos.translate(Vec3::new(-1.8, 0.0, 0.0)));
        idle(&mut sim, 40);
        let Some(now) = sim.targets().iter().find(|t| t.id == cow.id).cloned() else {
            break;
        };
        swing_at(&mut sim, now.pos.translate(Vec3::Z * def.height * 0.5));
    }
    // Broken by a swing or by a slam into the fence.
    assert!(sim.targets().iter().all(|t| t.id != cow.id), "the cow broke and is gone");
    // Long past its respawn time, it's still gone.
    idle(&mut sim, (def.respawn_time / Simulation::TICK_SECONDS) as usize * 3);
    assert!(sim.targets().len() < base::COWS.len(), "doesn't come back on its own");
    // A fall that knocks the player out brings the whole herd back, whole.
    sim.set_health(0.5);
    let spawn = sim.spawn();
    sim.teleport_player(spawn.translate(Vec3::Z * 30.0));
    let mut out = false;
    for _ in 0..5 * 60 {
        sim.step(&PlayerInput::default());
        out |= sim.take_player_hurts().iter().any(|h| h.died);
        if out {
            break;
        }
    }
    assert!(out, "knocked out by the fall");
    assert_eq!(sim.targets().len(), base::COWS.len(), "all back");
    assert!(sim.targets().iter().all(|t| t.health == def.health), "whole");
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

/// The middle of the cows, and the furthest any is from it, level.
fn herd_spread(sim: &Simulation) -> (WorldPos, f32) {
    let cows: Vec<WorldPos> = sim.targets().iter().map(|t| t.pos).collect();
    let first = cows[0];
    let middle = first.translate(cows.iter().map(|&c| first.delta_to(c)).sum::<Vec3>() / cows.len() as f32);
    let far = cows.iter().map(|&c| middle.delta_to(c).truncate().length()).fold(0.0, f32::max);
    (middle, far)
}

/// The closest any two cows' feet are, level.
fn closest_pair(sim: &Simulation) -> f32 {
    let cows: Vec<WorldPos> = sim.targets().iter().map(|t| t.pos).collect();
    let mut closest = f32::MAX;
    for (i, a) in cows.iter().enumerate() {
        for b in &cows[i + 1..] {
            closest = closest.min(a.delta_to(*b).truncate().length());
        }
    }
    closest
}

#[test]
fn cows_wander_about_as_a_herd_and_stay_in_the_pasture() {
    let mut sim = base_sim();
    let start = cow_spots(&sim);
    let (first_middle, _) = herd_spread(&sim);
    let mut moved = vec![0.0f32; start.len()];
    let mut clips = std::collections::BTreeSet::new();
    let mut widest = 0.0f32;
    let mut closest = f32::MAX;
    for tick in 0..120 * 60 {
        sim.step(&PlayerInput::default());
        for (i, cow) in sim.targets().iter().enumerate() {
            assert!(in_pasture(cow.pos), "cow {} left the pasture: {:?}", cow.id, cow.pos);
            moved[i] = moved[i].max(start[i].1.delta_to(cow.pos).truncate().length());
            clips.insert(cow.clip.clone());
        }
        // After the first few seconds, once they've spread from their spots.
        if tick > 10 * 60 {
            widest = widest.max(herd_spread(&sim).1);
            closest = closest.min(closest_pair(&sim));
        }
    }
    assert!(moved.iter().all(|&m| m > 1.0), "every cow wandered: {moved:?}");
    for clip in ["stand", "walk", "eat"] {
        assert!(clips.contains(clip), "{clip} played: {clips:?}");
    }
    assert!(widest < 9.0, "kept together: one got {widest} m from the middle");
    assert!(closest > 0.8, "never inside each other: {closest} m");
    let (middle, _) = herd_spread(&sim);
    assert!(first_middle.delta_to(middle).truncate().length() > 0.5, "the herd drifts about together");
}

#[test]
fn a_straying_cow_comes_back_to_the_herd() {
    let mut sim = timid_sim();
    // A short, slow stampede, as before Fighting cow's tuning: at the base's run speed of 8 and
    // spook time of 5.3 the herd scatters into groups further apart than they can see, and each
    // group keeps to itself, which isn't what this test is about.
    let brain = sim.brain_names().position(|n| n == base::cow::COW_BRAIN).unwrap();
    let brain = sim.brain_mut(brain).unwrap();
    assert!(brain.set_tunable("run_speed", 3.0) && brain.set_tunable("spook_time", 2.0));
    let (middle, _) = herd_spread(&sim);
    // Carry one cow off toward the fence, out of the others' way, by knocking it there.
    let cow = sim.targets().iter().next().unwrap().id;
    let mut far = 0.0;
    for _ in 0..30 {
        let pos = sim.targets().iter().find(|t| t.id == cow).unwrap().pos;
        far = middle.delta_to(pos).truncate().length();
        if far > 8.0 {
            break;
        }
        sim.teleport_player(pos.translate(Vec3::new(1.8, 1.8, 0.0)));
        idle(&mut sim, 20);
        let pos = sim.targets().iter().find(|t| t.id == cow).unwrap().pos;
        swing_at(&mut sim, pos.translate(Vec3::Z * 0.6));
        idle(&mut sim, 40);
    }
    assert!(far > 8.0, "got it away: {far}");
    // Take the player well away and let things settle.
    sim.teleport_player(WorldPos::at_boxel(BoxelPos::new(0, 0, 0)));
    idle(&mut sim, 60 * 60);
    let (middle, _) = herd_spread(&sim);
    let pos = sim.targets().iter().find(|t| t.id == cow).map(|t| t.pos).expect("not broken");
    let back = middle.delta_to(pos).truncate().length();
    assert!(back < 7.0, "back with the herd: {back} m from the middle");
}

#[test]
fn hitting_one_cow_spooks_the_others() {
    let mut sim = base_sim();
    idle(&mut sim, 60);
    let cow = sim.targets().iter().next().unwrap().clone();
    let def = sim.target_def(cow.kind).clone();
    sim.teleport_player(cow.pos.translate(Vec3::new(-2.0, -2.0, 0.0)));
    idle(&mut sim, 10);
    let now = sim.targets().iter().find(|t| t.id == cow.id).unwrap().clone();
    swing_at(&mut sim, now.pos.translate(Vec3::Z * def.height * 0.5)).hit.expect("in reach");
    // Within a couple of seconds most of the herd is running.
    let mut ran = std::collections::BTreeSet::new();
    for _ in 0..120 {
        sim.step(&PlayerInput::default());
        for t in sim.targets().iter().filter(|t| t.clip == "run") {
            ran.insert(t.id);
        }
    }
    assert!(ran.len() >= 5, "{} of {} ran", ran.len(), sim.targets().len());
    // And it dies out.
    sim.teleport_player(WorldPos::at_boxel(BoxelPos::new(0, 0, 0)));
    idle(&mut sim, 12 * 60);
    let running = sim.targets().iter().filter(|t| t.clip == "run").count();
    assert_eq!(running, 0, "all calm again");
}

#[test]
fn the_player_shoves_a_cow_and_cannot_walk_through_it() {
    let mut sim = dummy_sim();
    idle(&mut sim, 60);
    let cow = sim.targets().iter().next().unwrap().clone();
    // Walk north into the middle of its side (it faces east) for two seconds.
    let start = cow.pos.translate(Vec3::new(0.5, -2.0, 0.0));
    sim.teleport_player(start);
    idle(&mut sim, 30);
    let walk = PlayerInput { movement: api::Vec2::new(0.0, 1.0), yaw: std::f32::consts::FRAC_PI_2, ..Default::default() };
    let mut closest = f32::MAX;
    for _ in 0..120 {
        sim.step(&walk);
        let now = sim.targets().iter().find(|t| t.id == cow.id).unwrap();
        let player = sim.player().unwrap().pos();
        closest = closest.min(now.pos.delta_to(player).truncate().length());
        assert!(now.pos.delta_to(player).y < 0.0, "never got through to the far side");
    }
    let now = sim.targets().iter().find(|t| t.id == cow.id).unwrap();
    assert!(cow.pos.delta_to(now.pos).y > 0.5, "the cow was shoved north: {}", cow.pos.delta_to(now.pos));
    assert!(closest > 0.3, "and the player stayed mostly out of it: {closest}");
}

#[test]
fn the_player_walks_through_the_herd_without_getting_stuck() {
    let mut sim = base_sim();
    idle(&mut sim, 60);
    let (middle, _) = herd_spread(&sim);
    // From 12 m west of the middle to 12 m east of it, steering straight at the far side.
    let goal = middle.translate(Vec3::new(12.0, 0.0, 0.0));
    sim.teleport_player(middle.translate(Vec3::new(-12.0, 0.0, 0.0)));
    idle(&mut sim, 20);
    let mut made_it = false;
    for _ in 0..15 * 60 {
        let to = sim.player().unwrap().pos().delta_to(goal);
        if to.truncate().length() < 1.0 {
            made_it = true;
            break;
        }
        let input = PlayerInput { movement: api::Vec2::new(0.0, 1.0), yaw: to.y.atan2(to.x), ..Default::default() };
        sim.step(&input);
    }
    assert!(made_it, "got across within 15 s");
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
    let mut sim = timid_sim();
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
        respawns: true,
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

/// Hits the first cow from 2 m west, so it turns to fight, and returns its ID. The player is
/// unkillable, so blows never send them back to the spawn mid-test.
fn anger_a_cow(sim: &mut Simulation) -> u64 {
    sim.set_unkillable(true);
    idle(sim, 60);
    let cow = sim.targets().iter().next().unwrap().clone();
    let def = sim.target_def(cow.kind).clone();
    sim.teleport_player(cow.pos.translate(Vec3::new(-2.0, 0.0, 0.0)));
    idle(sim, 10);
    let now = sim.targets().iter().find(|t| t.id == cow.id).unwrap().clone();
    swing_at(sim, now.pos.translate(Vec3::Z * def.height * 0.5)).hit.expect("in reach");
    cow.id
}

/// Runs `seconds` standing still, gathering blows on the player.
fn blows(sim: &mut Simulation, seconds: f32) -> Vec<sim::PlayerHurt> {
    let mut hurts = Vec::new();
    for _ in 0..(seconds * 60.0) as usize {
        sim.step(&PlayerInput::default());
        hurts.extend(sim.take_player_hurts());
    }
    hurts
}

fn cow_mode(sim: &Simulation, id: u64) -> Option<f32> {
    sim.targets().iter().find(|t| t.id == id).and_then(|t| t.memory.first().copied())
}

#[test]
fn a_hit_cow_tosses_a_player_up_close_throwing_them_up_and_away() {
    let mut sim = base_sim();
    let cow = anger_a_cow(&mut sim);
    let hurts = blows(&mut sim, 5.0);
    let first = hurts.first().expect("tossed within 5 s");
    let brain = base::cow::CowBrain::default();
    assert_eq!(first.attacker.map(|a| a.0), Some(cow));
    assert!((first.damage - brain.toss_damage).abs() < 1e-4, "a toss's damage: {}", first.damage);
    assert!(first.push.z > first.push.truncate().length(), "mostly up: {}", first.push);
    assert!(first.push.truncate().length() > 1.0, "and away: {}", first.push);
}

#[test]
fn a_hit_cow_charges_a_player_far_off_for_big_damage_and_knockback() {
    let mut sim = base_sim();
    let cow = anger_a_cow(&mut sim);
    // 10 m off across the pasture, toward its middle.
    let at = sim.targets().iter().find(|t| t.id == cow).unwrap().pos;
    let ((x0, _), (x1, _)) = base::PASTURE;
    let middle_x = (x0 + x1) as f32 * 0.5;
    let toward = if (at.cell().x as f32) < middle_x { 10.0 } else { -10.0 };
    let spot = at.translate(Vec3::new(toward, 0.0, 0.0));
    sim.teleport_player(spot);
    assert!(in_pasture(spot), "{spot:?}");
    let hurts = blows(&mut sim, 8.0);
    let brain = base::cow::CowBrain::default();
    let charge = hurts.iter().find(|h| (h.damage - brain.charge_damage).abs() < 1e-4).expect("charged within 8 s");
    assert_eq!(charge.attacker.map(|a| a.0), Some(cow));
    assert!(charge.push.truncate().length() > 10.0, "thrown hard: {}", charge.push);
}

#[test]
fn a_cow_charging_into_the_fence_is_stunned_and_then_gives_up() {
    let mut sim = base_sim();
    let cow = anger_a_cow(&mut sim);
    // Lure it over to the east fence, standing just inside it until it gets there.
    let (_, out) = sim.waypoints().iter().find(|(n, _)| n == "pasture").unwrap().clone();
    let inside = out.translate(Vec3::new(-3.5, 0.0, 0.0));
    assert!(in_pasture(inside));
    for _ in 0..10 * 60 {
        sim.teleport_player(inside);
        sim.step(&PlayerInput::default());
        if !sim.take_player_hurts().is_empty() {
            break;
        }
    }
    // Then step out over it, far enough off that it charges, into the fence.
    let at = sim.targets().iter().find(|t| t.id == cow).unwrap().pos;
    let off = 8.0 - at.delta_to(out).x;
    sim.teleport_player(out.translate(Vec3::new(off.max(0.0), 0.0, 0.0)));
    let stunned = base::cow::Mode::Stunned as usize as f32;
    let mut was_stunned = false;
    let mut hurts = Vec::new();
    for _ in 0..12 * 60 {
        sim.step(&PlayerInput::default());
        hurts.extend(sim.take_player_hurts());
        was_stunned |= cow_mode(&sim, cow) == Some(stunned);
    }
    assert!(was_stunned, "ran into the fence");
    assert!(hurts.is_empty(), "never reached the player");
    let mode = cow_mode(&sim, cow).expect("not broken") as usize;
    assert!(mode < base::cow::Mode::Angry as usize, "gave up well before its anger ran out: mode {mode}");
    let calm = sim.targets().iter().find(|t| t.id == cow).unwrap();
    assert!(in_pasture(calm.pos), "still in the pasture");
}


#[test]
fn hitting_one_cow_brings_some_of_the_herd_to_its_side_and_scatters_the_rest() {
    let mut sim = base_sim();
    // Whether any one cow helps is down to chance; a keener herd makes both outcomes sure.
    let cow = sim.brain_names().position(|n| n == base::cow::COW_BRAIN).unwrap();
    assert!(sim.brain_mut(cow).unwrap().set_tunable("defend_chance", 0.5));
    let hit = anger_a_cow(&mut sim);
    let mut fought = std::collections::BTreeSet::new();
    let mut ran = std::collections::BTreeSet::new();
    for _ in 0..4 * 60 {
        sim.step(&PlayerInput::default());
        sim.take_player_hurts();
        for t in sim.targets().iter() {
            match t.signal.as_str() {
                base::cow::ANGRY => {
                    fought.insert(t.id);
                }
                base::cow::ALARMED if !fought.contains(&t.id) => {
                    ran.insert(t.id);
                }
                _ => {}
            }
        }
    }
    fought.remove(&hit);
    ran.retain(|id| *id != hit && !fought.contains(id));
    assert!(!fought.is_empty(), "at least one came to help");
    assert!(ran.len() >= 2, "others ran: {} ran, {} fought", ran.len(), fought.len());
    // They calm down once the player is far off.
    sim.teleport_player(WorldPos::at_boxel(BoxelPos::new(0, 0, 0)));
    idle(&mut sim, 6 * 60);
    assert!(sim.targets().iter().all(|t| t.signal != base::cow::ANGRY), "lost interest");
}


