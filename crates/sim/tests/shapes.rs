//! Blocks that aren't full cubes, and boxels shared by several of them, through the simulation
//! with the base mod loaded. The base test area's shapes corner sits west of spawn.

use api::{ApiError, BoxelPos, I64Vec3, Orientation, Vec2, Vec3, WorldPos};
use sim::{Hit, Occupant, PlayerInput, Simulation};

fn base_sim() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mods: [&dyn api::Mod; 3] = [&data, &base::BaseMod, &base_movement::BaseMovementMod];
    let mut sim = Simulation::with_mods(&mods).expect("base mod loads");
    sim.spawn_player().expect("base registers a player");
    sim
}

fn id(sim: &Simulation, name: &str) -> sim::BlockId {
    sim.blocks().id(name).unwrap()
}

fn feet_at(sim: &mut Simulation, x: i64, y: i64, z: i64, offset: Vec3) {
    sim.teleport_player(WorldPos::new(I64Vec3::new(x, y, z), offset));
}

fn run(sim: &mut Simulation, input: &PlayerInput, ticks: usize) {
    for _ in 0..ticks {
        sim.step(input);
    }
}

/// The feet's height above zero.
fn feet_z(sim: &Simulation) -> f32 {
    let pos = sim.player().unwrap().pos();
    pos.cell().z as f32 + pos.offset().z
}

fn walk(yaw: f32) -> PlayerInput {
    PlayerInput { movement: Vec2::Y, yaw, ..Default::default() }
}

const EAST: f32 = 0.0;
const WEST: f32 = std::f32::consts::PI;
const DOWN: f32 = -89.0 * std::f32::consts::PI / 180.0;

#[test]
fn bodies_stand_on_slabs_at_half_height() {
    let mut sim = base_sim();
    feet_at(&mut sim, -10, -4, 2, Vec3::new(0.2, 0.5, 0.0));
    run(&mut sim, &PlayerInput::default(), 60);
    assert!(sim.player().unwrap().on_ground());
    assert!((feet_z(&sim) - 0.5).abs() < 1e-3, "feet at {}", feet_z(&sim));
}

#[test]
fn walking_west_climbs_the_stairs_without_jumping() {
    let mut sim = base_sim();
    let (x, y, z) = base::SHAPES_SPOT;
    feet_at(&mut sim, x, y, z, Vec3::new(0.5, 0.5, 0.0));
    run(&mut sim, &PlayerInput::default(), 10);
    // Walk until the body is wholly on the platform, which ends at x = -12.
    let x = |sim: &Simulation| sim.player().unwrap().pos().cell().x as f32 + sim.player().unwrap().pos().offset().x;
    for _ in 0..300 {
        if x(&sim) < -9.5 {
            break;
        }
        sim.step(&walk(WEST));
    }
    assert!(x(&sim) < -9.5, "reached the platform, at x {}", x(&sim));
    assert!((feet_z(&sim) - 3.0).abs() < 1e-3, "standing on top, at {}", feet_z(&sim));
}

#[test]
fn walking_into_a_fence_post_stops_at_the_post() {
    let mut sim = base_sim();
    // Walk east into the lone post at (-9, -7), from x = -11.
    feet_at(&mut sim, -11, -7, 0, Vec3::new(0.5, 0.5, 0.0));
    run(&mut sim, &PlayerInput::default(), 10);
    run(&mut sim, &walk(EAST), 60);
    let pos = sim.player().unwrap().pos();
    let front = pos.cell().x as f32 + pos.offset().x + 0.3;
    assert!((front - (-9.0 + 6.0 / 16.0)).abs() < 1e-3, "front of the body at {front}");
}

/// A hit on a box face at `point` in boxel space.
fn hit(pos: BoxelPos, normal: I64Vec3, point: Vec3) -> Hit {
    Hit { pos, occupant: 0, normal, distance: 1.0, point }
}

#[test]
fn a_slab_on_a_bottom_slab_makes_a_double_slab() {
    let sim = base_sim();
    let slab = id(&sim, base::BRICK_SLAB);
    let cell = BoxelPos::new(-8, -4, 0);
    let place = sim.placement(&hit(cell, I64Vec3::Z, Vec3::new(0.5, 0.5, 0.5)), slab, EAST).unwrap();
    assert!(place.allowed);
    assert_eq!(place.pos, cell, "the top half of the same boxel");
    assert_eq!(place.occupant, Occupant::turned(slab, Orientation::quarter_turns(0, 2)));
}

#[test]
fn a_full_block_on_a_slab_goes_above_it() {
    let sim = base_sim();
    let stone = id(&sim, base::STONE);
    let cell = BoxelPos::new(-8, -4, 0);
    let place = sim.placement(&hit(cell, I64Vec3::Z, Vec3::new(0.5, 0.5, 0.5)), stone, EAST).unwrap();
    assert!(place.allowed);
    assert_eq!(place.pos, BoxelPos::new(-8, -4, 1));
}

#[test]
fn slabs_take_the_half_they_are_aimed_at() {
    let sim = base_sim();
    let slab = id(&sim, base::STONE_SLAB);
    let floor = BoxelPos::new(-14, 0, -1);
    // On top of the floor: the bottom of the boxel above.
    let place = sim.placement(&hit(floor, I64Vec3::Z, Vec3::new(0.5, 0.5, 1.0)), slab, EAST).unwrap();
    assert_eq!((place.pos, place.occupant.orientation), (BoxelPos::new(-14, 0, 0), Orientation::IDENTITY));
    // On the side of the platform, high up: the top half.
    let platform = BoxelPos::new(-10, 1, 2);
    let place = sim.placement(&hit(platform, I64Vec3::Y, Vec3::new(0.5, 1.0, 0.8)), slab, EAST).unwrap();
    assert_eq!(place.pos, BoxelPos::new(-10, 2, 2));
    assert_eq!(place.occupant.orientation, Orientation::quarter_turns(0, 2));
}

#[test]
fn a_slab_aimed_at_a_post_shares_the_post_boxel() {
    let mut sim = base_sim();
    let slab = id(&sim, base::STONE_SLAB);
    // A post on its own, then a slab aimed at the post's east side.
    let fence = id(&sim, base::WOOD_FENCE);
    let cell = BoxelPos::new(-14, 2, 0);
    let post_hit = hit(BoxelPos::new(-14, 2, -1), I64Vec3::Z, Vec3::new(0.5, 0.5, 1.0));
    let place = sim.placement(&post_hit, fence, EAST).unwrap();
    assert_eq!(place.pos, cell);
    // Placing for real goes through the tick, so aim at the floor from above.
    feet_at(&mut sim, -14, 2, 2, Vec3::new(0.5, 0.5, 0.0));
    sim.step(&PlayerInput { toggle_fly: true, ..Default::default() });
    let place_input = |block| PlayerInput {
        pitch: DOWN,
        place_pressed: true,
        place_held: true,
        place_block: block,
        ..Default::default()
    };
    run(&mut sim, &place_input(fence), 1);
    assert_eq!(sim.universe().main().cell(cell), [Occupant::new(fence)]);

    let side = hit(cell, I64Vec3::X, Vec3::new(10.0 / 16.0, 0.5, 0.25));
    let place = sim.placement(&side, slab, EAST).unwrap();
    assert!(place.allowed);
    assert_eq!(place.pos, cell);
    assert_eq!(place.occupant, Occupant::new(slab));
}

#[test]
fn stairs_rise_the_way_the_player_looks() {
    let sim = base_sim();
    let stairs = id(&sim, base::STONE_STAIRS);
    let on_floor = hit(BoxelPos::new(-14, 0, -1), I64Vec3::Z, Vec3::new(0.5, 0.5, 1.0));
    for (yaw, rises) in [(EAST, [1, 0, 0]), (WEST, [-1, 0, 0]), (1.5, [0, 1, 0]), (-1.6, [0, -1, 0])] {
        let o = sim.placement(&on_floor, stairs, yaw).unwrap().occupant.orientation;
        assert_eq!(o.apply([0, 1, 0]), rises, "looking along {yaw}");
        assert_eq!(o.apply([0, 0, 1]), [0, 0, 1], "right way up");
    }
    // Aimed at the upper half of a wall: upside down, still rising the way the player looks.
    let wall = hit(BoxelPos::new(-10, 1, 2), I64Vec3::Y, Vec3::new(0.5, 1.0, 0.8));
    let o = sim.placement(&wall, stairs, 1.5).unwrap().occupant.orientation;
    assert_eq!((o.apply([0, 1, 0]), o.apply([0, 0, 1])), ([0, 1, 0], [0, 0, -1]));
}

#[test]
fn logs_line_up_with_the_face_aimed_at() {
    let sim = base_sim();
    let log = id(&sim, base::LOG);
    let platform = BoxelPos::new(-10, 0, 1);
    for normal in [I64Vec3::X, I64Vec3::Y, I64Vec3::Z, I64Vec3::NEG_Y] {
        let point = Vec3::splat(0.5) + normal.as_vec3() * 0.5;
        let o = sim.placement(&hit(platform, normal, point), log, EAST).unwrap().occupant.orientation;
        let along = o.apply([0, 0, 1]).map(|c| c.abs());
        let expected = normal.abs().to_array().map(|c| c as i32);
        assert_eq!(along, expected, "aimed at the face toward {normal}");
    }
}

#[test]
fn a_refused_placement_still_says_where_it_would_go() {
    let sim = base_sim();
    let slab = id(&sim, base::BRICK_SLAB);
    // The post at x = -12 shares its boxel with a bottom slab, and so does the boxel east of it,
    // so a bottom slab aimed low at the post's east side has nowhere to go.
    let cell = BoxelPos::new(-12, -4, 0);
    let side = hit(cell, I64Vec3::X, Vec3::new(10.0 / 16.0, 0.5, 0.25));
    let place = sim.placement(&side, slab, EAST).unwrap();
    assert!(!place.allowed);
    assert_eq!(place.pos, cell);
}

#[test]
fn breaking_removes_only_the_occupant_aimed_at() {
    let mut sim = base_sim();
    let slab = id(&sim, base::STONE_SLAB);
    let cell = BoxelPos::new(-6, -4, 0);
    assert_eq!(sim.universe().main().cell(cell).len(), 2, "the double slab");
    // From straight above, the top slab is what the ray meets.
    feet_at(&mut sim, -6, -4, 3, Vec3::new(0.5, 0.5, 0.0));
    sim.step(&PlayerInput { toggle_fly: true, ..Default::default() });
    let hit = sim.target(0.0, DOWN).unwrap();
    assert_eq!(hit.pos, cell);
    assert!((hit.point.z - 1.0).abs() < 1e-4);
    sim.step(&PlayerInput { pitch: DOWN, break_pressed: true, break_held: true, ..Default::default() });
    assert_eq!(sim.universe().main().cell(cell), [Occupant::new(slab)], "the bottom slab stays");
    let hit = sim.target(0.0, DOWN).unwrap();
    assert!((hit.point.z - 0.5).abs() < 1e-4, "now the bottom slab's top is hit");
}

#[test]
fn world_building_refuses_blocks_that_cannot_share() {
    struct Crowd;
    impl api::Mod for Crowd {
        fn name(&self) -> &str {
            "crowd"
        }
        fn register(&self, _: &mut dyn api::Registrar) -> Result<(), ApiError> {
            Ok(())
        }
        fn build_world(&self, w: &mut dyn api::WorldEdit) -> Result<(), ApiError> {
            // Floor stone is full, so a slab can't join it.
            w.add_block(BoxelPos::new(0, 0, -1), base::STONE_SLAB, Orientation::IDENTITY)
        }
    }
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mods: [&dyn api::Mod; 3] = [&data, &base::BaseMod, &Crowd];
    assert!(matches!(
        Simulation::with_mods(&mods),
        Err(sim::SimError::Api(ApiError::CannotShare(_)))
    ));
}
