//! Blocks that aren't full cubes, and boxels shared by several of them, through the simulation
//! with the base mod loaded. The base test area's shapes corner sits west of spawn.

use api::{ApiError, BlockBox, BoxelPos, I64Vec3, Orientation, Vec2, Vec3, WorldPos};
use sim::{Hit, Occupant, PlayerInput, Simulation};

mod common;

fn base_sim() -> Simulation {
    let data = common::AllPlaceable::load();
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

/// A hit on the middle of a full boxel's face.
fn face(pos: BoxelPos, normal: I64Vec3) -> Hit {
    hit(pos, normal, Vec3::splat(0.5) + normal.as_vec3() * 0.5)
}

/// The space a placed occupant fills, in sixteenths.
fn space(sim: &Simulation, o: Occupant) -> Vec<BlockBox> {
    let def = sim.blocks().get(o.block).unwrap();
    def.shape.iter().map(|&b| o.orientation.apply_box(b)).collect()
}

const LEVEL: f32 = 0.0;
const BASIC: bool = false;
const PRECISE: bool = true;

const BOTTOM_SLAB: BlockBox = BlockBox::new([0, 0, 0], [16, 16, 8]);
const TOP_SLAB: BlockBox = BlockBox::new([0, 0, 8], [16, 16, 16]);

/// The north side of the platform, a wall facing north, aimed at height `z` and across at `x`.
fn wall_at(x: f32, z: f32) -> Hit {
    hit(BoxelPos::new(-10, 1, 2), I64Vec3::Y, Vec3::new(x, 1.0, z))
}

/// The floor west of the shapes corner, aimed at `x`, `y` across its top face.
fn floor_at(x: f32, y: f32) -> Hit {
    hit(BoxelPos::new(-14, 0, -1), I64Vec3::Z, Vec3::new(x, y, 1.0))
}

#[test]
fn a_slab_aimed_at_the_top_of_a_slab_fills_the_top_half() {
    let sim = base_sim();
    let cell = BoxelPos::new(-8, -4, 0);
    let top = hit(cell, I64Vec3::Z, Vec3::new(0.5, 0.5, 0.5));
    // The same material or another one, in either mode.
    for name in [base::BRICK_SLAB, base::STONE_SLAB] {
        for precise in [BASIC, PRECISE] {
            let place = sim.placement(&top, id(&sim, name), EAST, -0.5, precise).unwrap();
            assert!(place.allowed, "{name}");
            assert_eq!(place.pos, cell, "{name}");
            assert_eq!(space(&sim, place.occupant), [TOP_SLAB], "{name}, precise {precise}");
        }
    }
}

#[test]
fn two_blocks_never_fill_the_same_space() {
    let sim = base_sim();
    // A stone slab aimed at the floor under the brick slab path would fill the brick slab's
    // space, so it goes to the boxel on the far side of the face, which is that same boxel.
    let floor = face(BoxelPos::new(-10, -4, -1), I64Vec3::Z);
    let place = sim.placement(&floor, id(&sim, base::STONE_SLAB), EAST, -0.5, BASIC).unwrap();
    assert!(!place.allowed);
}

#[test]
fn a_slab_against_a_ceiling_over_a_slab_makes_a_double_slab() {
    let sim = base_sim();
    let slab = id(&sim, base::BRICK_SLAB);
    // Aimed up at the underside of the boxel above the slab at (-8, -4, 0).
    let ceiling = face(BoxelPos::new(-8, -4, 1), I64Vec3::NEG_Z);
    let place = sim.placement(&ceiling, slab, EAST, 0.5, BASIC).unwrap();
    assert!(place.allowed);
    assert_eq!(place.pos, BoxelPos::new(-8, -4, 0));
    assert_eq!(space(&sim, place.occupant), [TOP_SLAB]);
}

#[test]
fn a_full_block_on_a_slab_goes_above_it() {
    let sim = base_sim();
    let stone = id(&sim, base::STONE);
    let cell = BoxelPos::new(-8, -4, 0);
    let place = sim.placement(&hit(cell, I64Vec3::Z, Vec3::new(0.5, 0.5, 0.5)), stone, EAST, LEVEL, BASIC).unwrap();
    assert!(place.allowed);
    assert_eq!(place.pos, BoxelPos::new(-8, -4, 1));
}

#[test]
fn basic_slabs_take_the_top_or_bottom_like_minecraft() {
    let sim = base_sim();
    let slab = id(&sim, base::STONE_SLAB);
    let place = sim.placement(&floor_at(0.5, 0.5), slab, EAST, -0.5, BASIC).unwrap();
    assert_eq!((place.pos, space(&sim, place.occupant)), (BoxelPos::new(-14, 0, 0), vec![BOTTOM_SLAB]));
    // On a wall: the half aimed at, wherever across the wall.
    for x in [0.1, 0.5, 0.9] {
        let low = sim.placement(&wall_at(x, 0.2), slab, -1.6, LEVEL, BASIC).unwrap();
        assert_eq!((low.pos, space(&sim, low.occupant)), (BoxelPos::new(-10, 2, 2), vec![BOTTOM_SLAB]));
        let high = sim.placement(&wall_at(x, 0.8), slab, -1.6, LEVEL, BASIC).unwrap();
        assert_eq!(space(&sim, high.occupant), [TOP_SLAB]);
    }
}

#[test]
fn precise_slabs_go_where_the_face_is_aimed() {
    let sim = base_sim();
    let slab = id(&sim, base::STONE_SLAB);
    let above = BoxelPos::new(-14, 0, 0);
    // The floor: the middle lays it flat; each triangle stands it against that side.
    for (x, y, expected) in [
        (0.5, 0.5, BOTTOM_SLAB),
        (0.6, 0.95, BlockBox::new([0, 8, 0], [16, 16, 16])),
        (0.5, 0.05, BlockBox::new([0, 0, 0], [16, 8, 16])),
        (0.9, 0.6, BlockBox::new([8, 0, 0], [16, 16, 16])),
        (0.1, 0.3, BlockBox::new([0, 0, 0], [8, 16, 16])),
    ] {
        let place = sim.placement(&floor_at(x, y), slab, EAST, -0.5, PRECISE).unwrap();
        assert_eq!(place.pos, above);
        assert_eq!(space(&sim, place.occupant), [expected], "aimed at {x}, {y}");
    }
    // A wall: the middle stands it against the wall, the top and bottom triangles make level
    // slabs, the side triangles stand it against that side.
    let into = BoxelPos::new(-10, 2, 2);
    for (x, z, expected) in [
        (0.5, 0.5, BlockBox::new([0, 0, 0], [16, 8, 16])),
        (0.5, 0.9, TOP_SLAB),
        (0.5, 0.1, BOTTOM_SLAB),
        (0.95, 0.5, BlockBox::new([8, 0, 0], [16, 16, 16])),
    ] {
        let place = sim.placement(&wall_at(x, z), slab, -1.6, LEVEL, PRECISE).unwrap();
        assert_eq!(place.pos, into);
        assert_eq!(space(&sim, place.occupant), [expected], "aimed at {x}, {z}");
    }
}

#[test]
fn a_slab_aimed_at_a_post_shares_the_post_boxel() {
    let mut sim = base_sim();
    let slab = id(&sim, base::STONE_SLAB);
    // A post on its own, then a slab aimed at the post's east side.
    let fence = id(&sim, base::WOOD_FENCE);
    let cell = BoxelPos::new(-14, 2, 0);
    let place = sim.placement(&face(BoxelPos::new(-14, 2, -1), I64Vec3::Z), fence, EAST, LEVEL, BASIC).unwrap();
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

    let side = |z| hit(cell, I64Vec3::X, Vec3::new(10.0 / 16.0, 0.5, z));
    let place = sim.placement(&side(0.25), slab, WEST, LEVEL, BASIC).unwrap();
    assert!(place.allowed);
    assert_eq!((place.pos, space(&sim, place.occupant)), (cell, vec![BOTTOM_SLAB]));
    // Precise, in the middle: aimed from inside the post's boxel, so it stands against the
    // boxel's far (east) side.
    let place = sim.placement(&side(0.5), slab, WEST, LEVEL, PRECISE).unwrap();
    assert_eq!(space(&sim, place.occupant), [BlockBox::new([8, 0, 0], [16, 16, 16])]);
}

#[test]
fn basic_stairs_rise_the_way_the_player_looks() {
    let sim = base_sim();
    let stairs = id(&sim, base::STONE_STAIRS);
    for (yaw, rises) in [(EAST, [1, 0, 0]), (WEST, [-1, 0, 0]), (1.5, [0, 1, 0]), (-1.6, [0, -1, 0])] {
        let o = sim.placement(&floor_at(0.5, 0.5), stairs, yaw, -0.5, BASIC).unwrap().occupant.orientation;
        assert_eq!((o.apply([0, 1, 0]), o.apply([0, 0, 1])), (rises, [0, 0, 1]), "looking along {yaw}");
    }
    // Against a ceiling or high on a wall: upside down, still rising the way the player looks.
    let ceiling = face(BoxelPos::new(-14, 0, 3), I64Vec3::NEG_Z);
    let o = sim.placement(&ceiling, stairs, 1.5, 0.5, BASIC).unwrap().occupant.orientation;
    assert_eq!((o.apply([0, 1, 0]), o.apply([0, 0, 1])), ([0, 1, 0], [0, 0, -1]));
    let o = sim.placement(&wall_at(0.5, 0.8), stairs, -1.6, LEVEL, BASIC).unwrap().occupant.orientation;
    assert_eq!((o.apply([0, 1, 0]), o.apply([0, 0, 1])), ([0, -1, 0], [0, 0, -1]));
}

#[test]
fn precise_stairs_lie_against_the_middle_or_stand_back_to_the_face() {
    let sim = base_sim();
    let stairs = id(&sim, base::STONE_STAIRS);
    // The wall's middle, looking up along it: bottom against the wall, rising upward.
    let o = sim.placement(&wall_at(0.5, 0.5), stairs, -1.6, 0.8, PRECISE).unwrap().occupant.orientation;
    assert_eq!((o.apply([0, 1, 0]), o.apply([0, 0, 1])), ([0, 0, 1], [0, 1, 0]));
    // The wall's bottom triangle: upright, tall back against the wall to the south.
    let o = sim.placement(&wall_at(0.5, 0.1), stairs, -1.6, LEVEL, PRECISE).unwrap().occupant.orientation;
    assert_eq!((o.apply([0, 1, 0]), o.apply([0, 0, 1])), ([0, -1, 0], [0, 0, 1]));
}

#[test]
fn logs_line_up_with_the_face_in_basic_and_with_the_triangle_in_precise() {
    let sim = base_sim();
    let log = id(&sim, base::LOG);
    let platform = BoxelPos::new(-10, 0, 1);
    for normal in [I64Vec3::X, I64Vec3::Y, I64Vec3::Z, I64Vec3::NEG_Y] {
        let o = sim.placement(&face(platform, normal), log, 0.7, 0.3, BASIC).unwrap().occupant.orientation;
        let expected = normal.abs().to_array().map(|c| c as i32);
        assert_eq!(o.apply([0, 0, 1]).map(|c| c.abs()), expected, "aimed at the face toward {normal}");
    }
    // Precise on the floor's east triangle: lying east to west.
    let o = sim.placement(&floor_at(0.9, 0.5), log, EAST, -0.5, PRECISE).unwrap().occupant.orientation;
    assert_eq!(o.apply([0, 0, 1]).map(|c| c.abs()), [1, 0, 0]);
}

#[test]
fn plain_blocks_never_turn() {
    let sim = base_sim();
    let stone = id(&sim, base::STONE);
    for (aim, precise) in [(wall_at(0.5, 0.8), BASIC), (wall_at(0.9, 0.5), PRECISE), (floor_at(0.1, 0.5), PRECISE)] {
        let place = sim.placement(&aim, stone, -1.6, 0.8, precise).unwrap();
        assert_eq!(place.occupant, Occupant::new(stone));
    }
}

#[test]
fn a_refused_placement_still_says_where_it_would_go() {
    let sim = base_sim();
    let slab = id(&sim, base::BRICK_SLAB);
    // A bottom slab aimed at the floor under the slab path has the same slab in the way, and
    // the boxel on the far side of the face is that same boxel.
    let place = sim.placement(&face(BoxelPos::new(-11, -4, -1), I64Vec3::Z), slab, EAST, -0.5, BASIC).unwrap();
    assert!(!place.allowed);
    assert_eq!(place.pos, BoxelPos::new(-11, -4, 0));
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

#[test]
fn crouching_on_the_stairs_never_drops_more_than_the_guard_allows() {
    // Wiggle about crouched on the staircase near its north side, which drops to the floor.
    // Stepping down a stair is allowed (half a boxel); falling off the side is not.
    let guard = 0.6;
    for seed in 0..8u32 {
        let mut sim = base_sim();
        feet_at(&mut sim, -8, 1, 3, Vec3::new(0.3, 0.7, 0.0));
        let crouch = PlayerInput { crouch: true, ..Default::default() };
        run(&mut sim, &crouch, 60);
        let mut rng = seed.wrapping_mul(2_654_435_761).wrapping_add(12_345);
        let mut left_ground = None;
        for tick in 0..900 {
            if tick % 4 == 0 {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
            }
            let yaw = (rng % 8) as f32 * std::f32::consts::FRAC_PI_4;
            sim.step(&PlayerInput { movement: Vec2::Y, yaw, crouch: true, ..Default::default() });
            let (on_ground, z) = (sim.player().unwrap().on_ground(), feet_z(&sim));
            match (on_ground, left_ground) {
                (false, None) => left_ground = Some(z),
                (true, Some(from)) => {
                    let drop = from - z;
                    assert!(drop <= guard + 1e-3, "seed {seed}, tick {tick}: fell {drop} while crouched");
                    left_ground = None;
                }
                _ => {}
            }
        }
    }
}
