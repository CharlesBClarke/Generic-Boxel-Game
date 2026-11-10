//! The base game's materials lab: a lumite lamp behind a phasite shutter, and a crystal gun.
//! Nothing in the engine knows about lamps or guns: the lamp glows because its look does, the
//! gun fires because a blue crystal (a fuse) comes to touch a green one (a lens).

use api::{BoxelPos, Mod, Vec3, WorldPos};
use sim::{Occupant, PlayerInput, Simulation};

fn base_sim() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mods: [&dyn Mod; 4] = [&data, &base::BaseMod, &base_cow::BaseCowMod, &base_movement::BaseMovementMod];
    let mut sim = Simulation::with_mods(&mods).expect("base mod loads");
    sim.spawn_player().expect("base registers a player");
    sim
}

/// A boxel of the lab, from its spot.
fn lab(dx: i64, dy: i64, dz: i64) -> BoxelPos {
    let (x, y, z) = base::LAB_SPOT;
    BoxelPos::new(x + dx, y + dy, z + dz)
}

/// Stands the feet in the lab's boxel `(dx, dy, 0)` and lets the body settle.
fn stand(sim: &mut Simulation, dx: i64, dy: i64) {
    sim.teleport_player(WorldPos::new(lab(dx, dy, 0).0, Vec3::new(0.5, 0.5, 0.0)));
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
}

/// The look toward `point` in boxel `cell`.
fn aim(sim: &Simulation, cell: BoxelPos, point: [f32; 3]) -> (f32, f32) {
    let to = sim.player().unwrap().eye().delta_to(WorldPos::new(cell.0, Vec3::from_array(point)));
    (to.y.atan2(to.x), to.z.atan2(to.truncate().length()))
}

/// Presses interact looking at `point` in boxel `cell`.
fn interact(sim: &mut Simulation, cell: BoxelPos, point: [f32; 3]) {
    let (yaw, pitch) = aim(sim, cell, point);
    let hit = sim.target(yaw, pitch).expect("in reach");
    assert_eq!(hit.pos, cell, "aimed at the right boxel");
    sim.step(&PlayerInput { yaw, pitch, interact_pressed: true, ..Default::default() });
}

fn cell(sim: &Simulation, pos: BoxelPos) -> Vec<Occupant> {
    sim.universe().main().cell(pos).to_vec()
}

/// Whether the boxel holds a shown piece of `material`.
fn has(sim: &Simulation, pos: BoxelPos, material: &str) -> bool {
    let blocks = sim.blocks();
    let m = blocks.material(material).unwrap();
    cell(sim, pos).iter().any(|o| blocks.made_of(o.block) == Some(m) && !blocks.is_hidden(o.block))
}

#[test]
fn the_lab_gun_fires_through_stone_and_chains_through_the_second_lens() {
    let mut sim = base_sim();
    stand(&mut sim, 4, -2);
    assert!(!has(&sim, lab(4, 1, 0), base::BLUE_CRYSTAL), "the blue crystal starts hidden");
    assert!(has(&sim, lab(5, 1, 0), base::GREEN_CRYSTAL));
    sim.take_blasts();
    interact(&mut sim, lab(4, 0, 0), [0.5, 0.0, 0.5]);
    let blasts = sim.take_blasts();
    assert_eq!(blasts.len(), 2, "the gun's lens, then the second one: {blasts:?}");
    assert!(blasts.iter().all(|b| b.dir == Vec3::X), "both fire east");
    for (pos, what) in [(lab(4, 1, 0), base::BLUE_CRYSTAL), (lab(5, 1, 0), base::GREEN_CRYSTAL), (lab(11, 1, 0), base::GREEN_CRYSTAL)] {
        assert!(!has(&sim, pos, what), "{what} at {pos:?} used up");
    }
    assert!(cell(&sim, lab(8, 1, 0)).is_empty() && cell(&sim, lab(9, 1, 0)).is_empty(), "the stone broke");
    assert!(cell(&sim, lab(15, 1, 0)).is_empty(), "the brick wall has a hole");
    assert!(!cell(&sim, lab(15, 0, 0)).is_empty() && !cell(&sim, lab(15, 1, 1)).is_empty(), "a hole one boxel wide");
    assert!(has(&sim, lab(4, 1, 0), base::ANTIPHASITE), "the slab that hid the blue is shown");
    // Pulling the lever again flips the slab back, with nothing left to fire.
    interact(&mut sim, lab(4, 0, 0), [0.5, 0.0, 0.5]);
    assert!(sim.take_blasts().is_empty());
}

#[test]
fn the_lab_lamp_shows_when_its_shutter_hides() {
    let mut sim = base_sim();
    stand(&mut sim, 1, 2);
    assert!(has(&sim, lab(0, 4, 1), base::PHASITE), "the shutter starts closed");
    assert!(has(&sim, lab(0, 5, 1), base::LUMITE));
    interact(&mut sim, lab(1, 4, 1), [0.5, 0.0, 0.5]);
    assert!(!has(&sim, lab(0, 4, 1), base::PHASITE), "the shutter hid");
    let lamp = sim.blocks().id(base::LUMITE).unwrap();
    assert_eq!(cell(&sim, lab(0, 5, 1)).first().map(|o| o.block), Some(lamp), "the lamp is untouched, out of the shutter's grab");
    interact(&mut sim, lab(1, 4, 1), [0.5, 0.0, 0.5]);
    assert!(has(&sim, lab(0, 4, 1), base::PHASITE), "and closed again");
}

#[test]
fn placing_a_blue_crystal_against_a_green_one_fires_away_from_it() {
    let mut sim = base_sim();
    stand(&mut sim, 9, -1);
    let blue = sim.blocks().id(base::BLUE_CRYSTAL).unwrap();
    sim.take_blasts();
    // The second lens's west face, from the south-west: the blue goes west of it.
    let (yaw, pitch) = aim(&sim, lab(11, 1, 0), [0.0, 0.5, 0.5]);
    let hit = sim.target(yaw, pitch).expect("in reach");
    assert_eq!(hit.pos, lab(11, 1, 0));
    sim.step(&PlayerInput { yaw, pitch, alt_pressed: true, alt_held: true, place_block: blue, ..Default::default() });
    let blasts = sim.take_blasts();
    assert_eq!(blasts.len(), 1);
    assert_eq!(blasts[0].dir, Vec3::X);
    assert_eq!(blasts[0].from.translate(-Vec3::X * 0.01).boxel(), lab(11, 1, 0), "leaving the lens's boxel");
    assert!(cell(&sim, lab(10, 1, 0)).is_empty() && cell(&sim, lab(11, 1, 0)).is_empty(), "both used up");
    assert!(cell(&sim, lab(15, 1, 0)).is_empty(), "it reached the wall");
    assert_eq!(sim.universe().main().cell(lab(9, 1, 0)).len(), 1, "the stone behind the blue untouched");
}

#[test]
fn a_creature_in_a_beam_is_hurt_and_thrown_along_it() {
    let sim = base_sim();
    let cow = sim.targets().iter().next().expect("a cow").clone();
    let def = sim.target_defs()[cow.kind.index()].clone();
    let from = cow.pos.translate(Vec3::new(-3.0, 0.0, 0.5));
    let mut targets = sim.targets().clone();
    let blast = api::BlastDef { range: 6.0, strength: 1.0, radius: 0.5, damage: 10.0, player_damage: 1.0, push: 20.0 };
    let hit = targets.blast(sim.target_defs(), from, Vec3::X, 6.0, &blast);
    assert_eq!(hit, 1);
    let after = targets.iter().find(|t| t.id == cow.id).unwrap();
    assert_eq!(after.health, cow.health - 10.0);
    assert!((after.velocity.x - cow.velocity.x - 20.0 / def.weight).abs() < 1e-4, "thrown east, softened by weight");
    // A beam passing a meter above its back misses.
    let mut targets = sim.targets().clone();
    let over = cow.pos.translate(Vec3::new(-3.0, 0.0, def.height + 1.0 + 0.5 + def.width));
    assert_eq!(targets.blast(sim.target_defs(), over, Vec3::X, 6.0, &blast), 0);
}

#[test]
fn beam_distance_measures_from_the_middle_line_to_the_body() {
    use sim::combat::beam_distance;
    // A body 2 tall, its feet 3 east and 1 north of the beam's start, the beam 5 east.
    assert!((beam_distance(Vec3::new(3.0, 1.0, 0.0), 2.0, Vec3::X, 5.0) - 1.0).abs() < 1e-4);
    // Beside the body's head height, measured level; above it, from the top of the head.
    assert!((beam_distance(Vec3::new(3.0, 0.0, -1.5), 2.0, Vec3::X, 5.0)).abs() < 1e-4);
    assert!((beam_distance(Vec3::new(3.0, 0.0, -3.0), 2.0, Vec3::X, 5.0) - 1.0).abs() < 1e-4);
    // Past the beam's end.
    assert!((beam_distance(Vec3::new(7.0, 0.0, 0.0), 2.0, Vec3::X, 5.0) - 2.0).abs() < 1e-4);
}

#[test]
fn crystals_built_touching_wait_until_something_near_them_changes() {
    // The lab's gun has its green crystals beside stone and air: nothing fires at load or on
    // a tick with no changes.
    let mut sim = base_sim();
    for _ in 0..5 {
        sim.step(&PlayerInput::default());
    }
    assert!(sim.take_blasts().is_empty());
}
