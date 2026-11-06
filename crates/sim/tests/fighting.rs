//! Creatures hitting the player: attacks landing once, knockback, the guard, health running
//! out and coming back. A small arena of this test's own, with a creature whose brain jabs on
//! a fixed beat, so each rule shows on its own.

use api::{
    ApiError, Attack, BoxelPos, Brain, BrainContext, BrainOutput, HealthDef, I64Vec3, Mod, ModelDef, PlayerDef, Registrar,
    TargetDef, Texture, ToolDef, Tunable, Tunables, Vec3, WorldEdit, WorldPos,
};
use sim::{PlayerHurt, PlayerInput, Simulation, Strike};

/// Faces east and jabs the box in front of it for `on` ticks out of every `every`, counting from
/// its first think.
struct Jab {
    on: u32,
    every: u32,
}

impl Tunables for Jab {
    fn tunables(&self) -> Vec<Tunable> {
        Vec::new()
    }

    fn set_tunable(&mut self, _: &str, _: f32) -> bool {
        false
    }
}

impl Brain for Jab {
    fn think(&self, ctx: &BrainContext) -> BrainOutput {
        let tick = ctx.memory.first().copied().unwrap_or(0.0);
        let live = (tick as u32 % self.every) < self.on;
        BrainOutput {
            yaw: 0.0,
            attack: live.then(attack),
            memory: vec![tick + 1.0],
            ..Default::default()
        }
    }
}

const DAMAGE: f32 = 3.0;
/// The club's push back in the air, per point of damage, and the most it leaves the player
/// rising at.
const RECOIL: f32 = 2.0;
const BOUNCE: f32 = 3.0;
const PUSH: f32 = 8.0;
const LIFT: f32 = 5.0;

/// From 0.5 to 2.5 m in front of it, 1.2 wide.
fn attack() -> Attack {
    Attack {
        min: [-0.6, 0.5, 0.0],
        max: [0.6, 2.5, 1.8],
        damage: DAMAGE,
        push: PUSH,
        lift: LIFT,
        guarded_damage: 0.25,
        guarded_push: 0.5,
    }
}

fn health() -> HealthDef {
    HealthDef { health: 10.0, regen: 2.0, regen_delay: 1.0, guard_angle: 1.0, fall_speed: 12.0, fall_damage: 0.5, knock_out_resets: false }
}

/// A stone floor, the player at the spawn, a club, and the jabber 1.5 m west of the spawn,
/// facing it.
struct Arena {
    jab: (u32, u32),
    health: bool,
}

/// Where the player spawns.
fn spawn() -> WorldPos {
    WorldPos::new(I64Vec3::new(0, 0, 0), Vec3::new(0.5, 0.5, 0.0))
}

impl Mod for Arena {
    fn name(&self) -> &str {
        "arena"
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        r.register_block(api::BlockDef::new("arena:stone", true, [128; 3]))?;
        r.register_player(PlayerDef { width: 0.6, height: 1.8, eye_height: 1.6, step_height: 0.0, controller: "base:quake".into(), sounds: Default::default() })?;
        if self.health {
            r.register_health(health())?;
        }
        r.register_model(ModelDef {
            name: "arena:box".into(),
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            uvs: vec![[0.0; 2]; 3],
            indices: vec![0, 1, 2],
            texture: Texture { width: 1, height: 1, rgba: vec![255; 4] },
            skin: None,
        })?;
        r.register_brain("arena:jab", Box::new(Jab { on: self.jab.0, every: self.jab.1 }))?;
        r.register_target(TargetDef {
            name: "arena:jabber".into(),
            model: "arena:box".into(),
            sounds: Default::default(),
            brain: Some("arena:jab".into()),
            width: 0.8,
            height: 1.2,
            step_height: 0.0,
            hit_boxes: Vec::new(),
            health: 1000.0,
            // Heavy, so it stays put.
            weight: 100.0,
            gravity: 25.0,
            drag: 5.0,
            air_drag: 0.0,
            bounce: 0.0,
            respawn_time: 1.0,
            slam_speed: 1000.0,
            slam_damage: 0.0,
            hop_speed: 0.0,
            max_drop: 0.0,
            fall_speed: 1000.0,
            fall_damage: 0.0,
            respawns: true,
        })?;
        r.register_tool(ToolDef {
            name: "arena:club".into(),
            reach: 3.0,
            radius: 0.3,
            damage: 1.0,
            knockback: 1.0,
            lift: 0.0,
            momentum: 0.0,
            momentum_damage: 0.0,
            recoil: RECOIL,
            recoil_bounce: BOUNCE,
            windup: 0.1,
            recover: 0.2,
            sounds: Default::default(),
        })
    }

    fn build_world(&self, w: &mut dyn WorldEdit) -> Result<(), ApiError> {
        w.fill(BoxelPos::new(-30, -30, -1), BoxelPos::new(30, 30, -1), "arena:stone")?;
        w.set_spawn(spawn());
        // Its +Y front turned to face east, by the brain's yaw of 0.
        w.add_target("arena:jabber", spawn().translate(Vec3::new(-1.5, 0.0, 0.0)), 0.0)
    }
}

fn arena(on: u32, every: u32) -> Simulation {
    let mut sim = Simulation::with_mods(&[&Arena { jab: (on, every), health: true }, &base_movement::BaseMovementMod]).unwrap();
    sim.spawn_player().unwrap();
    sim
}

/// Runs `ticks` ticks with `input`, gathering the blows.
fn run(sim: &mut Simulation, input: &PlayerInput, ticks: usize) -> Vec<PlayerHurt> {
    let mut hurts = Vec::new();
    for _ in 0..ticks {
        sim.step(input);
        hurts.extend(sim.take_player_hurts());
    }
    hurts
}

/// Keeps the player standing at the spawn for `ticks` ticks, so knockback doesn't carry them out
/// of reach, gathering the blows.
fn held(sim: &mut Simulation, input: &PlayerInput, ticks: usize) -> Vec<PlayerHurt> {
    let mut hurts = Vec::new();
    for _ in 0..ticks {
        sim.teleport_player(spawn());
        sim.step(input);
        hurts.extend(sim.take_player_hurts());
    }
    hurts
}

#[test]
fn an_attack_kept_live_hits_once() {
    let mut sim = arena(1000, 1000);
    // Under the regen delay.
    let hurts = held(&mut sim, &PlayerInput::default(), 50);
    assert_eq!(hurts.len(), 1, "{hurts:?}");
    assert_eq!(hurts[0].damage, DAMAGE);
    assert!((sim.health() - (health().health - DAMAGE)).abs() < 1e-4);
}

#[test]
fn a_new_attack_after_a_gap_hits_again() {
    let mut sim = arena(3, 20);
    let hurts = held(&mut sim, &PlayerInput::default(), 60);
    assert_eq!(hurts.len(), 3, "one per jab");
}

#[test]
fn a_blow_knocks_the_player_away_and_up() {
    let mut sim = arena(1000, 1000);
    let hurts = run(&mut sim, &PlayerInput::default(), 2);
    assert_eq!(hurts.len(), 1);
    assert!((hurts[0].from - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-4, "came from the west: {}", hurts[0].from);
    sim.step(&PlayerInput::default());
    let v = sim.player().unwrap().velocity();
    assert!(v.x > PUSH * 0.8 && v.z > LIFT * 0.5, "thrown east and up: {v}");
    run(&mut sim, &PlayerInput::default(), 60);
    let moved = spawn().delta_to(sim.player().unwrap().pos());
    assert!(moved.x > 1.0, "landed further east: {moved}");
}

#[test]
fn running_at_the_attacker_does_not_eat_into_the_knockback() {
    let mut sim = arena(1000, 1000);
    sim.apply_player_impulse(Vec3::new(-6.0, 0.0, 0.0));
    let hurts = run(&mut sim, &PlayerInput::default(), 2);
    assert_eq!(hurts.len(), 1);
    sim.step(&PlayerInput::default());
    assert!(sim.player().unwrap().velocity().x > PUSH * 0.8, "{}", sim.player().unwrap().velocity());
}

#[test]
fn outside_the_box_it_misses() {
    let mut sim = arena(1000, 1000);
    // Behind the jabber, then off to its side.
    for spot in [Vec3::new(-3.5, 0.0, 0.0), Vec3::new(-1.5, 2.0, 0.0)] {
        let at = spawn().translate(spot);
        let mut hurts = Vec::new();
        for _ in 0..60 {
            sim.teleport_player(at);
            sim.step(&PlayerInput::default());
            hurts.extend(sim.take_player_hurts());
        }
        assert!(hurts.is_empty(), "missed at {spot}");
    }
}

#[test]
fn a_guard_facing_the_attacker_softens_the_blow_and_one_facing_away_does_not() {
    for (yaw, guarded) in [(std::f32::consts::PI, true), (0.0, false)] {
        let mut sim = arena(1000, 1000);
        let tool = sim.tool_ids().next();
        let input = PlayerInput { yaw, tool, alt_held: true, ..Default::default() };
        let hurts = run(&mut sim, &input, 2);
        assert_eq!(hurts.len(), 1);
        assert_eq!(hurts[0].guarded, guarded, "looking along {yaw}");
        let (damage, push) = if guarded { (DAMAGE * 0.25, PUSH * 0.5) } else { (DAMAGE, PUSH) };
        assert!((hurts[0].damage - damage).abs() < 1e-4, "{}", hurts[0].damage);
        assert!((hurts[0].push.x - push).abs() < 1e-3, "{}", hurts[0].push);
    }
}

#[test]
fn out_of_health_the_player_goes_back_whole_unless_unkillable() {
    let mut sim = arena(1, 10);
    // Three blows of 3 leave 1; the fourth knocks the player out.
    let hurts = held(&mut sim, &PlayerInput::default(), 21);
    assert_eq!(hurts.len(), 3);
    assert!(hurts.iter().all(|h| !h.died));
    assert!((sim.health() - 1.0).abs() < 1e-4);
    sim.teleport_player(spawn());
    let hurts = run(&mut sim, &PlayerInput::default(), 10);
    assert!(hurts.len() == 1 && hurts[0].died, "{hurts:?}");
    assert_eq!(sim.health(), health().health, "whole again");
    assert!(spawn().delta_to(sim.player().unwrap().pos()).length() < 0.1, "at the spawn");

    let mut sim = arena(1, 10);
    sim.set_unkillable(true);
    sim.set_health(1.0);
    let hurts = held(&mut sim, &PlayerInput::default(), 30);
    assert!(hurts.iter().all(|h| !h.died));
    assert_eq!(sim.health(), 0.0, "stays at none");
}

#[test]
fn health_comes_back_only_after_a_while_without_hits() {
    let mut sim = arena(1, 100_000);
    held(&mut sim, &PlayerInput::default(), 2);
    let hurt = sim.health();
    assert!(hurt < health().health);
    // Out of reach.
    sim.teleport_player(spawn().translate(Vec3::new(10.0, 0.0, 0.0)));
    run(&mut sim, &PlayerInput::default(), 50);
    assert_eq!(sim.health(), hurt, "not before the delay");
    run(&mut sim, &PlayerInput::default(), 30);
    assert!(sim.health() > hurt, "coming back");
    run(&mut sim, &PlayerInput::default(), 120);
    assert_eq!(sim.health(), health().health, "up to whole, no further");
}

#[test]
fn without_a_health_definition_blows_still_knock_but_never_hurt() {
    let mut sim = Simulation::with_mods(&[&Arena { jab: (1000, 1000), health: false }, &base_movement::BaseMovementMod]).unwrap();
    sim.spawn_player().unwrap();
    let hurts = run(&mut sim, &PlayerInput::default(), 3);
    assert_eq!(hurts.len(), 1);
    assert_eq!((hurts[0].damage, hurts[0].died), (0.0, false));
    assert!(sim.player().unwrap().velocity().x > 0.0);
    assert_eq!(sim.health(), 0.0);
}

#[test]
fn a_second_health_definition_is_refused() {
    struct Twice;
    impl Mod for Twice {
        fn name(&self) -> &str {
            "twice"
        }

        fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
            r.register_health(health())?;
            r.register_health(health())
        }
    }
    let err = Simulation::with_mods(&[&Twice]).err().expect("refused");
    assert_eq!(err, sim::SimError::Api(ApiError::DuplicateHealthDef));
}

/// Drops the player from `height` meters above the floor, away from the jabber, and gathers
/// what hurts them until just after they land, before health comes back.
fn drop_from(sim: &mut Simulation, height: f32) -> Vec<PlayerHurt> {
    sim.teleport_player(spawn().translate(Vec3::new(10.0, 0.0, height)));
    let landed = sim.player().unwrap().landings();
    let mut hurts = Vec::new();
    for _ in 0..180 {
        hurts.extend(run(sim, &PlayerInput::default(), 1));
        if sim.player().unwrap().landings() != landed {
            hurts.extend(run(sim, &PlayerInput::default(), 2));
            return hurts;
        }
    }
    panic!("never landed");
}

#[test]
fn a_short_drop_does_not_hurt() {
    let mut sim = arena(0, 1);
    let hurts = drop_from(&mut sim, 2.0);
    assert!(hurts.is_empty(), "{hurts:?}");
    assert_eq!(sim.health(), health().health);
}

#[test]
fn a_long_drop_hurts_by_how_far_past_the_limit_it_lands() {
    let mut sim = arena(0, 1);
    let hurts = drop_from(&mut sim, 6.0);
    assert_eq!(hurts.len(), 1, "{hurts:?}");
    let speed = sim.player().unwrap().last_landing_speed();
    assert!(speed > health().fall_speed, "{speed}");
    let fall = &hurts[0];
    assert_eq!(fall.attacker, None);
    assert!((fall.damage - (speed - health().fall_speed) * health().fall_damage).abs() < 1e-4, "{}", fall.damage);
    assert!((sim.health() - (health().health - fall.damage)).abs() < 1e-4);
    // Further hurts more.
    let mut sim = arena(0, 1);
    let further = drop_from(&mut sim, 12.0);
    assert!(further[0].damage > fall.damage);
}

#[test]
fn a_fall_can_knock_the_player_out_unless_unkillable() {
    let mut sim = arena(0, 1);
    sim.set_health(1.0);
    let hurts = drop_from(&mut sim, 8.0);
    assert!(hurts.len() == 1 && hurts[0].died, "{hurts:?}");
    assert_eq!(sim.health(), health().health);
    assert!(spawn().delta_to(sim.player().unwrap().pos()).length() < 0.1, "back at the spawn");
    let mut sim = arena(0, 1);
    sim.set_unkillable(true);
    sim.set_health(1.0);
    let hurts = drop_from(&mut sim, 8.0);
    assert!(hurts.iter().all(|h| !h.died));
    assert_eq!(sim.health(), 0.0);
}

#[test]
fn a_blow_mid_jump_throws_no_higher_than_one_standing() {
    // How high the player goes, after an upward kick of `rising` m/s just as the blow lands.
    let peak = |rising: f32| {
        let mut sim = arena(1000, 1000);
        sim.apply_player_impulse(Vec3::Z * rising);
        let mut top = 0.0f32;
        for _ in 0..90 {
            sim.step(&PlayerInput::default());
            top = top.max(spawn().delta_to(sim.player().unwrap().pos()).z);
        }
        top
    };
    let standing = peak(0.0);
    let jumping = peak(4.0);
    assert!(standing > 0.3, "{standing}");
    // Only the bit the jump had already risen before the blow; adding the lift to the jump's
    // speed would have thrown it about three times as high.
    assert!(jumping < standing + 0.15, "mid-jump {jumping} m, standing {standing} m");
    // Rising faster than the lift already, it adds nothing.
    let mut sim = arena(1000, 1000);
    sim.apply_player_impulse(Vec3::Z * 20.0);
    let hurts = run(&mut sim, &PlayerInput::default(), 2);
    assert_eq!(hurts.len(), 1);
    assert_eq!(hurts[0].push.z, 0.0);
}

#[test]
fn a_knock_out_puts_every_creature_back_when_the_health_def_says_so() {
    for resets in [true, false] {
        struct Resetting(bool);
        impl Mod for Resetting {
            fn name(&self) -> &str {
                "resetting"
            }

            fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
                r.register_health(HealthDef { knock_out_resets: self.0, ..health() })
            }
        }
        let mut sim = Simulation::with_mods(&[&Arena { jab: (1, 30), health: false }, &Resetting(resets), &base_movement::BaseMovementMod]).unwrap();
        sim.spawn_player().unwrap();
        let before: Vec<u64> = sim.targets().iter().map(|t| t.id).collect();
        sim.set_health(1.0);
        let hurts = held(&mut sim, &PlayerInput::default(), 3);
        assert!(hurts.iter().any(|h| h.died));
        let after: Vec<u64> = sim.targets().iter().map(|t| t.id).collect();
        assert_eq!(after != before, resets, "put back, under new IDs: {resets}");
    }
}

/// The middle of the jabber's body.
fn jabber_middle(sim: &Simulation) -> WorldPos {
    sim.targets().iter().next().unwrap().pos.translate(Vec3::Z * 0.6)
}

/// The yaw and pitch that look from the player's eyes at `pos`.
fn aim_at(sim: &Simulation, pos: WorldPos) -> (f32, f32) {
    let d = sim.player().unwrap().eye().delta_to(pos);
    (d.y.atan2(d.x), d.z.atan2(d.truncate().length()))
}

/// Drops the player from `height` meters beside the jabber, which never attacks. With `swing`,
/// swings the club at it once it's in reach on the way down. Returns the strike, if one landed,
/// and what hurt the player by the time they've landed.
fn fall_past(height: f32, swing: bool, recoil: f32) -> (Option<Strike>, Vec<PlayerHurt>) {
    let mut sim = arena(0, 1);
    // The base club's hit, and falling into it adds damage.
    let club = &mut sim.tools_mut()[0];
    (club.damage, club.momentum_damage, club.recoil) = (5.0, 0.5, recoil);
    let beside = sim.targets().iter().next().unwrap().pos.translate(Vec3::new(1.2, 0.0, height));
    sim.teleport_player(beside);
    let tool = sim.tool_ids().next();
    let landed = sim.player().unwrap().landings();
    let (mut strike, mut hurts, mut pressed) = (None, Vec::new(), false);
    for _ in 0..600 {
        let (yaw, pitch) = aim_at(&sim, jabber_middle(&sim));
        // Pressed early enough that it's in reach when the wind-up ends, as a player would.
        let p = sim.player().unwrap();
        let ahead = p.eye().translate(p.velocity() * sim.tools()[0].windup);
        let near = ahead.delta_to(jabber_middle(&sim)).length() < 2.5;
        let press = swing && near && !pressed;
        pressed |= press;
        sim.step(&PlayerInput { yaw, pitch, tool, use_pressed: press, ..Default::default() });
        hurts.extend(sim.take_player_hurts());
        if let Some(s) = sim.take_strikes().into_iter().find(|s| s.hit.is_some()) {
            strike = Some(s);
        }
        if sim.player().unwrap().landings() != landed {
            hurts.extend(run(&mut sim, &PlayerInput::default(), 2));
            return (strike, hurts);
        }
    }
    panic!("never landed");
}

#[test]
fn a_hit_on_the_way_down_pushes_back_from_the_look_by_the_damage() {
    let (strike, _) = fall_past(12.0, true, RECOIL);
    let strike = strike.expect("hit it on the way down");
    let damage = strike.hit.as_ref().unwrap().damage;
    assert!(strike.look.z < -0.3, "looking down at it: {}", strike.look);
    let level = Vec3::new(strike.recoil.x, strike.recoil.y, 0.0);
    let wanted = -strike.look * RECOIL * damage;
    assert!((level - Vec3::new(wanted.x, wanted.y, 0.0)).length() < 1e-3, "back from the look: {}", strike.recoil);
    assert!(strike.recoil.z > 0.0 && strike.recoil.z <= wanted.z + 1e-3, "up, by up to the damage: {}", strike.recoil);
}

#[test]
fn recoil_never_leaves_the_player_rising_faster_than_the_bounce() {
    let mut sim = arena(0, 1);
    sim.tools_mut()[0].recoil = 50.0;
    // Just above it, falling slowly, swing down at it.
    let above = sim.targets().iter().next().unwrap().pos.translate(Vec3::new(1.0, 0.0, 1.5));
    sim.teleport_player(above);
    let tool = sim.tool_ids().next();
    let (yaw, pitch) = aim_at(&sim, jabber_middle(&sim));
    let mut input = PlayerInput { yaw, pitch, tool, use_pressed: true, ..Default::default() };
    let mut strike = None;
    for _ in 0..30 {
        sim.step(&input);
        input.use_pressed = false;
        if let Some(s) = sim.take_strikes().pop() {
            strike = Some(s);
            break;
        }
    }
    let strike = strike.expect("struck");
    assert!(strike.hit.is_some());
    sim.step(&PlayerInput::default());
    assert!(sim.player().unwrap().velocity().z <= BOUNCE + 1e-3, "{}", sim.player().unwrap().velocity());
}

#[test]
fn a_hit_on_the_ground_gives_no_recoil() {
    let mut sim = arena(0, 1);
    run(&mut sim, &PlayerInput::default(), 30);
    let tool = sim.tool_ids().next();
    let (yaw, pitch) = aim_at(&sim, jabber_middle(&sim));
    let mut input = PlayerInput { yaw, pitch, tool, use_pressed: true, ..Default::default() };
    for _ in 0..30 {
        sim.step(&input);
        input.use_pressed = false;
        if let Some(s) = sim.take_strikes().pop() {
            assert!(s.hit.is_some());
            assert_eq!(s.recoil, Vec3::ZERO);
            return;
        }
    }
    panic!("never struck");
}

#[test]
fn swinging_down_onto_something_on_the_way_down_cuts_the_fall_damage() {
    // The base club's recoil.
    let (_, plain) = fall_past(12.0, false, 0.8);
    let (strike, broken) = fall_past(12.0, true, 0.8);
    assert!(strike.is_some(), "hit it on the way down");
    let fall = |hurts: &[PlayerHurt]| hurts.iter().filter(|h| h.attacker.is_none()).map(|h| h.damage).sum::<f32>();
    assert!(fall(&plain) > 1.0, "a 12 m drop hurts: {}", fall(&plain));
    assert!(fall(&broken) < fall(&plain) * 0.5, "broken by the hit: {} against {}", fall(&broken), fall(&plain));
    // Without recoil the hit doesn't help.
    let (_, unbroken) = fall_past(12.0, true, 0.0);
    assert!((fall(&unbroken) - fall(&plain)).abs() < 0.5, "{} against {}", fall(&unbroken), fall(&plain));
}
