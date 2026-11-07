//! The base game's door and levers: nothing in the engine knows about doors. The door's two
//! panels swap because one is phasite and the other antiphasite (phase materials, the second
//! starting hidden), the levers work because their handles are brass (a trigger), and the
//! signal reaches the door through copper (a conductor).

use api::{BoxelPos, I64Vec3, Mod, Vec3, WorldPos};
use sim::{Occupant, PlayerInput, Simulation};

fn base_sim() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mods: [&dyn Mod; 4] = [&data, &base::BaseMod, &base_cow::BaseCowMod, &base_movement::BaseMovementMod];
    let mut sim = Simulation::with_mods(&mods).expect("base mod loads");
    sim.spawn_player().expect("base registers a player");
    sim
}

/// The door's lower and upper boxels, the lever beside it and the wired lever.
fn spots() -> (BoxelPos, BoxelPos, BoxelPos, BoxelPos) {
    let (x, y, z) = base::DOOR_SPOT;
    (BoxelPos::new(x, y, z), BoxelPos::new(x, y, z + 1), BoxelPos::new(x + 1, y, z), BoxelPos::new(x + 3, y, z))
}

/// The handle on a lever's south face.
const HANDLE: [f32; 3] = [0.5, 0.0, 0.5];

/// Stands the feet at `offset` in boxel `(x, y, 0)` and lets the body settle.
fn stand(sim: &mut Simulation, x: i64, y: i64, offset: [f32; 2]) {
    sim.teleport_player(WorldPos::new(I64Vec3::new(x, y, 0), Vec3::new(offset[0], offset[1], 0.0)));
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

/// Whether the boxel's piece of `material` is shown.
fn shown(sim: &Simulation, pos: BoxelPos, material: &str) -> bool {
    let blocks = sim.blocks();
    let m = blocks.material(material).unwrap();
    let piece = cell(sim, pos).into_iter().find(|o| blocks.made_of(o.block) == Some(m)).expect("a piece of it");
    !blocks.is_hidden(piece.block)
}

/// Whether the door stands open: the closed panel hidden and the open one shown, both halves.
fn is_open(sim: &Simulation) -> bool {
    let (lower, upper, ..) = spots();
    let open = |p| !shown(sim, p, base::PHASITE) && shown(sim, p, base::ANTIPHASITE);
    let shut = |p| shown(sim, p, base::PHASITE) && !shown(sim, p, base::ANTIPHASITE);
    match (open(lower), open(upper), shut(lower), shut(upper)) {
        (true, true, ..) => true,
        (_, _, true, true) => false,
        _ => panic!("half open"),
    }
}

/// Every boxel around the door, the wall and the wire.
fn around(sim: &Simulation) -> Vec<Vec<Occupant>> {
    let (lower, ..) = spots();
    let mut out = Vec::new();
    for dz in -1..=2 {
        for dy in -1..=1 {
            for dx in -2..=4 {
                out.push(cell(sim, lower.offset(I64Vec3::new(dx, dy, dz))));
            }
        }
    }
    out
}

#[test]
fn the_door_starts_shut_with_its_open_panel_hidden() {
    let sim = base_sim();
    assert!(!is_open(&sim));
    let (lower, ..) = spots();
    assert_eq!(cell(&sim, lower).len(), 3, "phasite, antiphasite and copper");
}

#[test]
fn the_lever_beside_it_swaps_the_panels_and_back_exactly() {
    let mut sim = base_sim();
    let (_, _, lever, _) = spots();
    let before = around(&sim);
    stand(&mut sim, lever.0.x, lever.0.y - 2, [0.5, 0.5]);
    interact(&mut sim, lever, HANDLE);
    assert!(is_open(&sim));
    assert_ne!(around(&sim), before, "the frame lost what touched the closed panel");
    interact(&mut sim, lever, HANDLE);
    assert!(!is_open(&sim));
    assert_eq!(around(&sim), before, "everything back exactly as it was");
}

#[test]
fn the_wired_lever_reaches_the_door_through_copper() {
    let mut sim = base_sim();
    let (.., far) = spots();
    stand(&mut sim, far.0.x, far.0.y - 2, [0.5, 0.5]);
    interact(&mut sim, far, HANDLE);
    assert!(is_open(&sim));
    interact(&mut sim, far, HANDLE);
    assert!(!is_open(&sim));
}

#[test]
fn the_open_panel_pushes_the_player_out_of_its_place() {
    let mut sim = base_sim();
    let (lower, _, lever, _) = spots();
    // In the doorway's south-west, where the open panel goes.
    stand(&mut sim, lower.0.x, lower.0.y, [0.2, 0.3]);
    assert_eq!(sim.player().unwrap().pos().cell(), lower.0, "standing in the doorway");
    interact(&mut sim, lever, HANDLE);
    assert!(is_open(&sim), "the whole door swaps, never half");
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
    let (body, blocks, grid) = (sim.player().unwrap(), sim.blocks(), sim.universe().main());
    let boxes: Vec<_> = grid.cell(lower).iter().filter(|o| blocks.is_solid(o.block)).flat_map(|&o| sim::occupant_boxes(o, blocks)).collect();
    assert!(!body.overlaps_boxes(lower, &boxes), "pushed out of the panel");
    // And it closes again.
    stand(&mut sim, lever.0.x, lever.0.y - 2, [0.5, 0.5]);
    interact(&mut sim, lever, HANDLE);
    assert!(!is_open(&sim));
}

#[test]
fn hidden_pieces_are_aimed_at_only_holding_a_phase_material() {
    let mut sim = base_sim();
    let (lower, ..) = spots();
    stand(&mut sim, lower.0.x, lower.0.y - 2, [0.5, 0.5]);
    // The hidden open panel, along the doorway's west side.
    let (yaw, pitch) = aim(&sim, lower, [0.125, 0.25, 0.5]);
    let blocks = sim.blocks();
    let hit = |hidden: bool| sim.target_seeing(yaw, pitch, hidden).map(|h| (h.pos, cell(&sim, h.pos)[h.occupant]));
    let seen = hit(true).expect("the ghost");
    assert_eq!(seen.0, lower);
    assert!(blocks.is_hidden(seen.1.block));
    let through = hit(false).expect("the closed panel behind it");
    assert!(!blocks.is_hidden(through.1.block), "looking through it");
}

#[test]
fn breaking_any_piece_of_the_door_takes_its_half_whole() {
    let mut sim = base_sim();
    let (lower, upper, ..) = spots();
    let antiphasite = sim.blocks().id(base::ANTIPHASITE).unwrap();
    stand(&mut sim, lower.0.x, lower.0.y - 2, [0.5, 0.5]);
    let before_upper = cell(&sim, upper);
    // Holding antiphasite, the hidden open panel can be aimed at and broken.
    let (yaw, pitch) = aim(&sim, lower, [0.125, 0.25, 0.5]);
    let input = PlayerInput { yaw, pitch, place_block: antiphasite, use_pressed: true, use_held: true, ..Default::default() };
    sim.step(&input);
    let blocks = sim.blocks();
    assert!(cell(&sim, lower).is_empty(), "every piece of the lower half");
    assert_eq!(cell(&sim, upper), before_upper, "the upper half is a bundle of its own");
    let items: Vec<_> = sim.items().iter().map(|i| i.drop).collect();
    assert_eq!(items.len(), 1, "one item: {items:?}");
    assert_eq!(blocks.pieces(items[0].material).len(), 3, "the lower half's bundle, whole");
}

#[test]
fn antiphasite_is_placed_hidden() {
    let mut sim = base_sim();
    let (lower, ..) = spots();
    let antiphasite = sim.blocks().id(base::ANTIPHASITE).unwrap();
    // On the floor two boxels in front of the door.
    stand(&mut sim, lower.0.x, lower.0.y - 4, [0.5, 0.5]);
    let spot = BoxelPos::new(lower.0.x, lower.0.y - 2, 0);
    let (yaw, pitch) = aim(&sim, spot.offset(-I64Vec3::Z), [0.5, 0.5, 1.0]);
    sim.step(&PlayerInput { yaw, pitch, alt_pressed: true, alt_held: true, place_block: antiphasite, ..Default::default() });
    let placed = cell(&sim, spot);
    assert_eq!(placed.len(), 1);
    assert!(sim.blocks().is_hidden(placed[0].block), "there, but hidden");
    // The floor under it lost the sixteenth touching it.
    assert_eq!(cell(&sim, spot.offset(-I64Vec3::Z)).len(), 2);
}
