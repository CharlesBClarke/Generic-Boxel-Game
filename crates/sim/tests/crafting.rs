//! Crafting at the base game's table: which face is aimed at, making parts, and parts placed,
//! broken and picked up again.

use api::{ApiError, BoxelPos, I64Vec3, Mod, Orientation, Registrar, TableFace, Vec3, WorldEdit, WorldPos};
use sim::{Craft, CraftRefusal, FULL_VOLUME, Hit, MaterialId, PlayerInput, Simulation};

/// A second crafting table, a quarter turn counterclockwise, so its front faces east.
struct TurnedTable;

const TURNED: BoxelPos = BoxelPos::new(-2, 8, 0);

impl Mod for TurnedTable {
    fn name(&self) -> &str {
        "turned"
    }

    fn register(&self, _: &mut dyn Registrar) -> Result<(), ApiError> {
        Ok(())
    }

    fn build_world(&self, w: &mut dyn WorldEdit) -> Result<(), ApiError> {
        w.add_block(TURNED, base::CRAFTING_TABLE, Orientation::quarter_turns(2, 1))
    }
}

fn base_sim() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mods: [&dyn Mod; 4] = [&data, &base::BaseMod, &base_movement::BaseMovementMod, &TurnedTable];
    let mut sim = Simulation::with_mods(&mods).expect("base mod loads");
    sim.spawn_player().expect("base registers a player");
    sim
}

fn table() -> BoxelPos {
    let (x, y, z) = base::TABLE_SPOT;
    BoxelPos::new(x, y, z)
}

/// Stands the feet in the middle of boxel `(x, y, 0)` and lets the body settle.
fn stand(sim: &mut Simulation, x: i64, y: i64) {
    sim.teleport_player(WorldPos::new(I64Vec3::new(x, y, 0), Vec3::new(0.5, 0.5, 0.0)));
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
}

/// The yaw and pitch that look from the eye at `point` in boxel `cell`.
fn aim(sim: &Simulation, cell: BoxelPos, point: [f32; 3]) -> (f32, f32) {
    let to = sim.player().unwrap().eye().delta_to(WorldPos::new(cell.0, Vec3::from_array(point)));
    (to.y.atan2(to.x), to.z.atan2(to.truncate().length()))
}

/// What the player hits looking at `point` in `cell`.
fn look_at(sim: &Simulation, cell: BoxelPos, point: [f32; 3]) -> Hit {
    let (yaw, pitch) = aim(sim, cell, point);
    sim.target(yaw, pitch).expect("in reach")
}

fn material(sim: &Simulation, name: &str) -> MaterialId {
    sim.blocks().material(name).unwrap()
}

/// Cells from text rows, top first: `#` is `m`, `.` empty.
fn cells(rows: [&str; 4], m: MaterialId) -> [[Option<MaterialId>; 4]; 4] {
    rows.map(|r| {
        let v: Vec<_> = r.chars().map(|c| (c == '#').then_some(m)).collect();
        v.try_into().unwrap()
    })
}

const SLAB: [&str; 4] = ["....", "....", "####", "####"];

fn craft(sim: &mut Simulation, face: TableFace, cells: [[Option<MaterialId>; 4]; 4], count: u32) {
    let craft = Craft { table: table(), face, cells, count };
    sim.step(&PlayerInput { craft: Some(craft), ..Default::default() });
}

#[test]
fn aiming_at_the_table_tells_which_face() {
    let mut sim = base_sim();
    let t = table();
    // The base table is unturned: its front faces south.
    stand(&mut sim, t.0.x, t.0.y - 2);
    assert_eq!(sim.table_face(&look_at(&sim, t, [0.5, 0.0, 0.5])), Some(TableFace::Front));
    assert_eq!(sim.table_face(&look_at(&sim, t, [0.5, 0.3, 1.0])), Some(TableFace::Top));
    stand(&mut sim, t.0.x - 2, t.0.y);
    assert_eq!(sim.table_face(&look_at(&sim, t, [0.0, 0.5, 0.5])), Some(TableFace::Left));
    stand(&mut sim, t.0.x + 2, t.0.y);
    assert_eq!(sim.table_face(&look_at(&sim, t, [1.0, 0.5, 0.5])), Some(TableFace::Right));
    stand(&mut sim, t.0.x, t.0.y + 2);
    assert_eq!(sim.table_face(&look_at(&sim, t, [0.5, 1.0, 0.5])), Some(TableFace::Back));
    // The floor isn't a table.
    assert_eq!(sim.table_face(&look_at(&sim, BoxelPos::new(t.0.x, t.0.y + 1, -1), [0.5, 0.5, 1.0])), None);

    // A table turned a quarter counterclockwise has its front facing east.
    stand(&mut sim, TURNED.0.x + 2, TURNED.0.y);
    assert_eq!(sim.table_face(&look_at(&sim, TURNED, [1.0, 0.5, 0.5])), Some(TableFace::Front));
    // Standing at that front looking west, south is on the left.
    stand(&mut sim, TURNED.0.x, TURNED.0.y - 2);
    assert_eq!(sim.table_face(&look_at(&sim, TURNED, [0.5, 0.0, 0.5])), Some(TableFace::Left));
}

#[test]
fn crafting_makes_parts_that_are_placed_broken_and_picked_up_again() {
    let mut sim = base_sim();
    let t = table();
    stand(&mut sim, t.0.x, t.0.y - 2);
    let brick = material(&sim, base::BRICK);
    assert_eq!(sim.inventory().amount(brick), 16 * FULL_VOLUME);

    craft(&mut sim, TableFace::Front, cells(SLAB, brick), 2);
    let Some(Ok((part, 2))) = sim.last_craft() else { panic!("two slabs: {:?}", sim.last_craft()) };
    assert_eq!(sim.inventory().amount(brick), 16 * FULL_VOLUME, "crafting is free");
    assert_eq!(sim.inventory().amount(part), FULL_VOLUME, "two slabs carried");
    let slab = sim.blocks().material_block(part);
    assert_eq!(Some(slab), sim.blocks().find_shaped(brick, &base::shapes::slab()), "the test area's own brick slab");

    // Placing one uses one.
    let spot = BoxelPos::new(t.0.x + 1, t.0.y - 2, 0);
    let (yaw, pitch) = aim(&sim, BoxelPos::new(spot.0.x, spot.0.y, -1), [0.5, 0.5, 1.0]);
    sim.step(&PlayerInput { yaw, pitch, alt_pressed: true, alt_held: true, place_block: slab, ..Default::default() });
    assert_eq!(sim.universe().main().cell(spot).iter().map(|o| o.block).collect::<Vec<_>>(), [slab]);
    assert_eq!(sim.inventory().amount(part), FULL_VOLUME / 2);

    // Breaking it gives the slab back whole, and walking over it picks it up.
    let (yaw, pitch) = aim(&sim, spot, [0.5, 0.5, 0.5]);
    sim.step(&PlayerInput { yaw, pitch, use_pressed: true, use_held: true, ..Default::default() });
    assert!(sim.universe().main().cell(spot).is_empty());
    assert_eq!(sim.items().iter().map(|i| (i.drop.material, i.drop.amount)).collect::<Vec<_>>(), [(part, FULL_VOLUME / 2)]);
    for _ in 0..120 {
        sim.step(&PlayerInput::default());
    }
    assert!(sim.items().is_empty(), "picked up");
    assert_eq!(sim.inventory().amount(part), FULL_VOLUME);
    assert_eq!(sim.inventory().amount(brick), 16 * FULL_VOLUME, "and still not brick");
}

#[test]
fn crafting_needs_the_table_in_reach_and_nothing_else() {
    let mut sim = base_sim();
    let t = table();
    let brick = material(&sim, base::BRICK);
    stand(&mut sim, t.0.x + 8, t.0.y);
    craft(&mut sim, TableFace::Front, cells(SLAB, brick), 1);
    assert_eq!(sim.last_craft(), Some(Err(CraftRefusal::TooFar)));

    stand(&mut sim, t.0.x, t.0.y - 2);
    // None of a material is needed: stone isn't carried, and stone slabs are made anyway.
    let stone = material(&sim, base::STONE);
    craft(&mut sim, TableFace::Front, cells(SLAB, stone), 1);
    assert!(matches!(sim.last_craft(), Some(Ok((_, 1)))), "{:?}", sim.last_craft());
    // The base game carries without limit, so asking for many makes them all. (A limit, and
    // running out of room under it, is tested in `sim::crafting`.)
    craft(&mut sim, TableFace::Front, cells(SLAB, stone), 1000);
    assert!(matches!(sim.last_craft(), Some(Ok((_, 1000)))), "{:?}", sim.last_craft());
    let elsewhere =
        Craft { table: BoxelPos::new(t.0.x + 1, t.0.y, t.0.z), face: TableFace::Front, cells: cells(SLAB, brick), count: 1 };
    sim.step(&PlayerInput { craft: Some(elsewhere), ..Default::default() });
    assert_eq!(sim.last_craft(), Some(Err(CraftRefusal::NoTable)));
    assert_eq!(sim.inventory().amount(brick), 16 * FULL_VOLUME, "brick was never touched");
}

#[test]
fn a_part_in_a_cell_makes_a_finer_part_and_stays() {
    let mut sim = base_sim();
    let t = table();
    stand(&mut sim, t.0.x, t.0.y - 2);
    let wood = material(&sim, base::WOOD);
    // A pipe at the front, then the pipe in one cell at the right side: four little holes.
    craft(&mut sim, TableFace::Front, cells(["####", "#..#", "#..#", "####"], wood), 1);
    let Some(Ok((pipe, 1))) = sim.last_craft() else { panic!("a pipe: {:?}", sim.last_craft()) };
    let mut grid = cells(["...."; 4], wood);
    grid[3][0] = Some(pipe);
    let craft = Craft { table: t, face: TableFace::Right, cells: grid, count: 1 };
    let plan = sim.craft_plan(&craft).unwrap();
    assert_eq!(plan.volume, FULL_VOLUME / 16 * 12 / 16, "a quarter column, three quarters filled");
    sim.step(&PlayerInput { craft: Some(craft), ..Default::default() });
    let Some(Ok((holes, 1))) = sim.last_craft() else { panic!("{:?}", sim.last_craft()) };
    assert_ne!(holes, pipe);
    assert_eq!(sim.inventory().amount(pipe), FULL_VOLUME * 12 / 16, "the pipe is still carried");
    assert_eq!(sim.blocks().part_of(holes), Some(wood));
    // That part has finer detail than quarters, so it can't go in a cell.
    let mut again = cells(["...."; 4], wood);
    again[0][0] = Some(holes);
    let craft = Craft { table: t, face: TableFace::Front, cells: again, count: 1 };
    assert!(matches!(sim.craft_plan(&craft), Err(CraftRefusal::Pattern(api::CraftError::TooFine { row: 0, col: 0 }))));
}

#[test]
fn discarding_throws_away_everything_of_one_part() {
    let mut sim = base_sim();
    let t = table();
    stand(&mut sim, t.0.x, t.0.y - 2);
    let wood = material(&sim, base::WOOD);
    craft(&mut sim, TableFace::Front, cells(SLAB, wood), 5);
    let Some(Ok((slab, 5))) = sim.last_craft() else { panic!("{:?}", sim.last_craft()) };
    sim.step(&PlayerInput { discard: Some(slab), ..Default::default() });
    assert_eq!(sim.inventory().amount(slab), 0, "all five gone");
    assert!(sim.items().is_empty(), "thrown away, not dropped");
    assert_eq!(sim.inventory().amount(wood), 16 * FULL_VOLUME, "nothing else touched");
}
