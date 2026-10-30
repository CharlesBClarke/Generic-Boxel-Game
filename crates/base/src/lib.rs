//! The base game, written as a mod on the public API only. This is its code half; its blocks
//! and textures are a data mod in `mods/base`, loaded by the engine alongside it.

use api::{
    ApiError, BoxelPos, I64Vec3, InteractionDef, ItemDropDef, Mod, Orientation, PlayerDef, PushOutDef, Registrar, Vec3, WorldEdit,
    WorldPos,
};

/// The base game's data half, in the source tree. Both halves share the `base` namespace.
pub const DATA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../mods/base");

// Block names, defined in `mods/base/blocks.toml`.
pub const STONE: &str = "base:stone";
pub const DIRT: &str = "base:dirt";
pub const GRASS: &str = "base:grass";
pub const BRICK: &str = "base:brick";
pub const MARKER: &str = "base:marker";
pub const ICE: &str = "base:ice";
pub const STONE_SLAB: &str = "base:stone_slab";
pub const STONE_STAIRS: &str = "base:stone_stairs";
pub const BRICK_SLAB: &str = "base:brick_slab";
pub const BRICK_STAIRS: &str = "base:brick_stairs";
pub const WOOD_FENCE: &str = "base:wood_fence";
pub const LOG: &str = "base:log";

/// Where the shapes corner of the test area starts, relative to its origin: where the feet stand
/// at the foot of the staircase.
pub const SHAPES_SPOT: (i64, i64, i64) = (-5, 0, 0);

/// Where the far copy of the test area sits, for checking that play is the same in the millions.
pub const FAR_ORIGIN: BoxelPos = BoxelPos::new(3_000_000, -2_000_000, 1_000);

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
    ("3-high ledge", 27, -34, 3),
];

pub struct BaseMod;

impl Mod for BaseMod {
    fn name(&self) -> &str {
        "base"
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        // Blocks and textures are data, in `mods/base`; see `DATA_DIR`.
        r.register_player(PlayerDef {
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            // Enough to walk up slabs and stairs, not full blocks.
            step_height: 0.6,
            controller: "base:quake".into(),
        })?;
        // Minecraft-like for now: about Minecraft's survival reach, 4 edits a second held.
        r.register_interaction(InteractionDef { reach: 4.5, repeat_rate: 4.0 })?;
        // Block-jumping: placing a block at your feet lifts you onto it and launches you.
        r.register_push_out(PushOutDef { strength: 6.0, upward_bias: 1.0 })?;
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
        for o in [origin, FAR_ORIGIN] {
            for (name, x, y, z) in LANDMARKS {
                w.add_waypoint(&format!("landmark:{name}"), WorldPos::new(o.0 + I64Vec3::new(*x, *y, *z), Vec3::new(0.5, 0.5, 0.0)));
            }
        }
        Ok(())
    }
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
    w.clear(at(-8, 13, 0), at(-8, 15, 1));

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

/// Blocks that aren't full cubes, west of spawn: a staircase rising west, a slab path with
/// fence posts sharing its boxels, a lone post, a double slab and logs.
fn build_shapes_corner(w: &mut dyn WorldEdit, o: BoxelPos) -> Result<(), ApiError> {
    let at = |x: i64, y: i64, z: i64| o.offset(I64Vec3::new(x, y, z));
    let west = Orientation::quarter_turns(2, 1);
    let top = Orientation::quarter_turns(0, 2);

    // Stairs up to a platform 3 high, 3 wide. Unturned stairs rise to the north; a quarter
    // turn makes them rise to the west.
    for y in -1..=1 {
        for step in 0..3 {
            let x = -7 - step;
            if step > 0 {
                w.fill(at(x, y, 0), at(x, y, step - 1), STONE)?;
            }
            w.add_block(at(x, y, step), STONE_STAIRS, west)?;
        }
        w.fill(at(-12, y, 0), at(-10, y, 2), STONE)?;
    }

    // A path of bottom slabs, with fence posts poking through two of them.
    for x in -12..=-7 {
        w.add_block(at(x, -4, 0), BRICK_SLAB, Orientation::IDENTITY)?;
    }
    for x in [-12, -9] {
        w.add_block(at(x, -4, 0), WOOD_FENCE, Orientation::IDENTITY)?;
    }
    // And a post standing on its own.
    w.add_block(at(-9, -7, 0), WOOD_FENCE, Orientation::IDENTITY)?;
    // A double slab: a bottom and a top slab in one boxel.
    w.add_block(at(-6, -4, 0), STONE_SLAB, Orientation::IDENTITY)?;
    w.add_block(at(-6, -4, 0), STONE_SLAB, top)?;

    // A log lying east to west, and one standing.
    for x in -12..=-10 {
        w.add_block(at(x, 3, 0), LOG, Orientation::z_along(0))?;
    }
    w.fill(at(-7, 3, 0), at(-7, 3, 1), LOG)
}

/// Things to move around on: jump gaps, a pillar line, a sprint straight, an ice patch and
/// crouch-jump ledges.
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

    // Crouch-jump ledges facing west: 2 high clears with a tuck, 3 high with a charged jump
    // and a tuck. Neither clears with a plain jump.
    w.fill(at(26, -30, 0), at(29, -27, 1), BRICK)?;
    w.fill(at(26, -30, 1), at(29, -27, 1), MARKER)?;
    w.fill(at(26, -36, 0), at(29, -33, 2), BRICK)?;
    w.fill(at(26, -36, 2), at(29, -33, 2), MARKER)?;
    Ok(())
}
