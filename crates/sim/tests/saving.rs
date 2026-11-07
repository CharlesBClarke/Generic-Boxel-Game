//! Saving the base game's world and loading it back: into a new game whose made blocks got other
//! IDs, and into the same game to undo what changed since. Blocks travel by name.

use std::collections::BTreeSet;

use api::{BoxelPos, CHUNK_VOLUME, ChunkPos, I64Vec3, LocalPos, Mod, ShapeGrid, Vec3, WorldPos};
use sim::{Cells, Craft, MaterialId, Piece, PlayerInput, SaveData, Simulation};

/// The base game with a small terrain, so tests stay quick. Not generated yet.
fn base_sim() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mods: [&dyn Mod; 4] = [&data, &base::BaseMod, &base_cow::BaseCowMod, &base_movement::BaseMovementMod];
    let mut sim = Simulation::with_mods(&mods).expect("base mod loads");
    sim.spawn_player().expect("base registers a player");
    let w = sim.world_settings_mut();
    w.terrain_width = 2;
    w.terrain_height = 16;
    sim
}

fn generated() -> Simulation {
    let mut sim = base_sim();
    sim.generate_terrain().unwrap();
    sim
}

fn settle(sim: &mut Simulation) {
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
}

/// Stands the feet in the middle of boxel `(x, y, 0)` and lets the body settle.
fn stand(sim: &mut Simulation, x: i64, y: i64) {
    sim.teleport_player(WorldPos::new(I64Vec3::new(x, y, 0), Vec3::new(0.5, 0.5, 0.0)));
    settle(sim);
}

/// The yaw and pitch that look from the eye at `point` in boxel `cell`.
fn aim(sim: &Simulation, cell: BoxelPos, point: [f32; 3]) -> (f32, f32) {
    let to = sim.player().unwrap().eye().delta_to(WorldPos::new(cell.0, Vec3::from_array(point)));
    (to.y.atan2(to.x), to.z.atan2(to.truncate().length()))
}

fn material(sim: &Simulation, name: &str) -> MaterialId {
    sim.blocks().material(name).unwrap()
}

fn table() -> BoxelPos {
    let (x, y, z) = base::TABLE_SPOT;
    BoxelPos::new(x, y, z)
}

/// A slab of wood crafted on the table's top, and a phasite panel with a brass knob on its side
/// (a bundle).
fn craft_things(sim: &mut Simulation) {
    let t = table();
    stand(sim, t.0.x + 1, t.0.y - 4);
    let wood = material(sim, base::WOOD);
    let slab: Cells = [[[Some(Piece::new(wood)), Some(Piece::new(wood)), None, None]; 4]; 4];
    let craft = Craft::grid(t, slab, false, false, 2);
    sim.step(&PlayerInput { craft: Some(craft), ..Default::default() });
    assert!(matches!(sim.last_craft(), Some(Ok(_))), "a slab: {:?}", sim.last_craft());

    let (phasite, brass) = (material(sim, base::PHASITE), material(sim, base::BRASS));
    let mut door: Cells = [[[None; 4]; 4]; 4];
    for line in door.iter_mut() {
        for stack in line.iter_mut() {
            stack[0] = Some(Piece::new(phasite));
        }
    }
    door[1][3][1] = Some(Piece::new(brass));
    let craft = Craft::grid(t, door, true, false, 1);
    sim.step(&PlayerInput { craft: Some(craft), ..Default::default() });
    assert!(matches!(sim.last_craft(), Some(Ok(_))), "a bundle: {:?}", sim.last_craft());
}

/// Opens or shuts the base game's door with the lever beside it, cutting pieces by hiding.
fn work_the_door(sim: &mut Simulation) {
    let (x, y, z) = base::DOOR_SPOT;
    let lever = BoxelPos::new(x + 1, y, z);
    stand(sim, lever.0.x, lever.0.y - 2);
    let (yaw, pitch) = aim(sim, lever, [0.5, 0.0, 0.5]);
    sim.step(&PlayerInput { yaw, pitch, interact_pressed: true, ..Default::default() });
}

/// Breaks the block under the player's feet out on the generated terrain. Returns its chunk.
fn dig_terrain(sim: &mut Simulation) -> ChunkPos {
    // Just above the ground, which teleporting lifts the player onto: a long fall would hurt.
    let ground = base::terrain::Hills::default().ground(5, 7);
    sim.teleport_player(WorldPos::at_boxel(base::TERRAIN_CENTER.offset(I64Vec3::new(5, 7, ground))));
    settle(sim);
    let below = BoxelPos(sim.player().unwrap().pos().cell() - I64Vec3::Z);
    assert!(!sim.universe().main().cell(below).is_empty(), "standing on the ground");
    let pitch = -89.0f32.to_radians();
    sim.step(&PlayerInput { pitch, use_pressed: true, use_held: true, ..Default::default() });
    assert!(sim.universe().main().cell(below).is_empty(), "dug out");
    below.split().0
}

/// The same boxels, by block name and turn, in both worlds; every chunk either has.
fn assert_same_world(a: &Simulation, b: &Simulation) {
    let chunks = |s: &Simulation| s.universe().main().chunks().map(|(p, _)| p.0.to_array()).collect::<BTreeSet<_>>();
    let all: BTreeSet<_> = chunks(a).union(&chunks(b)).copied().collect();
    let named = |s: &Simulation, pos: BoxelPos| {
        let cell = s.universe().main().cell(pos);
        cell.iter().map(|o| (s.blocks().get(o.block).unwrap().name.clone(), o.orientation)).collect::<Vec<_>>()
    };
    for c in all {
        let chunk = ChunkPos(I64Vec3::from_array(c));
        for i in 0..CHUNK_VOLUME {
            let pos = BoxelPos(chunk.0 * api::CHUNK_SIZE + LocalPos::from_index(i).to_ivec());
            let (x, y) = (named(a, pos), named(b, pos));
            assert_eq!(x, y, "at {pos:?}");
        }
    }
}

fn inventory_by_name(sim: &Simulation) -> Vec<(String, u32)> {
    sim.inventory().iter().map(|(m, n)| (sim.blocks().material_name(m).to_owned(), n)).collect::<BTreeSet<_>>().into_iter().collect()
}

#[test]
fn a_world_comes_back_the_same_in_a_new_game() {
    let mut sim = generated();
    craft_things(&mut sim);
    work_the_door(&mut sim);
    let dug = dig_terrain(&mut sim);
    assert!(sim.edited_chunks().contains(&dug), "the dug chunk counts as edited");
    sim.step(&PlayerInput { toggle_fly: true, yaw: 1.25, pitch: -0.5, ..Default::default() });
    assert!(sim.player().unwrap().flying());
    let save = sim.save();
    assert!(save.chunks.iter().any(|c| c.pos == dug), "the edited terrain chunk is saved");
    assert!(!generated().save().chunks.iter().any(|c| c.pos == dug), "but only once edited: it's generated terrain");
    assert!(save.chunks.len() < sim.universe().main().chunks().count(), "untouched terrain isn't saved");
    let bytes = save.to_bytes();

    // A new game that made another shape first, so every made block gets another ID.
    let mut other = base_sim();
    let mut odd = ShapeGrid::EMPTY;
    odd.set(3, 4, 5, true);
    other.shaped_block(material(&other, base::STONE), &odd).unwrap();
    let report = other.load(&SaveData::from_bytes(&bytes).unwrap()).unwrap();
    assert_eq!(report.skipped, Vec::<String>::new());

    assert_same_world(&sim, &other);
    assert_eq!(inventory_by_name(&other), inventory_by_name(&sim));
    let (p, q) = (sim.player().unwrap(), other.player().unwrap());
    assert_eq!(q.pos(), p.pos());
    assert!(q.flying());
    assert_eq!(other.health(), sim.health());
    let items = |s: &Simulation| s.items().iter().map(|i| (s.blocks().material_name(i.drop.material).to_owned(), i.drop.amount, i.pos.cell())).collect::<Vec<_>>();
    assert_eq!(items(&other), items(&sim));
    // Saving the loaded world keeps the same things, look included.
    let (again, first) = (other.save(), SaveData::from_bytes(&bytes).unwrap());
    assert_eq!(again.terrain, first.terrain);
    assert_eq!(again.chunks, first.chunks);
    assert_eq!(again.player, first.player);
    assert_eq!(again.inventory, first.inventory);
    assert_eq!(again.items, first.items);
    assert!(first.made.iter().all(|m| again.made.contains(m)), "every made block, after the other game's own");
}

#[test]
fn loading_undoes_what_changed_since_the_save() {
    let mut sim = generated();
    craft_things(&mut sim);
    let save = sim.save();
    let mut before = generated();
    before.load(&save).unwrap();

    work_the_door(&mut sim);
    let dug = dig_terrain(&mut sim);
    sim.load(&save).unwrap();
    assert_same_world(&sim, &before);
    assert!(!sim.edited_chunks().contains(&dug), "made again from scratch");
    assert_eq!(inventory_by_name(&sim), inventory_by_name(&before));
}

#[test]
fn the_terrain_comes_back_with_the_values_it_was_saved_with() {
    let mut sim = base_sim();
    sim.world_settings_mut().terrain_width = 3;
    sim.generate_terrain().unwrap();
    let save = sim.save();
    let mut other = base_sim();
    other.load(&save).unwrap();
    assert_eq!(other.world_settings().terrain_width, 3);
    assert_eq!(other.generation().unwrap().chunks, 3 * 3 * 16);
}

#[test]
fn what_a_missing_mod_named_is_left_out() {
    let mut sim = generated();
    craft_things(&mut sim);
    let mut save = sim.save();
    let chunk = save.chunks.iter_mut().find(|c| c.lists.iter().any(|l| !l.is_empty())).unwrap();
    let list = chunk.lists.iter_mut().find(|l| !l.is_empty()).unwrap();
    list[0].0 = "gone:thing".into();
    save.inventory.push(("gone:stuff".into(), 5));
    save.terrain.push(("terrain gone:hills".into(), "seed".into(), 1.0));

    let mut other = base_sim();
    let report = other.load(&save).unwrap();
    assert_eq!(report.skipped.len(), 3, "{:?}", report.skipped);
    assert!(report.skipped.iter().any(|s| s.contains("gone:thing")));
    assert!(report.skipped.iter().any(|s| s.contains("gone:stuff")));
    assert!(report.skipped.iter().any(|s| s.contains("gone:hills")));
}
