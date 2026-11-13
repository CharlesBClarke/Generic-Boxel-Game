//! Loading by cube, on a terrain made for it: rock below z = 0 with a shaft that never ends, a
//! sealed room a kilometer down and a roof 300 boxels up. The world loads around the player
//! however deep or high they are, the dark stays dark at the edge of what's loaded, and what
//! isn't loaded overhead still shades.

use api::{
    ApiError, BlockDef, BoxelPos, CHUNK_SIZE, ChunkFill, ChunkPos, Generator, I64Vec3, InteractionDef, Mod, NO_TOP, PlayerDef, Registrar,
    TerrainDef, Tunable, Tunables, Vec3, WorldEdit, WorldPos,
};
use sim::{Light, PlayerInput, Simulation};

/// Rock below z = 0, with three things cut into or set over it.
struct Burrows;

/// A shaft two boxels square, open all the way down and up.
fn in_shaft(x: i64, y: i64) -> bool {
    (0..2).contains(&x) && (0..2).contains(&y)
}

/// A sealed room far underground.
fn in_room(p: I64Vec3) -> bool {
    (100..110).contains(&p.x) && (0..10).contains(&p.y) && (ROOM_FLOOR..ROOM_FLOOR + 4).contains(&p.z)
}

const ROOM_FLOOR: i64 = -1000;

/// A roof one boxel thick, high over open ground.
fn under_roof(x: i64, y: i64) -> bool {
    (200..264).contains(&x) && (0..64).contains(&y)
}

const ROOF: i64 = 300;

impl Tunables for Burrows {
    fn tunables(&self) -> Vec<Tunable> {
        Vec::new()
    }
    fn set_tunable(&mut self, _: &str, _: f32) -> bool {
        false
    }
}

impl Generator for Burrows {
    fn fill(&self, origin: I64Vec3, out: &mut ChunkFill) {
        let stone = out.entry("deep:stone");
        let n = CHUNK_SIZE as usize;
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    let p = origin + I64Vec3::new(x as i64, y as i64, z as i64);
                    let rock = p.z < 0 && !in_shaft(p.x, p.y) && !in_room(p);
                    if rock || (p.z == ROOF && under_roof(p.x, p.y)) {
                        out.set(x, y, z, stone);
                    }
                }
            }
        }
    }

    fn tops(&self, x: i64, y: i64, step: u32, out: &mut [i64]) {
        for (i, top) in out.iter_mut().enumerate() {
            let (px, py) = (x + (i as i64 % CHUNK_SIZE) * step as i64, y + (i as i64 / CHUNK_SIZE) * step as i64);
            *top = if in_shaft(px, py) {
                NO_TOP
            } else if under_roof(px, py) {
                ROOF
            } else {
                -1
            };
        }
    }
}

struct Deep;

impl Mod for Deep {
    fn name(&self) -> &str {
        "deep"
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        r.register_block(BlockDef::new("deep:stone", true, [0, 0, 0]))?;
        r.register_movement_controller(base_movement::QUAKE, Box::new(base_movement::QuakeController::default()))?;
        r.register_player(PlayerDef {
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            step_height: 0.0,
            controller: base_movement::QUAKE.into(),
            sounds: Default::default(),
        })?;
        r.register_interaction(InteractionDef { reach: 5.0, repeat_rate: 4.0 })?;
        r.register_generator("deep:burrows", Box::new(Burrows))
    }

    fn build_world(&self, w: &mut dyn WorldEdit) -> Result<(), ApiError> {
        w.set_spawn(WorldPos::new(I64Vec3::new(50, 50, 0), Vec3::new(0.5, 0.5, 0.0)));
        w.add_terrain(TerrainDef { generator: "deep:burrows".into(), center: BoxelPos::new(0, 0, 0) })
    }
}

/// The world, keeping 64 m loaded around the player, who stands at `at`.
fn world_at(at: I64Vec3) -> Simulation {
    let mut sim = Simulation::with_mods(&[&Deep]).unwrap();
    sim.spawn_player().unwrap();
    sim.set_view_distance(32.0, 0.0);
    sim.teleport_player(WorldPos::new(at, Vec3::new(0.5, 0.5, 0.0)));
    sim.generate_terrain().unwrap();
    sim.stream_all();
    sim
}

fn sky(sim: &Simulation, x: i64, y: i64, z: i64) -> u8 {
    sim.universe().main().light(BoxelPos::new(x, y, z)).sky()
}

fn loaded_rows(sim: &Simulation) -> (i64, i64) {
    let rows = || sim.universe().main().chunks().map(|(pos, _)| pos.0.z);
    (rows().min().unwrap(), rows().max().unwrap())
}

#[test]
fn a_room_a_kilometer_down_loads_around_the_player_and_is_dark() {
    let sim = world_at(I64Vec3::new(105, 5, ROOM_FLOOR));
    let player = sim.player().unwrap().pos();
    assert_eq!(player.cell().z, ROOM_FLOOR, "standing in the room, not lifted out of the ground");
    assert!(!sim.awaiting_ground(player));
    // Only what's around the room is loaded: nothing near the surface.
    let (low, high) = loaded_rows(&sim);
    assert!(high < -25 && low > -37, "rows {low} to {high}");
    assert_eq!(sim.stream_stats().waiting, 0);
    // The room is open and its walls are rock.
    let grid = sim.universe().main();
    assert!(grid.cell(BoxelPos::new(105, 5, ROOM_FLOOR + 1)).is_empty());
    assert!(!grid.cell(BoxelPos::new(105, 5, ROOM_FLOOR - 1)).is_empty());
    // No light anywhere that's loaded, the edge of it included, where chunks that aren't
    // loaded stand on every side: they're under the ground's top, so they're dark walls.
    for (pos, chunk) in grid.chunks() {
        for i in 0..api::CHUNK_VOLUME {
            assert_eq!(chunk.light.at(i), Light::DARK, "chunk {:?}, boxel {i}", pos.0.to_array());
        }
    }
}

#[test]
fn a_shaft_with_no_end_is_lit_from_a_sky_that_isnt_loaded() {
    // Deep in rock beside the shaft: the player is lifted to the top once the rock comes in,
    // so stand them in the shaft itself, flying.
    let mut sim = Simulation::with_mods(&[&Deep]).unwrap();
    sim.spawn_player().unwrap();
    sim.set_view_distance(32.0, 0.0);
    sim.player_mut().unwrap().set_flying(true);
    sim.teleport_player(WorldPos::new(I64Vec3::new(0, 0, -600), Vec3::new(0.9, 0.9, 0.0)));
    sim.generate_terrain().unwrap();
    sim.stream_all();
    assert_eq!(sim.player().unwrap().pos().cell().z, -600, "in the shaft, not lifted");
    let (_, high) = loaded_rows(&sim);
    assert!(high < -15, "the surface isn't loaded: rows up to {high}");
    // The sky comes straight down the shaft through everything that isn't loaded, undimmed,
    // and the rock beside it is dark.
    for z in [-640, -600, -570] {
        assert_eq!(sky(&sim, 0, 0, z), Light::MAX, "in the shaft at {z}");
        assert_eq!(sky(&sim, 1, 1, z), Light::MAX);
        assert_eq!(sky(&sim, 2, 0, z), 0, "in the rock beside it at {z}");
    }
    // Digging a side passage lets it in, a step dimmer each boxel.
    let grid = sim.universe().main();
    assert!(!grid.cell(BoxelPos::new(2, 0, -600)).is_empty());
    let yaw = 0.0;
    let eye = sim.player().unwrap().eye();
    let to = eye.delta_to(WorldPos::new(I64Vec3::new(2, 0, -600), Vec3::new(0.0, 0.5, 0.5)));
    let pitch = to.z.atan2(to.truncate().length());
    sim.step(&PlayerInput { yaw, pitch, use_pressed: true, use_held: true, ..Default::default() });
    assert!(sim.universe().main().cell(BoxelPos::new(2, 0, -600)).is_empty(), "dug into the wall");
    assert_eq!(sky(&sim, 2, 0, -600), Light::MAX - Light::SKY_STEP);
    // The passage is under the rock above it: its top hasn't moved, only the shaft has none.
    let tops = &sim.universe().main().tops;
    assert_eq!((tops.top(0, 0), tops.top(2, 0)), (Some(NO_TOP), Some(-1)));
}

#[test]
fn a_roof_far_overhead_shades_the_ground_without_being_loaded() {
    // On the ground under the middle of the roof, which is 300 boxels up: well past the 64 m
    // kept loaded.
    let sim = world_at(I64Vec3::new(232, 32, 0));
    let (_, high) = loaded_rows(&sim);
    assert!(high * CHUNK_SIZE < ROOF - 100, "the roof isn't loaded: rows up to {high}");
    assert_eq!(sky(&sim, 232, 32, 0), 0, "under the middle of the roof");
    assert_eq!(sky(&sim, 232, 32, 40), 0, "and in the air under it");
    // Past its edge the ground is open.
    let sim = world_at(I64Vec3::new(190, 32, 0));
    assert_eq!(sky(&sim, 190, 32, 0), Light::MAX, "open ground beside it");
    assert_eq!(sky(&sim, 200, 32, 0), Light::MAX - Light::SKY_STEP, "just under its edge");
    assert_eq!(sky(&sim, 215, 32, 0), 0, "too far under for any to reach");
}

#[test]
fn the_world_loads_as_high_as_the_player_goes() {
    let mut sim = Simulation::with_mods(&[&Deep]).unwrap();
    sim.spawn_player().unwrap();
    sim.set_view_distance(32.0, 0.0);
    sim.player_mut().unwrap().set_flying(true);
    sim.teleport_player(WorldPos::new(I64Vec3::new(232, 32, 5000), Vec3::new(0.5, 0.5, 0.0)));
    sim.generate_terrain().unwrap();
    sim.stream_all();
    let (low, high) = loaded_rows(&sim);
    assert!(low > 150 && high < 160, "rows {low} to {high}");
    let here = sim.player().unwrap().pos().boxel().chunk();
    assert!(sim.is_terrain(here) && sim.is_terrain(ChunkPos(here.0 + I64Vec3::Z)) && sim.is_terrain(ChunkPos(here.0 - I64Vec3::Z)));
    assert_eq!(sky(&sim, 232, 32, 5000), Light::MAX);
    // Down to the roof: the chunks holding it load, and it shades what's under it.
    sim.teleport_player(WorldPos::new(I64Vec3::new(232, 32, ROOF + 2), Vec3::new(0.5, 0.5, 0.0)));
    sim.stream_all();
    assert!(!sim.universe().main().cell(BoxelPos::new(232, 32, ROOF)).is_empty());
    assert_eq!(sky(&sim, 232, 32, ROOF + 1), Light::MAX);
    assert_eq!(sky(&sim, 232, 32, ROOF - 1), 0);
}

#[test]
fn a_body_waits_over_a_chunk_that_isnt_loaded() {
    let sim = world_at(I64Vec3::new(50, 50, 0));
    let stand = sim.player().unwrap().pos();
    assert!(!sim.awaiting_ground(stand));
    // Far below what's loaded, and far above: still to come.
    assert!(sim.awaiting_ground(WorldPos::at_boxel(BoxelPos::new(50, 50, -500))));
    assert!(sim.awaiting_ground(WorldPos::at_boxel(BoxelPos::new(50, 50, 500))));
    // At the foot of a loaded chunk over one that isn't, it waits; a little higher it doesn't.
    let (low, _) = loaded_rows(&sim);
    let column: Vec<i64> = sim.universe().main().chunks().map(|(p, _)| p.0).filter(|p| p.x == 1 && p.y == 1).map(|p| p.z).collect();
    let lowest = *column.iter().min().unwrap();
    assert!(lowest >= low);
    let foot = BoxelPos::new(50, 50, lowest * CHUNK_SIZE);
    assert!(sim.awaiting_ground(WorldPos::at_boxel(foot)));
    assert!(!sim.awaiting_ground(WorldPos::at_boxel(foot.offset(I64Vec3::new(0, 0, 8)))));
}

#[test]
fn a_world_takes_one_terrain() {
    struct Two;
    impl Mod for Two {
        fn name(&self) -> &str {
            "two"
        }
        fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
            Deep.register(r)
        }
        fn build_world(&self, w: &mut dyn WorldEdit) -> Result<(), ApiError> {
            Deep.build_world(w)?;
            w.add_terrain(TerrainDef { generator: "deep:burrows".into(), center: BoxelPos::new(0, 0, 3200) })
        }
    }
    let Err(error) = Simulation::with_mods(&[&Two]) else { panic!("a second terrain is refused") };
    assert!(format!("{error:?}").contains("a second terrain"), "{error:?}");
}
