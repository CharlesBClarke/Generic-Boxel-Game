//! The base game's mender in the lab: mendite behind the crystal gun's stone remembers the
//! stone's skin, the cachite floor catches what the blast breaks, and a lever mends.

use api::{BoxelPos, Mod, Vec3, WorldPos};
use sim::{Occupant, PlayerInput, SaveData, Simulation};

fn base_sim() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mods: [&dyn Mod; 4] = [&data, &base::BaseMod, &base_cow::BaseCowMod, &base_movement::BaseMovementMod];
    let mut sim = Simulation::with_mods(&mods).expect("base mod loads");
    sim.spawn_player().expect("base registers a player");
    sim
}

fn lab(dx: i64, dy: i64, dz: i64) -> BoxelPos {
    let (x, y, z) = base::LAB_SPOT;
    BoxelPos::new(x + dx, y + dy, z + dz)
}

fn stand(sim: &mut Simulation, dx: i64, dy: i64) {
    sim.teleport_player(WorldPos::new(lab(dx, dy, 0).0, Vec3::new(0.5, 0.5, 0.0)));
    wait(sim, 30);
}

fn wait(sim: &mut Simulation, ticks: usize) {
    for _ in 0..ticks {
        sim.step(&PlayerInput::default());
    }
}

fn interact(sim: &mut Simulation, cell: BoxelPos, point: [f32; 3]) {
    let to = sim.player().unwrap().eye().delta_to(WorldPos::new(cell.0, Vec3::from_array(point)));
    let (yaw, pitch) = (to.y.atan2(to.x), to.z.atan2(to.truncate().length()));
    let hit = sim.target(yaw, pitch).expect("in reach");
    assert_eq!(hit.pos, cell, "aimed at the right boxel");
    sim.step(&PlayerInput { yaw, pitch, interact_pressed: true, ..Default::default() });
}

fn cell(sim: &Simulation, pos: BoxelPos) -> Vec<Occupant> {
    sim.universe().main().cell(pos).to_vec()
}

/// How much stone the lab's bin holds, every boxel of it together.
fn binned(sim: &Simulation) -> u32 {
    let stone = sim.blocks().material(base::STONE).unwrap();
    let grid = sim.universe().main();
    (7..=10).flat_map(|x| (0..=2).map(move |y| lab(x, y, -1))).filter_map(|p| grid.record(p)).map(|r| r.amount(stone)).sum()
}

/// The stone's skin back against the mendite: the texel layer on its north side.
fn skin_back(sim: &Simulation, pos: BoxelPos) -> bool {
    let blocks = sim.blocks();
    let stone = Occupant::new(blocks.id(base::STONE).unwrap());
    let skin = api::ShapeGrid::from_boxes(&[api::BlockBox::new([0, 15, 0], [16, 16, 16])]);
    cell(sim, pos).iter().any(|&o| blocks.origin(o) == (stone, skin.clone()) && !blocks.is_hidden(o.block))
}

#[test]
fn the_lab_mender_puts_back_the_stone_skin_from_what_the_bin_caught() {
    let mut sim = base_sim();
    let grid = sim.universe().main();
    assert_eq!(grid.record(lab(8, 1, 0)).map(|r| r.remembered()), Some(256), "built remembering the stone's skin");

    // Fire the gun from out of pickup range; the broken stone falls into the bin.
    stand(&mut sim, 4, -2);
    interact(&mut sim, lab(4, 0, 0), [0.5, 0.0, 0.5]);
    assert!(cell(&sim, lab(8, 1, 0)).is_empty() && cell(&sim, lab(9, 1, 0)).is_empty(), "the stone broke");
    wait(&mut sim, 240);
    let caught = binned(&sim);
    assert!(caught >= 512, "the bin caught at least a skin's worth of the stone, got {caught}");

    stand(&mut sim, 7, 0);
    interact(&mut sim, lab(7, 2, 0), [0.5, 0.0, 0.5]);
    assert!(skin_back(&sim, lab(8, 1, 0)) && skin_back(&sim, lab(9, 1, 0)), "both skins back: {:?}", cell(&sim, lab(8, 1, 0)));
    assert_eq!(binned(&sim), caught - 512, "paid a texel of stone a texel");

    // Nothing more gone: mending again changes nothing.
    interact(&mut sim, lab(7, 2, 0), [0.5, 0.0, 0.5]);
    assert_eq!(binned(&sim), caught - 512);

    // Saved and loaded, the memory and the bin are kept.
    let save = SaveData::from_bytes(&sim.snapshot().to_bytes()).unwrap();
    let mut loaded = base_sim();
    let report = loaded.load(&save).unwrap();
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    assert_eq!(binned(&loaded), caught - 512);
    for pos in [lab(8, 1, 0), lab(9, 1, 0), lab(8, 2, 0)] {
        assert_eq!(loaded.universe().main().record(pos), sim.universe().main().record(pos), "the record at {pos:?}");
    }
}

/// All the stone there is outside blocks: carried, lying loose and in the bin.
fn loose_stone(sim: &Simulation) -> u32 {
    let stone = sim.blocks().material(base::STONE).unwrap();
    let lying: u32 = sim.items().iter().filter(|i| i.drop.material == stone).map(|i| i.drop.amount).sum();
    sim.inventory().amount(stone) + lying + binned(sim)
}

#[test]
fn breaking_the_bin_loses_none_of_what_it_held() {
    let mut sim = base_sim();
    stand(&mut sim, 4, -2);
    interact(&mut sim, lab(4, 0, 0), [0.5, 0.0, 0.5]);
    wait(&mut sim, 240);
    let before = loose_stone(&sim);
    let stone = sim.blocks().material(base::STONE).unwrap();
    let full: Vec<BoxelPos> = (7..=10)
        .flat_map(|x| (0..=2).map(move |y| lab(x, y, -1)))
        .filter(|&p| sim.universe().main().record(p).is_some_and(|r| r.amount(stone) > 0))
        .collect();
    assert!(!full.is_empty() && before > 0);
    for pos in full {
        let stand_at = pos.offset(api::I64Vec3::new(0, -1, 1));
        sim.teleport_player(WorldPos::new(stand_at.0, Vec3::new(0.5, 0.5, 0.0)));
        sim.step(&PlayerInput::default());
        let to = sim.player().unwrap().eye().delta_to(WorldPos::new(pos.0, Vec3::new(0.5, 0.5, 1.0)));
        let (yaw, pitch) = (to.y.atan2(to.x), to.z.atan2(to.truncate().length()));
        assert_eq!(sim.target(yaw, pitch).expect("in reach").pos, pos);
        sim.step(&PlayerInput { yaw, pitch, use_pressed: true, use_held: true, ..Default::default() });
        sim.step(&PlayerInput::default());
        assert!(cell(&sim, pos).is_empty(), "broke the bin's block at {pos:?}");
        assert!(sim.universe().main().record(pos).is_none_or(|r| r.stored.is_empty()), "emptied");
        assert_eq!(loose_stone(&sim), before, "none lost breaking {pos:?}");
    }
}

/// Places `block` against the `point` of boxel `cell`'s face, aiming from where the player stands.
fn place(sim: &mut Simulation, cell: BoxelPos, point: [f32; 3], block: sim::BlockId) {
    let to = sim.player().unwrap().eye().delta_to(WorldPos::new(cell.0, Vec3::from_array(point)));
    let (yaw, pitch) = (to.y.atan2(to.x), to.z.atan2(to.truncate().length()));
    assert_eq!(sim.target(yaw, pitch).expect("in reach").pos, cell, "aimed at the right boxel");
    sim.step(&PlayerInput { yaw, pitch, alt_pressed: true, alt_held: true, place_block: block, ..Default::default() });
    sim.step(&PlayerInput::default());
}

#[test]
fn crystals_that_fire_the_moment_they_touch_are_remembered_and_mended() {
    let mut sim = base_sim();
    let blocks = sim.blocks();
    let (mendite, blue) = (blocks.id(base::MENDITE).unwrap(), blocks.id(base::BLUE_CRYSTAL).unwrap());
    // Mendite on the bin's east end, beside where a blue crystal will go against the second lens.
    stand(&mut sim, 9, -1);
    place(&mut sim, lab(10, 2, -1), [0.5, 0.5, 1.0], mendite);
    assert!(!cell(&sim, lab(10, 2, 0)).is_empty(), "the mendite went down");
    sim.take_blasts();
    place(&mut sim, lab(11, 1, 0), [0.0, 0.5, 0.5], blue);
    assert_eq!(sim.take_blasts().len(), 1, "fired as it touched");
    assert!(cell(&sim, lab(10, 1, 0)).is_empty() && cell(&sim, lab(11, 1, 0)).is_empty(), "both used up");
    let grid = sim.universe().main();
    let remembered = |pos| grid.record(pos).map_or(0, |r: &sim::Record| r.remembered());
    assert_eq!(remembered(lab(10, 1, 0)), 16 * 16, "the blue's skin against the mendite");
    assert!(remembered(lab(11, 1, 0)) > 0, "the green's edge, corner to corner");

    // With crystal in the bin, the lever mends them, and they fire again.
    let (b, g) = (sim.blocks().material(base::BLUE_CRYSTAL).unwrap(), sim.blocks().material(base::GREEN_CRYSTAL).unwrap());
    let over = WorldPos::new(lab(10, 0, 0).0, Vec3::new(0.5, 0.5, 0.3));
    stand(&mut sim, 0, -8);
    sim.spawn_item(b, 4096, over);
    sim.spawn_item(g, 4096, over);
    wait(&mut sim, 60);
    stand(&mut sim, 7, 0);
    let plan = sim.view().mend_plan(lab(7, 2, 0)).unwrap();
    assert!(plan.as_ref().is_ok_and(|m| m.texels > 0), "{plan:?}");
    interact(&mut sim, lab(7, 2, 0), [0.5, 0.0, 0.5]);
    assert_eq!(sim.take_blasts().len(), 1, "mended, they touched and fired again");
}
