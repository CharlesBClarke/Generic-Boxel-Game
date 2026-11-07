//! The base game, written as a mod on the public API only. This is its code half; its blocks
//! and their looks are a data mod in `mods/base`, loaded by the engine alongside it.

use api::{
    ApiError, BoxelPos, CrowdDef, HealthDef, I64Vec3, InteractionDef, ItemDropDef, Mod, Orientation, PlayerDef, PlayerSounds, PushOutDef, Registrar, SignalDef,
    TerrainDef, ToolDef, ToolSounds, Vec3, WorldEdit, WorldPos,
};

pub mod shapes;
pub mod terrain;

/// The base game's data half, in the source tree. Both halves share the `base` namespace.
pub const DATA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../mods/base");

// Block names, defined in `mods/base/blocks.toml`.
pub const STONE: &str = "base:stone";
pub const DIRT: &str = "base:dirt";
pub const GRASS: &str = "base:grass";
pub const BRICK: &str = "base:brick";
pub const MARKER: &str = "base:marker";
pub const ICE: &str = "base:ice";
pub const WOOD: &str = "base:wood";
pub const LOG: &str = "base:log";
pub const GRAVEL: &str = "base:gravel";
pub const CRAFTING_TABLE: &str = "base:crafting_table";
/// Brass: a trigger, for knobs and levers.
pub const BRASS: &str = "base:brass";
/// Iron: a plain metal.
pub const IRON: &str = "base:iron";
/// Copper: a conductor, a wire carrying signals on.
pub const COPPER: &str = "base:copper";
/// Phasite, make-believe: a signal flips it between shown and hidden, taking what touches it.
pub const PHASITE: &str = "base:phasite";
/// Antiphasite: phasite that starts hidden, for what a signal brings in, like an open door.
pub const ANTIPHASITE: &str = "base:antiphasite";

/// The cow, defined in `mods/base/targets.toml`, with its brain in the `base_cow` crate.
pub const COW: &str = "base:cow";
/// The melee tool.
pub const CLUB: &str = "base:club";

/// Where the cows stand in the pasture, relative to the test area's origin, and which way each
/// faces, in degrees counterclockwise from east. Loosely bunched near the middle, as a herd.
pub const COWS: &[(i64, i64, f32)] = &[
    (-50, -49, 0.0),
    (-47, -51, 90.0),
    (-44, -48, 200.0),
    (-49, -45, 300.0),
    (-46, -44, 45.0),
    (-43, -45, 160.0),
    (-48, -41, 270.0),
    (-44, -41, 10.0),
];
/// The pasture's wall, [`PASTURE_WALL`] blocks high, around the square between these corners
/// (wall included), relative to the test area's origin. A fight can't throw the player over it.
pub const PASTURE: ((i64, i64), (i64, i64)) = ((-62, -62), (-32, -32));
/// How high the pasture's wall is, in blocks.
pub const PASTURE_WALL: i64 = 8;
/// The pasture's doorway, in its east wall: 3 wide, from y - 1 to y + 1, with a sill a block
/// high that the player jumps and cows can't step.
pub const PASTURE_DOOR_Y: i64 = -47;
/// Just outside the pasture's east doorway, looking in.
pub const PASTURE_SPOT: (i64, i64, i64) = (-30, -47, 0);
/// Just outside the empty cow pen's doorway, looking in.
pub const COW_PEN_SPOT: (i64, i64, i64) = (-6, 14, 0);

/// Where the shapes corner of the test area starts, relative to its origin: where the feet stand
/// at the foot of the staircase.
pub const SHAPES_SPOT: (i64, i64, i64) = (-5, 0, 0);

/// Where the crafting table stands, relative to the test area's origin: north-west of spawn,
/// its front facing south toward it.
pub const TABLE_SPOT: (i64, i64, i64) = (-2, 3, 0);

/// Where the door stands, relative to the test area's origin: in a brick wall north of spawn,
/// its knob and the lever beside it facing south toward spawn.
pub const DOOR_SPOT: (i64, i64, i64) = (2, 10, 0);

/// Where the far copy of the test area sits, for checking that play is the same in the millions.
pub const FAR_ORIGIN: BoxelPos = BoxelPos::new(3_000_000, -2_000_000, 1_000);

/// The generated terrain's center, far north of the test area so the two never meet. The
/// terrain's height map is measured from here.
pub const TERRAIN_CENTER: BoxelPos = BoxelPos::new(0, 4096, 0);

/// Cow herds on the terrain: where each herd gathers, relative to the terrain's center, and
/// how many cows it has.
pub const TERRAIN_HERDS: &[(i64, i64, usize)] = &[(24, 18, 5), (-140, 95, 4), (160, -130, 6)];

/// Half the width of the square test area floor.
pub const AREA_HALF: i64 = 64;

/// Notable spots in the test area, relative to its origin: where the feet stand. Used as
/// waypoints named `landmark:<name>`, which the chase drill picks objectives from.
pub const LANDMARKS: &[(&str, i64, i64, i64)] = &[
    ("marker post top", 3, 0, 2),
    ("top of the steps", 15, 5, 10),
    ("inside the pen", -14, 14, 0),
    ("tunnel", 0, -14, 0),
    ("bottom of the pit", 17, -12, -10),
    ("end of the jump course", 56, 15, 3),
    ("end of the pillar line", 51, 27, 2),
    ("end of the sprint straight", 60, 41, 0),
    ("ice patch", -45, 53, 0),
    ("2-high ledge", 27, -28, 2),
];

pub struct BaseMod;

impl Mod for BaseMod {
    fn name(&self) -> &str {
        "base"
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        // Blocks and their looks are data, in `mods/base`; see `DATA_DIR`.
        r.register_generator(terrain::HILLS, Box::new(terrain::Hills::default()))?;
        r.register_player(PlayerDef {
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            // Enough to walk up slabs and stairs, not full blocks.
            step_height: 0.6,
            controller: "base:quake".into(),
            // A robot: metal and motors. The sounds are data, in `mods/base/sounds.toml`.
            sounds: PlayerSounds {
                step: Some("base:step_concrete".into()),
                // A landing is the surface's own footstep, louder; only a hard one adds a sound.
                land: None,
                hard_land: Some("base:hardland_robot".into()),
                hurt: Some("base:hurt_robot".into()),
                knocked_out: Some("base:knockout_robot".into()),
                guard: Some("base:guard".into()),
                servo: Some("base:servo".into()),
                wind: Some("base:wind".into()),
                pickup: Some("base:pickup_robot".into()),
                craft: Some("base:craft".into()),
                craft_refused: Some("base:craft_refused".into()),
            },
        })?;
        // Minecraft-like for now: about Minecraft's survival reach, 4 edits a second held.
        r.register_interaction(InteractionDef { reach: 4.5, repeat_rate: 4.0 })?;
        // Block-jumping: placing a block at your feet lifts you onto it and launches you.
        r.register_push_out(PushOutDef { strength: 6.0, upward_bias: 1.0 })?;
        // Hiding takes what touches a phase group, a sixteenth deep; a wire carries far.
        r.register_signals(SignalDef { reach: 1.0, conduct_limit: 4096 })?;
        // Creatures see each other across a herd, and bodies push apart firmly enough that you
        // can't walk through a cow; the player is a little lighter than one (2.5).
        r.register_crowd(CrowdDef { sight: 12.0, push: 25.0, max_push: 8.0, player_weight: 1.5 })?;
        // Two charges or ten tosses from whole; it comes back slowly once you've got clear. A
        // guard catches what's within 60 degrees of the look. Falls hurt from about 4 blocks
        // (16.5 m/s, with the controller's fall gravity of 35), so a 10-block drop costs about
        // half your health. Killed cows stay dead until you're knocked out, so taking down a
        // herd has to happen in one life; then the whole herd is back.
        r.register_health(HealthDef {
            health: 20.0,
            regen: 1.0,
            regen_delay: 4.0,
            guard_angle: 1.05,
            fall_speed: 16.5,
            fall_damage: 1.0,
            knock_out_resets: true,
        })?;
        // Minecraft-like drops: a small hop out of the broken block, pulled in from a couple of
        // meters, picked up on touch.
        r.register_item_drops(ItemDropDef {
            pop_speed: 3.0,
            gravity: 20.0,
            bounce: 0.3,
            drag: 8.0,
            size: 0.25,
            pull_radius: 2.0,
            pull_speed: 8.0,
            pickup_radius: 0.3,
            merge_radius: 1.0,
        })?;
        // A blunt club: a quick wind-up, a heavy knock along the ground more than up, and a short
        // recovery. Running into a swing passes most of your speed on.
        r.register_tool(ToolDef {
            name: CLUB.into(),
            reach: 2.8,
            radius: 0.35,
            damage: 5.0,
            knockback: 22.0,
            lift: 4.0,
            momentum: 0.8,
            momentum_damage: 0.5,
            // Swing down onto a cow to break a fall: a 20-damage head hit carried in at 20 m/s
            // takes 16 m/s off, enough to land a 10-block drop safely.
            recoil: 0.8,
            recoil_bounce: 3.0,
            windup: 0.07,
            recover: 0.28,
            sounds: ToolSounds { swing: Some("base:swing_club".into()), hit: Some("base:hit_club".into()) },
        })
    }

    fn build_world(&self, w: &mut dyn WorldEdit) -> Result<(), ApiError> {
        let origin = BoxelPos::new(0, 0, 0);
        build_test_area(w, origin)?;
        build_test_area(w, FAR_ORIGIN)?;

        let spawn = |o: BoxelPos| WorldPos::new(o.0, Vec3::new(0.5, 0.5, 0.0));
        w.set_spawn(spawn(origin));
        w.add_waypoint("origin", spawn(origin));
        w.add_waypoint("far", spawn(FAR_ORIGIN));
        let (x, y, z) = SHAPES_SPOT;
        w.add_waypoint("shapes", WorldPos::new(origin.0 + I64Vec3::new(x, y, z), Vec3::new(0.5, 0.5, 0.0)));
        let (x, y, z) = DOOR_SPOT;
        w.add_waypoint("door", WorldPos::new(origin.0 + I64Vec3::new(x, y - 2, z), Vec3::new(0.5, 0.5, 0.0)));
        let (x, y, z) = COW_PEN_SPOT;
        w.add_waypoint("cow pen", WorldPos::new(origin.0 + I64Vec3::new(x, y, z), Vec3::new(0.5, 0.5, 0.0)));
        let (x, y, z) = PASTURE_SPOT;
        w.add_waypoint("pasture", WorldPos::new(origin.0 + I64Vec3::new(x, y, z), Vec3::new(0.5, 0.5, 0.0)));
        for &(x, y, facing) in COWS {
            w.add_target(COW, WorldPos::new(origin.0 + I64Vec3::new(x, y, 0), Vec3::new(0.5, 0.5, 0.0)), facing.to_radians())?;
        }
        build_terrain(w)?;
        for o in [origin, FAR_ORIGIN] {
            for (name, x, y, z) in LANDMARKS {
                w.add_waypoint(&format!("landmark:{name}"), WorldPos::new(o.0 + I64Vec3::new(*x, *y, *z), Vec3::new(0.5, 0.5, 0.0)));
            }
        }
        Ok(())
    }
}

/// Places the generated terrain, a waypoint at its center and the herds on it. Feet go where
/// the default tuning puts the ground; the engine lifts anything that ends up buried.
fn build_terrain(w: &mut dyn WorldEdit) -> Result<(), ApiError> {
    w.add_terrain(TerrainDef { generator: terrain::HILLS.into(), center: TERRAIN_CENTER })?;
    let hills = terrain::Hills::default();
    let feet = |x: i64, y: i64| WorldPos::new(TERRAIN_CENTER.0 + I64Vec3::new(x, y, hills.surface(x, y)), Vec3::new(0.5, 0.5, 0.0));
    w.add_waypoint("terrain", feet(0, 0));
    for (h, &(x, y, count)) in TERRAIN_HERDS.iter().enumerate() {
        for i in 0..count as i64 {
            // Loosely bunched: a little ring around the herd's spot.
            let (dx, dy) = [(0, 0), (3, 1), (-2, 3), (1, -3), (-3, -1), (4, -2)][i as usize % 6];
            let yaw = ((h as i64 * 97 + i * 61) % 360) as f32;
            w.add_target(COW, feet(x + dx, y + dy), yaw.to_radians())?;
        }
    }
    Ok(())
}

/// A flat floor with steps, walls, a low tunnel, a pit and a tall tower. The floor's top
/// surface is at `o.z`; the player stands on it at `o`.
pub fn build_test_area(w: &mut dyn WorldEdit, o: BoxelPos) -> Result<(), ApiError> {
    let at = |x: i64, y: i64, z: i64| o.offset(I64Vec3::new(x, y, z));
    let h = AREA_HALF;

    // Ground: stone, then dirt, then grass on top. Stone stripes on chunk borders.
    w.fill(at(-h, -h, -12), at(h - 1, h - 1, -4), STONE)?;
    w.fill(at(-h, -h, -3), at(h - 1, h - 1, -2), DIRT)?;
    w.fill(at(-h, -h, -1), at(h - 1, h - 1, -1), GRASS)?;
    for i in (-h..h).filter(|i| (o.0.x + i).rem_euclid(32) == 0) {
        w.fill(at(i, -h, -1), at(i, h - 1, -1), STONE)?;
    }
    for i in (-h..h).filter(|i| (o.0.y + i).rem_euclid(32) == 0) {
        w.fill(at(-h, i, -1), at(h - 1, i, -1), STONE)?;
    }

    // Steps rising east, one boxel per step, 3 wide.
    for step in 0..10 {
        w.fill(at(6 + step, 4, 0), at(6 + step, 6, step), BRICK)?;
    }

    // A walled pen with a doorway, 3 high.
    w.fill(at(-20, 8, 0), at(-8, 8, 2), BRICK)?;
    w.fill(at(-20, 20, 0), at(-8, 20, 2), BRICK)?;
    w.fill(at(-20, 8, 0), at(-20, 20, 2), BRICK)?;
    w.fill(at(-8, 8, 0), at(-8, 20, 2), BRICK)?;
    // The doorway has a sill a block high: the player jumps it, cows can't step it.
    w.clear(at(-8, 13, 0), at(-8, 15, 2));
    w.fill(at(-8, 13, 0), at(-8, 15, 0), BRICK)?;
    // In the pen's north-west corner, a platform 2 high, reached from the east by slab steps
    // half a block each: cows walk up and down the steps but not off the sides.
    w.fill(at(-19, 17, 0), at(-18, 19, 1), BRICK)?;
    w.fill(at(-17, 17, 0), at(-16, 19, 0), BRICK)?;
    for y in 17..=19 {
        w.add_shaped(at(-17, y, 1), BRICK, &shapes::slab(), Orientation::IDENTITY)?;
        w.add_shaped(at(-15, y, 0), BRICK, &shapes::slab(), Orientation::IDENTITY)?;
    }

    // The pasture: a square of tall walls, with a doorway east whose sill keeps the cows in.
    let ((x0, y0), (x1, y1)) = PASTURE;
    let top = PASTURE_WALL - 1;
    w.fill(at(x0, y0, 0), at(x1, y0, top), WOOD)?;
    w.fill(at(x0, y1, 0), at(x1, y1, top), WOOD)?;
    w.fill(at(x0, y0, 0), at(x0, y1, top), WOOD)?;
    w.fill(at(x1, y0, 0), at(x1, y1, top), WOOD)?;
    w.clear(at(x1, PASTURE_DOOR_Y - 1, 1), at(x1, PASTURE_DOOR_Y + 1, top));
    build_cow_course(w, o)?;

    // A tunnel with exactly 2 boxels of headroom.
    w.fill(at(-3, -20, 0), at(-3, -8, 2), STONE)?;
    w.fill(at(3, -20, 0), at(3, -8, 2), STONE)?;
    w.fill(at(-3, -20, 2), at(3, -8, 2), STONE)?;

    // A pit, 6×6 and 10 deep, cutting through the ground layers.
    w.clear(at(15, -15, -10), at(20, -10, -1));

    // A tower tall enough to cross a vertical chunk border, with the top marked.
    w.fill(at(-30, -30, 0), at(-28, -28, 39), STONE)?;
    w.fill(at(-30, -30, 40), at(-28, -28, 40), MARKER)?;

    // A marker post next to spawn so the starting direction is obvious: it sits east.
    w.fill(at(3, 0, 0), at(3, 0, 1), MARKER)?;

    build_shapes_corner(w, o)?;
    build_movement_course(w, o)
}

/// Things for cows to find their way round, inside the pasture away from the herd: a pen in the
/// north-west corner whose only way in is a gap one block wide, with a terrace inside one block
/// up and a second tier above it; a pillar 3 high no cow can get up; and a mesa 5 high in the
/// south-west corner, climbed by block-high steps on its east side, whose cliffs a charge can
/// be baited off.
fn build_cow_course(w: &mut dyn WorldEdit, o: BoxelPos) -> Result<(), ApiError> {
    let at = |x: i64, y: i64, z: i64| o.offset(I64Vec3::new(x, y, z));
    // The pen: walls 3 high, the gap at x = -56.
    w.fill(at(-61, -38, 0), at(-51, -38, 2), BRICK)?;
    w.fill(at(-51, -38, 0), at(-51, -33, 2), BRICK)?;
    w.clear(at(-56, -38, 0), at(-56, -38, 2));
    // Its terrace, and the tier above.
    w.fill(at(-61, -37, 0), at(-57, -33, 0), STONE)?;
    w.fill(at(-61, -35, 1), at(-59, -33, 1), STONE)?;
    // The pillar.
    w.fill(at(-40, -36, 0), at(-40, -36, 2), STONE)?;
    // The mesa and its steps, against the south wall.
    w.fill(at(-61, -61, 0), at(-56, -56, 4), STONE)?;
    for (i, x) in (-55..=-52).enumerate() {
        w.fill(at(x, -61, 0), at(x, -59, 3 - i as i64), STONE)?;
    }
    Ok(())
}

/// Blocks that aren't full cubes, west of spawn: a staircase rising west, a slab path with
/// fence posts sharing its boxels, a lone post, a double slab, logs and the crafting table.
/// Every shape is crafted, the same way a player would make it (see [`shapes`]).
fn build_shapes_corner(w: &mut dyn WorldEdit, o: BoxelPos) -> Result<(), ApiError> {
    let at = |x: i64, y: i64, z: i64| o.offset(I64Vec3::new(x, y, z));
    let west = Orientation::quarter_turns(2, 1);
    let top = Orientation::quarter_turns(0, 2);
    let (stairs, slab, post) = (shapes::stairs(), shapes::slab(), shapes::post());

    // Stairs up to a platform 3 high, 3 wide. Unturned stairs rise to the north; a quarter
    // turn makes them rise to the west.
    for y in -1..=1 {
        for step in 0..3 {
            let x = -7 - step;
            if step > 0 {
                w.fill(at(x, y, 0), at(x, y, step - 1), STONE)?;
            }
            w.add_shaped(at(x, y, step), STONE, &stairs, west)?;
        }
        w.fill(at(-12, y, 0), at(-10, y, 2), STONE)?;
    }

    // A path of bottom slabs, with fence posts poking through two of them.
    for x in -12..=-7 {
        w.add_shaped(at(x, -4, 0), BRICK, &slab, Orientation::IDENTITY)?;
    }
    for x in [-12, -9] {
        w.add_shaped(at(x, -4, 0), WOOD, &post, Orientation::IDENTITY)?;
    }
    // And a post standing on its own.
    w.add_shaped(at(-9, -7, 0), WOOD, &post, Orientation::IDENTITY)?;
    // A double slab: a bottom and a top slab in one boxel.
    w.add_shaped(at(-6, -4, 0), STONE, &slab, Orientation::IDENTITY)?;
    w.add_shaped(at(-6, -4, 0), STONE, &slab, top)?;

    // A log lying east to west, and one standing.
    for x in -12..=-10 {
        w.add_block(at(x, 3, 0), LOG, Orientation::z_along(0))?;
    }
    w.fill(at(-7, 3, 0), at(-7, 3, 1), LOG)?;

    build_door(w, o)?;

    // The crafting table, unturned: its front faces south.
    let (x, y, z) = TABLE_SPOT;
    w.add_block(at(x, y, z), CRAFTING_TABLE, Orientation::IDENTITY)
}

/// A brick wall 3 high with a doorway, a two-high door in it like a Minecraft door, a lever
/// in the wall beside it, and another lever further along wired to the door through copper
/// along the foot of the wall, all facing south. Nothing here is a door to the engine: the
/// panels swap because they're phasite and antiphasite, the levers work because their handles
/// are brass, and the signal reaches the door through copper (see `blocks.toml`).
fn build_door(w: &mut dyn WorldEdit, o: BoxelPos) -> Result<(), ApiError> {
    let (x, y, z) = DOOR_SPOT;
    let at = |dx: i64, dy: i64, dz: i64| o.offset(I64Vec3::new(x + dx, y + dy, z + dz));
    let south = Orientation::IDENTITY;
    w.fill(at(-3, 0, 0), at(3, 0, 2), BRICK)?;
    w.clear(at(0, 0, 0), at(0, 0, 1));
    w.clear(at(1, 0, 0), at(1, 0, 0));
    w.clear(at(3, 0, 0), at(3, 0, 0));
    for dx in [1, 3] {
        for (m, shape) in shapes::lever(BRICK) {
            w.add_shaped(at(dx, 0, 0), m, &shape, south)?;
        }
    }
    // The wire: from in front of the far lever to in front of the door, whose copper it meets.
    for dx in 0..=3 {
        w.add_shaped(at(dx, -1, 0), COPPER, &shapes::wire(), south)?;
    }
    // The door last, so its open panel, starting hidden, takes its reach of all the rest.
    w.add_bundle(at(0, 0, 0), &shapes::door_lower(), south)?;
    w.add_bundle(at(0, 0, 1), &shapes::door_upper(), south)?;
    Ok(())
}

/// Things to move around on: jump gaps, a pillar line, a sprint straight, an ice patch and
/// a crouch-jump ledge.
/// The tower in the test area doubles as a drop tower: fly up, turn fly off, fall.
fn build_movement_course(w: &mut dyn WorldEdit, o: BoxelPos) -> Result<(), ApiError> {
    let at = |x: i64, y: i64, z: i64| o.offset(I64Vec3::new(x, y, z));

    // Jump gaps of 2, 3, 4 and 5 between platforms 3 high, reached by three steps.
    let (y0, y1) = (14, 17);
    for step in 0..3 {
        w.fill(at(22 + step, y0, 0), at(22 + step, y1, step), BRICK)?;
    }
    let mut x = 25;
    for gap in [2, 3, 4, 5, 0] {
        w.fill(at(x, y0, 0), at(x + 3, y1, 2), BRICK)?;
        w.fill(at(x, y0, 2), at(x + 3, y1, 2), MARKER)?;
        x += 4 + gap;
    }

    // A pillar line: single pillars 2 high, 3 apart, zigzagging.
    for i in 0..10 {
        let px = 24 + i * 3;
        let py = if i % 2 == 0 { 25 } else { 27 };
        w.fill(at(px, py, 0), at(px, py, 1), STONE)?;
    }

    // A sprint straight: a stone lane with a marker post every 10 boxels.
    w.fill(at(-60, 40, -1), at(60, 42, -1), STONE)?;
    for x in (-60..=60).step_by(10) {
        w.fill(at(x, 43, 0), at(x, 43, 1), MARKER)?;
    }

    // An ice patch set into the floor.
    w.fill(at(-60, 46, -1), at(-30, 60, -1), ICE)?;

    // A crouch-jump ledge facing west, 2 high: a tuck clears it, a plain jump doesn't.
    w.fill(at(26, -30, 0), at(29, -27, 1), BRICK)?;
    w.fill(at(26, -30, 1), at(29, -27, 1), MARKER)?;
    Ok(())
}
