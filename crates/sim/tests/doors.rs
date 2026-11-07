//! The base game's door and lever: nothing in the engine knows about doors. The door swings
//! because its strip is iron (a hinge), the knob and lever work because they're brass (a
//! trigger), and the lever reaches the door through its copper plate (a listener).

use api::{BoxelPos, I64Vec3, Mod, ShapeGrid, Vec3, WorldPos};
use sim::{Occupant, PlayerInput, Simulation};

fn base_sim() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mods: [&dyn Mod; 4] = [&data, &base::BaseMod, &base_cow::BaseCowMod, &base_movement::BaseMovementMod];
    let mut sim = Simulation::with_mods(&mods).expect("base mod loads");
    sim.spawn_player().expect("base registers a player");
    sim
}

/// The door's lower and upper boxels, and the lever's.
fn spots() -> (BoxelPos, BoxelPos, BoxelPos) {
    let (x, y, z) = base::DOOR_SPOT;
    (BoxelPos::new(x, y, z), BoxelPos::new(x, y, z + 1), BoxelPos::new(x + 1, y, z))
}

/// Stands the feet at `offset` in boxel `(x, y, 0)` and lets the body settle.
fn stand(sim: &mut Simulation, x: i64, y: i64, offset: [f32; 2]) {
    sim.teleport_player(WorldPos::new(I64Vec3::new(x, y, 0), Vec3::new(offset[0], offset[1], 0.0)));
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
}

/// Presses interact looking at `point` in boxel `cell`.
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

/// Everything in the boxel, as one shape.
fn filled(sim: &Simulation, pos: BoxelPos) -> ShapeGrid {
    let blocks = sim.blocks();
    cell(sim, pos).iter().fold(ShapeGrid::EMPTY, |all, o| all.union(&blocks.shape(o.block).turned(o.orientation)))
}

fn is_open(sim: &Simulation, pos: BoxelPos) -> bool {
    let c = cell(sim, pos);
    !c.is_empty() && c.iter().all(|o| sim.blocks().closed_of(o.block).is_some())
}

/// The knob, closed: sticking out of the panel's south face near its east end, halfway up.
const KNOB_CLOSED: [f32; 3] = [0.875, 0.5, 0.625];
/// The knob, open: on the panel's west face, at its south end.
const KNOB_OPEN: [f32; 3] = [0.125, 0.0, 0.625];

#[test]
fn the_door_opens_and_closes_with_its_knob() {
    let mut sim = base_sim();
    let (lower, upper, _) = spots();
    let before = (cell(&sim, lower), cell(&sim, upper));
    assert_eq!(before.0.len(), 4, "wood, iron, brass and copper");
    assert!(filled(&sim, lower).get(10, 14, 2), "closed: the panel across the north side");
    stand(&mut sim, lower.0.x, lower.0.y - 2, [0.5, 0.5]);

    interact(&mut sim, lower, KNOB_CLOSED);
    assert!(is_open(&sim, lower) && is_open(&sim, upper), "both halves swing, as one hinge line");
    let open = filled(&sim, lower);
    assert!(!open.get(10, 14, 2) && open.get(6, 6, 2), "open: the panel along the west side");
    assert_eq!(filled(&sim, upper).get(6, 6, 2), open.get(6, 6, 2), "the halves line up");
    // Open pieces break into the parts they were made as.
    let blocks = sim.blocks();
    for (o, closed) in cell(&sim, lower).iter().zip(&before.0) {
        assert_eq!(blocks.drop(o.block), blocks.drop(closed.block));
    }

    interact(&mut sim, lower, KNOB_OPEN);
    assert_eq!((cell(&sim, lower), cell(&sim, upper)), before, "closed again exactly as it was");
}

#[test]
fn the_lever_beside_it_opens_it_through_the_copper_plate() {
    let mut sim = base_sim();
    let (lower, upper, lever) = spots();
    stand(&mut sim, lever.0.x, lever.0.y - 2, [0.5, 0.5]);
    // The handle sticks out of the lever's south face, a little west of its middle.
    interact(&mut sim, lever, [0.375, 0.0, 0.625]);
    assert!(is_open(&sim, lower) && is_open(&sim, upper));
    assert_eq!(cell(&sim, lever).len(), 2, "the lever itself doesn't move");
    interact(&mut sim, lever, [0.375, 0.0, 0.625]);
    assert!(!is_open(&sim, lower) && !is_open(&sim, upper));
}

#[test]
fn interact_on_the_door_itself_does_nothing() {
    let mut sim = base_sim();
    let (lower, upper, _) = spots();
    stand(&mut sim, lower.0.x, lower.0.y - 2, [0.5, 0.5]);
    // Interact on the wood does nothing: only the trigger works it.
    interact(&mut sim, upper, [0.5, 0.75, 0.5]);
    assert!(!is_open(&sim, lower) && !is_open(&sim, upper));
}

#[test]
fn the_door_wont_swing_into_the_player() {
    let mut sim = base_sim();
    let (lower, upper, _) = spots();
    // In the doorway's south-west, clear of the closed door but where it would swing.
    stand(&mut sim, lower.0.x, lower.0.y, [0.4, 0.3]);
    let feet = sim.player().unwrap().pos();
    assert_eq!(feet.cell(), lower.0, "standing in the doorway");
    let before = (cell(&sim, lower), cell(&sim, upper));
    interact(&mut sim, lower, KNOB_CLOSED);
    assert_eq!((cell(&sim, lower), cell(&sim, upper)), before, "refused: nothing moved");
    // Stepping out, it opens.
    stand(&mut sim, lower.0.x, lower.0.y - 2, [0.5, 0.5]);
    interact(&mut sim, lower, KNOB_CLOSED);
    assert!(is_open(&sim, lower));
}
