//! The base game, written as a mod on the public API only.

pub mod movement;

use api::{
    ApiError, BlockDef, BoxelPos, I64Vec3, InteractionDef, Mod, PlayerDef, Registrar, Vec3, WorldEdit, WorldPos,
};

use crate::movement::QuakeController;

pub const STONE: &str = "base:stone";
pub const DIRT: &str = "base:dirt";
pub const GRASS: &str = "base:grass";
pub const BRICK: &str = "base:brick";
pub const MARKER: &str = "base:marker";
pub const ICE: &str = "base:ice";

/// Where the far copy of the test area sits, for checking that play is the same in the millions.
pub const FAR_ORIGIN: BoxelPos = BoxelPos::new(3_000_000, -2_000_000, 1_000);

/// Half the width of the square test area floor.
pub const AREA_HALF: i64 = 64;

pub struct BaseMod;

impl Mod for BaseMod {
    fn name(&self) -> &str {
        "base"
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        r.register_block(BlockDef::new(STONE, true, [125, 125, 125]))?;
        r.register_block(BlockDef::new(DIRT, true, [134, 96, 67]))?;
        r.register_block(BlockDef::new(GRASS, true, [96, 160, 64]))?;
        r.register_block(BlockDef::new(BRICK, true, [160, 82, 64]))?;
        r.register_block(BlockDef::new(MARKER, true, [240, 180, 40]))?;
        r.register_block(BlockDef::new(ICE, true, [170, 210, 240]).with_friction(0.1))?;
        r.register_player(PlayerDef {
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            step_height: 0.0,
            controller: Box::new(QuakeController::default()),
        })?;
        // Minecraft-like for now: about Minecraft's survival reach, 4 edits a second held.
        r.register_interaction(InteractionDef { reach: 4.5, repeat_rate: 4.0 })
    }

    fn build_world(&self, w: &mut dyn WorldEdit) -> Result<(), ApiError> {
        let origin = BoxelPos::new(0, 0, 0);
        build_test_area(w, origin)?;
        build_test_area(w, FAR_ORIGIN)?;

        let spawn = |o: BoxelPos| WorldPos::new(o.0, Vec3::new(0.5, 0.5, 0.0));
        w.set_spawn(spawn(origin));
        w.add_waypoint("origin", spawn(origin));
        w.add_waypoint("far", spawn(FAR_ORIGIN));
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

    build_movement_course(w, o)
}

/// Things to move around on: jump gaps, a pillar line, a sprint straight and an ice patch.
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
    Ok(())
}
