//! Crafting at the base game's table: aiming at it, making parts, and parts placed, broken and
//! picked up again.

use api::{BoxelPos, I64Vec3, Mod, ShapeGrid, Vec3, WorldPos};
use sim::{Cells, Craft, CraftRefusal, FULL_VOLUME, Hit, MaterialId, Piece, PlayerInput, Simulation};

fn base_sim() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mods: [&dyn Mod; 4] = [&data, &base::BaseMod, &base_cow::BaseCowMod, &base_movement::BaseMovementMod];
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

/// Cells from text rows, far side first: each character a stack's height in `m`, `.` for none.
fn cells(rows: [&str; 4], m: MaterialId) -> Cells {
    rows.map(|r| {
        let v: Vec<_> = r
            .chars()
            .map(|c| {
                let h = c.to_digit(10).unwrap_or(0) as usize;
                std::array::from_fn(|layer| (layer < h).then_some(Piece::new(m)))
            })
            .collect();
        v.try_into().unwrap()
    })
}

const SLAB: [&str; 4] = ["2222"; 4];

fn request(cells: Cells, count: u32) -> Craft {
    Craft { table: table(), cells, side: false, flip: false, count }
}

fn craft(sim: &mut Simulation, cells: Cells, count: u32) {
    sim.step(&PlayerInput { craft: Some(request(cells, count)), ..Default::default() });
}

#[test]
fn every_face_of_the_table_is_the_table() {
    let mut sim = base_sim();
    let t = table();
    stand(&mut sim, t.0.x, t.0.y - 2);
    assert!(sim.is_table(&look_at(&sim, t, [0.5, 0.0, 0.5])), "the front");
    assert!(sim.is_table(&look_at(&sim, t, [0.5, 0.3, 1.0])), "the top");
    stand(&mut sim, t.0.x - 2, t.0.y);
    assert!(sim.is_table(&look_at(&sim, t, [0.0, 0.5, 0.5])), "a side");
    // The floor isn't a table.
    assert!(!sim.is_table(&look_at(&sim, BoxelPos::new(t.0.x - 1, t.0.y, -1), [0.5, 0.5, 1.0])));
}

#[test]
fn crafting_makes_parts_that_are_placed_broken_and_picked_up_again() {
    let mut sim = base_sim();
    let t = table();
    stand(&mut sim, t.0.x, t.0.y - 2);
    let brick = material(&sim, base::BRICK);
    assert_eq!(sim.inventory().amount(brick), 16 * FULL_VOLUME);

    craft(&mut sim, cells(SLAB, brick), 2);
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
    craft(&mut sim, cells(SLAB, brick), 1);
    assert_eq!(sim.last_craft(), Some(Err(CraftRefusal::TooFar)));

    stand(&mut sim, t.0.x, t.0.y - 2);
    // None of a material is needed: stone isn't carried, and stone slabs are made anyway.
    let stone = material(&sim, base::STONE);
    craft(&mut sim, cells(SLAB, stone), 1);
    assert!(matches!(sim.last_craft(), Some(Ok((_, 1)))), "{:?}", sim.last_craft());
    // The base game carries without limit, so asking for many makes them all. (A limit, and
    // running out of room under it, is tested in `sim::crafting`.)
    craft(&mut sim, cells(SLAB, stone), 1000);
    assert!(matches!(sim.last_craft(), Some(Ok((_, 1000)))), "{:?}", sim.last_craft());
    let elsewhere = Craft { table: BoxelPos::new(t.0.x + 1, t.0.y, t.0.z), ..request(cells(SLAB, brick), 1) };
    sim.step(&PlayerInput { craft: Some(elsewhere), ..Default::default() });
    assert_eq!(sim.last_craft(), Some(Err(CraftRefusal::NoTable)));
    assert_eq!(sim.inventory().amount(brick), 16 * FULL_VOLUME, "brick was never touched");
}

#[test]
fn a_part_in_a_layer_makes_a_finer_part_and_stays() {
    let mut sim = base_sim();
    let t = table();
    stand(&mut sim, t.0.x, t.0.y - 2);
    let wood = material(&sim, base::WOOD);
    // A wall along the far side, then that wall in one layer turned a quarter: a thin wall
    // along the left of its quarter.
    craft(&mut sim, cells(["4444", "....", "....", "...."], wood), 1);
    let Some(Ok((wall, 1))) = sim.last_craft() else { panic!("a wall: {:?}", sim.last_craft()) };
    let mut grid = cells(["...."; 4], wood);
    grid[3][0][0] = Some(Piece { thing: wall, turns: 1 });
    let plan = sim.craft_plan(&request(grid, 1)).unwrap();
    assert_eq!(plan.volume, FULL_VOLUME / 64 / 4, "a quarter cube, a quarter filled");
    assert!(plan.shape.get(0, 0, 0) && !plan.shape.get(3, 0, 0));
    sim.step(&PlayerInput { craft: Some(request(grid, 1)), ..Default::default() });
    let Some(Ok((thin, 1))) = sim.last_craft() else { panic!("{:?}", sim.last_craft()) };
    assert_ne!(thin, wall);
    assert_eq!(sim.inventory().amount(wall), FULL_VOLUME / 4, "the wall is still carried");
    assert_eq!(sim.blocks().part_of(thin), Some(wood));
    // That part has finer detail than quarters, so it can't go in a layer.
    let mut again = cells(["...."; 4], wood);
    again[0][0][0] = Some(Piece::new(thin));
    assert!(matches!(
        sim.craft_plan(&request(again, 1)),
        Err(CraftRefusal::Pattern(api::CraftError::TooFine { row: 0, col: 0, layer: 0 }))
    ));
}

#[test]
fn a_flipped_craft_is_upside_down() {
    let mut sim = base_sim();
    let t = table();
    stand(&mut sim, t.0.x, t.0.y - 2);
    let wood = material(&sim, base::WOOD);
    let plan = sim.craft_plan(&Craft { flip: true, ..request(cells(SLAB, wood), 1) }).unwrap();
    assert!(plan.shape.get(0, 0, 15) && !plan.shape.get(0, 0, 0), "a top slab");
}

#[test]
fn discarding_throws_away_everything_of_one_part() {
    let mut sim = base_sim();
    let t = table();
    stand(&mut sim, t.0.x, t.0.y - 2);
    let wood = material(&sim, base::WOOD);
    craft(&mut sim, cells(SLAB, wood), 5);
    let Some(Ok((slab, 5))) = sim.last_craft() else { panic!("{:?}", sim.last_craft()) };
    sim.step(&PlayerInput { discard: Some(slab), ..Default::default() });
    assert_eq!(sim.inventory().amount(slab), 0, "all five gone");
    assert!(sim.items().is_empty(), "thrown away, not dropped");
    assert_eq!(sim.inventory().amount(wood), 16 * FULL_VOLUME, "nothing else touched");
}

/// A plain door half as cells on the table's side: a phasite panel against the wall with a
/// brass knob a layer out from it, near the top on the right.
fn door_cells(sim: &Simulation) -> Cells {
    let (phasite, brass) = (material(sim, base::PHASITE), material(sim, base::BRASS));
    let mut grid = cells(["...."; 4], phasite);
    for line in grid.iter_mut() {
        for stack in line.iter_mut() {
            stack[0] = Some(Piece::new(phasite));
        }
    }
    grid[1][3][1] = Some(Piece::new(brass));
    grid
}

#[test]
fn several_materials_make_one_bundle_placed_whole_and_broken_whole() {
    let mut sim = base_sim();
    let t = table();
    stand(&mut sim, t.0.x + 1, t.0.y - 4);
    let grid = door_cells(&sim);
    let on_side = Craft { side: true, ..request(grid, 1) };
    let plan = sim.craft_plan(&on_side).unwrap();
    let box_of = |min, max| ShapeGrid::from_boxes(&[api::BlockBox::new(min, max)]);
    let (phasite, brass) = (material(&sim, base::PHASITE), material(&sim, base::BRASS));
    let mut pieces = plan.pieces.clone();
    pieces.sort_by_key(|(m, _)| *m);
    let mut expected = vec![(phasite, box_of([0, 12, 0], [16, 16, 16])), (brass, box_of([12, 8, 8], [16, 12, 12]))];
    expected.sort_by_key(|(m, _)| *m);
    assert_eq!(pieces, expected, "a panel against the wall, the knob out toward you");
    // On the top, the knob would rest on the panel just the same, but the door would lie flat.
    let on_top = sim.craft_plan(&request(grid, 1)).unwrap();
    assert!(on_top.shape.get(8, 8, 0) && !on_top.shape.get(8, 8, 15), "lying flat");

    sim.step(&PlayerInput { craft: Some(on_side), ..Default::default() });
    let Some(Ok((bundle, 1))) = sim.last_craft() else { panic!("a bundle: {:?}", sim.last_craft()) };
    assert_eq!(sim.inventory().amount(bundle), plan.volume);
    let first = sim.blocks().material_block(bundle);

    // Placed on the floor looking north: every piece in one boxel, turned alike, upright.
    let spot = BoxelPos::new(t.0.x + 1, t.0.y - 2, 0);
    let (yaw, pitch) = aim(&sim, BoxelPos::new(spot.0.x, spot.0.y, -1), [0.5, 0.5, 1.0]);
    sim.step(&PlayerInput { yaw, pitch, alt_pressed: true, alt_held: true, place_block: first, ..Default::default() });
    let placed = sim.universe().main().cell(spot).to_vec();
    assert_eq!(placed.iter().map(|o| o.block).collect::<Vec<_>>(), sim.blocks().pieces(bundle));
    assert!(placed.iter().all(|o| o.orientation == api::Orientation::IDENTITY));
    assert_eq!(sim.inventory().amount(bundle), 0, "used up whole");

    // It's a working door: the knob hides the panel and the sixteenth of itself touching it,
    // then brings them back.
    let knob = [0.875, 0.5, 0.625];
    let (yaw, pitch) = aim(&sim, spot, knob);
    sim.step(&PlayerInput { yaw, pitch, interact_pressed: true, ..Default::default() });
    let blocks = sim.blocks();
    let cell = sim.universe().main().cell(spot).to_vec();
    assert_eq!(cell.iter().filter(|o| blocks.is_hidden(o.block)).count(), 2, "the panel, and a slice of the knob");
    assert_eq!(cell.len(), 3, "the rest of the knob stays");
    sim.step(&PlayerInput { yaw: 0.0, pitch: 0.0, ..Default::default() });
    sim.step(&PlayerInput { yaw, pitch, interact_pressed: true, ..Default::default() });
    assert_eq!(sim.universe().main().cell(spot).to_vec(), placed, "and back");

    // Breaking the knob takes off the whole door, and gives it back as one bundle.
    sim.step(&PlayerInput { yaw, pitch, use_pressed: true, use_held: true, ..Default::default() });
    assert!(sim.universe().main().cell(spot).is_empty());
    let dropped: Vec<_> = sim.items().iter().map(|i| i.drop.material).collect();
    assert_eq!(dropped, [bundle]);
}
