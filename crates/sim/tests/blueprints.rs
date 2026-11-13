//! Blueprints in the base game: edited through the player's input, made at the table without
//! carrying what they use, and kept in a save by name.

use api::{BoxelPos, I64Vec3, Mod, ShapeGrid, Vec3, WorldPos};
use sim::save::{SavedChange, SavedDesign, SavedThing};
use sim::{
    BlueprintEdit, BlueprintId, BlueprintRefusal, Cells, Change, Craft, Design, MaterialId, Piece, PlayerInput, Recipe, SaveData,
    Simulation, Source, Variation,
};

/// The base game with a small terrain, the player standing in front of the table.
fn at_table() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mods: [&dyn Mod; 4] = [&data, &base::BaseMod, &base_cow::BaseCowMod, &base_movement::BaseMovementMod];
    let mut sim = Simulation::with_mods(&mods).expect("base mod loads");
    sim.spawn_player().expect("base registers a player");
    let w = sim.world_settings_mut();
    w.terrain_height = 16;
    sim.set_view_distance(32.0, 0.0);
    let t = table();
    sim.teleport_player(WorldPos::new(I64Vec3::new(t.0.x + 1, t.0.y - 3, t.0.z), Vec3::new(0.5, 0.5, 0.0)));
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
    sim
}

fn table() -> BoxelPos {
    let (x, y, z) = base::TABLE_SPOT;
    BoxelPos::new(x, y, z)
}

fn material(sim: &Simulation, name: &str) -> MaterialId {
    sim.blocks().material(name).unwrap()
}

fn edit(sim: &mut Simulation, edit: BlueprintEdit) -> Result<BlueprintId, BlueprintRefusal> {
    sim.step(&PlayerInput { blueprint: Some(edit), ..Default::default() });
    sim.last_blueprint().cloned().expect("an outcome")
}

/// A door's corner, on the side: phasite one layer out from the wall in the left column, brass
/// insulation a layer on it at the bottom.
fn corner(sim: &Simulation) -> Recipe {
    let (phasite, brass) = (material(sim, base::PHASITE), material(sim, base::BRASS));
    let mut cells: Cells = [[[None; 4]; 4]; 4];
    for row in cells.iter_mut() {
        row[0][0] = Some(Piece::new(phasite));
    }
    cells[3][0][1] = Some(Piece::new(brass));
    Recipe { cells, side: true, flip: false }
}

#[test]
fn a_variant_swapped_to_antiphasite_is_made_without_making_its_base() {
    let mut sim = at_table();
    let (phasite, antiphasite) = (material(&sim, base::PHASITE), material(&sim, base::ANTIPHASITE));
    let recipe = corner(&sim);
    let base = edit(&mut sim, BlueprintEdit::Save { name: "corner".into(), recipe }).unwrap();
    let swap = Variation::Change(Change::Swap { from: phasite, to: antiphasite });
    let anti = edit(&mut sim, BlueprintEdit::Vary { id: base, variation: swap }).unwrap();
    assert_ne!(anti, base, "a change on a recipe makes a new variant");
    let carried_before: Vec<_> = sim.inventory().iter().collect();
    sim.step(&PlayerInput { craft: Some(Craft { table: table(), source: Source::Blueprint(anti), count: 2 }), ..Default::default() });
    let Some(Ok((made, 2))) = sim.last_craft() else { panic!("{:?}", sim.last_craft()) };
    let pieces: Vec<_> = sim.blocks().pieces(made).iter().map(|&b| sim.blocks().made_of(b).unwrap()).collect();
    assert_eq!(pieces, [antiphasite, material(&sim, base::BRASS)]);
    let carried: Vec<_> = sim.inventory().iter().filter(|(m, _)| *m != made).collect();
    assert_eq!(carried, carried_before, "nothing else carried changed: the base was never made");
}

#[test]
fn a_blueprint_paints_into_a_layer_and_its_variants_follow_it() {
    let mut sim = at_table();
    let wood = material(&sim, base::WOOD);
    // A quarter slab, and a design with it on a wood post.
    let slab: Cells = [[[Some(Piece::new(wood)), None, None, None]; 4]; 4];
    let slab = edit(&mut sim, BlueprintEdit::Save { name: "thin".into(), recipe: Recipe::top(slab) }).unwrap();
    let mut cells: Cells = [[[None; 4]; 4]; 4];
    cells[3][0] = [Some(Piece::new(wood)), Some(Piece::new(slab)), None, None];
    let design = edit(&mut sim, BlueprintEdit::Save { name: "post".into(), recipe: Recipe::top(cells) }).unwrap();
    let turned = edit(&mut sim, BlueprintEdit::Vary { id: design, variation: Variation::Change(Change::Turn { axis: 2, quarters: 2 }) })
        .unwrap();
    let whole = |sim: &Simulation, id| {
        sim.blueprints().resolve(sim.blocks(), id).unwrap().iter().fold(ShapeGrid::EMPTY, |a, (_, s)| a.union(s))
    };
    assert_eq!(whole(&sim, design).volume(), 64 + 16, "a quarter cube and a texel-thin slab on it");
    assert!(whole(&sim, turned).get(15, 15, 0), "turned half way, the post is at the far right");
    // Thicker inner slab: both follow.
    let thicker: Cells = [[[Some(Piece::new(wood)), Some(Piece::new(wood)), None, None]; 4]; 4];
    edit(&mut sim, BlueprintEdit::Overwrite { id: slab, recipe: Recipe::top(thicker) }).unwrap();
    assert_eq!(whole(&sim, design).volume(), 64 + 32);
    assert_eq!(whole(&sim, turned).volume(), 64 + 32);
    assert_eq!(sim.blueprints().roots(), [design], "the slab folds under the design, the variant under it");
}

#[test]
fn blueprints_come_back_from_a_save_by_name() {
    let mut sim = at_table();
    let (phasite, antiphasite) = (material(&sim, base::PHASITE), material(&sim, base::ANTIPHASITE));
    let recipe = corner(&sim);
    let base = edit(&mut sim, BlueprintEdit::Save { name: "corner".into(), recipe }).unwrap();
    // A quarter to the right: still on the quarters, so it fits in a layer.
    let v = edit(&mut sim, BlueprintEdit::Vary { id: base, variation: Variation::Change(Change::Shift([4, 0, 0])) }).unwrap();
    edit(&mut sim, BlueprintEdit::Vary { id: v, variation: Variation::Change(Change::Swap { from: phasite, to: antiphasite }) }).unwrap();
    // Centered, the one-quarter-wide corner is off the quarters: it places, but can't go in a
    // layer.
    let centered = edit(&mut sim, BlueprintEdit::Copy { id: v }).unwrap();
    edit(&mut sim, BlueprintEdit::Vary { id: centered, variation: Variation::Center { axis: 0 } }).unwrap();
    let mut too_fine = Recipe::top([[[None; 4]; 4]; 4]);
    too_fine.cells[0][0][0] = Some(Piece::new(centered));
    let refused = edit(&mut sim, BlueprintEdit::Save { name: "too fine".into(), recipe: too_fine });
    assert!(matches!(refused, Err(BlueprintRefusal::Makes(sim::CraftRefusal::Pattern(api::CraftError::TooFine { .. })))));
    let mut outer = Recipe::top([[[None; 4]; 4]; 4]);
    outer.cells[0][0][0] = Some(Piece { thing: v.into(), turns: 1 });
    edit(&mut sim, BlueprintEdit::Save { name: "outer".into(), recipe: outer }).unwrap();

    let bytes = sim.save().unwrap().to_bytes();
    let mut again = at_table();
    let revision = again.blueprints_revision();
    let report = again.load(&SaveData::from_bytes(&bytes).unwrap()).unwrap();
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    assert_ne!(again.blueprints_revision(), revision, "a load counts as a change, so it's sent");
    let names = |s: &Simulation| s.blueprints().iter().map(|(_, b)| b.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&again), names(&sim));
    for name in names(&sim) {
        let made = |s: &Simulation| {
            let pieces = s.blueprints().resolve(s.blocks(), s.blueprints().find(&name).unwrap()).unwrap();
            pieces.into_iter().map(|(m, shape)| (s.blocks().material_name(m).to_owned(), shape)).collect::<Vec<_>>()
        };
        assert_eq!(made(&again), made(&sim), "{name} makes the same");
    }
    let variant = again.blueprints().find(&names(&sim)[1]).unwrap();
    assert!(matches!(again.blueprints().get(variant).unwrap().design, Design::Variant { .. }), "still linked");
}

#[test]
fn a_blueprint_naming_a_lost_material_is_left_out_with_those_using_it() {
    let mut sim = at_table();
    let recipe = corner(&sim);
    let base = edit(&mut sim, BlueprintEdit::Save { name: "corner".into(), recipe }).unwrap();
    edit(&mut sim, BlueprintEdit::Vary { id: base, variation: Variation::Change(Change::Mirror { axis: 0 }) }).unwrap();
    let wood = material(&sim, base::WOOD);
    let plain = Recipe::top([[[Some(Piece::new(wood)), None, None, None]; 4]; 4]);
    edit(&mut sim, BlueprintEdit::Save { name: "plain".into(), recipe: plain }).unwrap();
    let mut save = sim.save().unwrap();
    let SavedDesign::Recipe { layers, .. } = &mut save.blueprints[0].design else { panic!("the corner is a recipe") };
    let first = layers.iter_mut().flatten().next().unwrap();
    first.0 = SavedThing::Material("gone:unobtainium".into());
    assert!(matches!(&save.blueprints[1].design, SavedDesign::Variant { changes, .. } if changes == &[SavedChange::Mirror { axis: 0 }]));
    let report = sim.load(&save).unwrap();
    assert_eq!(report.skipped.len(), 2, "{:?}", report.skipped);
    let names: Vec<_> = sim.blueprints().iter().map(|(_, b)| b.name.clone()).collect();
    assert_eq!(names, ["plain"]);
}
