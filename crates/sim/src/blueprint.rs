//! Blueprints: the plans gadgets are made from, kept in a player's library rather than
//! carried. A blueprint is either a recipe (stacks on the table, as the crafting menu builds
//! them) or a variant: another blueprint, its base, with changes on top (turned, mirrored,
//! shifted, a material swapped). A variant is linked: it's worked out from its base every time,
//! so editing the base changes every variant of it.
//!
//! A layer on the table can hold a blueprint, so a gadget used only inside a bigger one never
//! has to be made or carried. Crafting is free, so working out what a blueprint makes needs
//! nothing but the blueprints and the materials. A blueprint that uses itself, however far
//! down, makes nothing.
//!
//! **Edits.** A change on a recipe makes a new variant of it; a change on a variant changes
//! that variant, merged with the last change where they're alike (two quarter turns about one
//! axis are a half turn), so undo takes off the last kind of change. Every edit that would
//! leave its blueprint making nothing is refused, and the library stays as it was.

use api::{Orientation, ShapeGrid};

use crate::crafting::{CraftRefusal, Recipe, Thing, design};
use crate::{BlockRegistry, MaterialId};

/// A blueprint in a library. Never reused within a run, so a deleted blueprint's id names
/// nothing. Saves name blueprints instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlueprintId(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct Blueprint {
    /// Unique within its library.
    pub name: String,
    pub design: Design,
}

/// What a blueprint is made from.
// A recipe is a whole grid, far bigger than a variant; libraries are small, so it isn't boxed.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Design {
    /// Stacks on the table.
    Recipe(Recipe),
    /// Another blueprint, with changes made in order.
    Variant { base: BlueprintId, changes: Vec<Change> },
}

/// One change a variant makes to what its base makes. Axes are 0 for X (east, the grid's
/// right), 1 for Y (north, away from you at the table) and 2 for Z (up).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Quarter turns about an axis through the boxel's middle, counterclockwise looking down
    /// the axis from its positive end: 1 to 3.
    Turn { axis: u8, quarters: u8 },
    /// Mirrored across the boxel's middle along an axis.
    Mirror { axis: u8 },
    /// Slid by texels along each axis.
    Shift([i8; 3]),
    /// Every piece of one material made of another instead.
    Swap { from: MaterialId, to: MaterialId },
}

impl Change {
    /// A few words saying what it does, for names and the menu.
    pub fn describe(&self, blocks: &BlockRegistry) -> String {
        const AXES: [&str; 3] = ["X", "Y", "Z"];
        match *self {
            Change::Turn { axis, quarters } => {
                let turn = ["", "quarter turn", "half turn", "three quarter turn"][quarters as usize % 4];
                format!("{turn} {}", AXES[axis as usize % 3])
            }
            Change::Mirror { axis } => format!("mirrored {}", AXES[axis as usize % 3]),
            Change::Shift(by) => {
                let parts: Vec<String> = by
                    .iter()
                    .zip(AXES)
                    .filter(|(b, _)| **b != 0)
                    .map(|(b, a)| format!("{}{b}", a))
                    .collect();
                format!("shifted {}", parts.join(" "))
            }
            Change::Swap { from, to } => {
                format!("{} to {}", short(blocks.material_name(from)), short(blocks.material_name(to)))
            }
        }
    }

    /// `self` then `next` as one change, if they're alike: `Some(None)` when they cancel out.
    fn merged(self, next: Change) -> Option<Option<Change>> {
        match (self, next) {
            (Change::Turn { axis: a, quarters: q }, Change::Turn { axis: b, quarters: r }) if a == b => {
                let quarters = (q + r) % 4;
                Some((quarters != 0).then_some(Change::Turn { axis: a, quarters }))
            }
            (Change::Mirror { axis: a }, Change::Mirror { axis: b }) if a == b => Some(None),
            (Change::Shift(a), Change::Shift(b)) => {
                let by = [0, 1, 2].map(|i| a[i].saturating_add(b[i]));
                Some((by != [0; 3]).then_some(Change::Shift(by)))
            }
            _ => None,
        }
    }

    /// The change made to `pieces`: each material and the texels it fills.
    pub fn apply(&self, pieces: Vec<(MaterialId, ShapeGrid)>) -> Result<Vec<(MaterialId, ShapeGrid)>, CraftRefusal> {
        Ok(match *self {
            Change::Turn { axis, quarters } => {
                let turn = Orientation::quarter_turns(axis as usize % 3, quarters as i32);
                pieces.into_iter().map(|(m, s)| (m, s.turned(turn))).collect()
            }
            Change::Mirror { axis } => pieces.into_iter().map(|(m, s)| (m, s.mirrored(axis as usize % 3))).collect(),
            Change::Shift(by) => {
                let by = by.map(i32::from);
                pieces
                    .into_iter()
                    .map(|(m, s)| Some((m, s.shifted(by)?)))
                    .collect::<Option<_>>()
                    .ok_or(CraftRefusal::OutOfBoxel)?
            }
            Change::Swap { from, to } => {
                // Swapped into a material already there, the two become one piece.
                let mut out: Vec<(MaterialId, ShapeGrid)> = Vec::new();
                for (m, s) in pieces {
                    let m = if m == from { to } else { m };
                    match out.iter_mut().find(|(o, _)| *o == m) {
                        Some((_, shape)) => *shape = shape.union(&s),
                        None => out.push((m, s)),
                    }
                }
                out
            }
        })
    }
}

/// A material's name without its mod, for short labels.
fn short(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// A change asked for from the menu: a [`Change`], or centering, worked out into a shift from
/// where the gadget sits when it's asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variation {
    Change(Change),
    /// Slides it so it's in the middle of the boxel along an axis, or as near as whole texels
    /// go.
    Center { axis: u8 },
}

/// Something a player does to their library.
#[derive(Clone, Debug, PartialEq)]
pub enum BlueprintEdit {
    /// A new blueprint from stacks on the table, named `name` (or the next free name like it).
    Save { name: String, recipe: Recipe },
    /// Puts new stacks in a recipe blueprint.
    Overwrite { id: BlueprintId, recipe: Recipe },
    /// On a recipe, makes a new variant of it with the change; on a variant, adds the change.
    Vary { id: BlueprintId, variation: Variation },
    /// Takes a variant's last change off.
    Undo { id: BlueprintId },
    /// A copy with the same design, linked to the same base if it's a variant.
    Copy { id: BlueprintId },
    Rename { id: BlueprintId, name: String },
    /// Refused while another blueprint uses it.
    Delete { id: BlueprintId },
}

/// Why an edit to the library was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum BlueprintRefusal {
    /// No blueprint with that id.
    Missing,
    /// The name's empty or another blueprint has it.
    Name,
    /// Only a recipe's stacks can be put back on the table and saved over.
    NotRecipe,
    /// Only a variant has changes to undo.
    NoChanges,
    /// Another blueprint uses it, named here.
    InUse(String),
    /// After the edit it would make nothing, for this reason.
    Makes(CraftRefusal),
}

impl std::fmt::Display for BlueprintRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlueprintRefusal::Missing => write!(f, "that blueprint is gone"),
            BlueprintRefusal::Name => write!(f, "that name is empty or taken"),
            BlueprintRefusal::NotRecipe => write!(f, "a variant has no stacks of its own; edit its base"),
            BlueprintRefusal::NoChanges => write!(f, "no changes to undo"),
            BlueprintRefusal::InUse(by) => write!(f, "{by} uses it"),
            BlueprintRefusal::Makes(why) => write!(f, "it would make nothing: {why}"),
        }
    }
}

/// A player's blueprints, in the order they were made.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Library {
    entries: Vec<(BlueprintId, Blueprint)>,
    next: u32,
}

impl Library {
    pub fn get(&self, id: BlueprintId) -> Option<&Blueprint> {
        self.entries.iter().find(|(i, _)| *i == id).map(|(_, b)| b)
    }

    fn get_mut(&mut self, id: BlueprintId) -> Option<&mut Blueprint> {
        self.entries.iter_mut().find(|(i, _)| *i == id).map(|(_, b)| b)
    }

    pub fn find(&self, name: &str) -> Option<BlueprintId> {
        self.entries.iter().find(|(_, b)| b.name == name).map(|(i, _)| *i)
    }

    pub fn iter(&self) -> impl Iterator<Item = (BlueprintId, &Blueprint)> {
        self.entries.iter().map(|(i, b)| (*i, b))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Adds `blueprint` as it is, for loading; its name must be free.
    pub fn insert(&mut self, blueprint: Blueprint) -> Option<BlueprintId> {
        if blueprint.name.trim().is_empty() || self.find(&blueprint.name).is_some() {
            return None;
        }
        let id = BlueprintId(self.next);
        self.next += 1;
        self.entries.push((id, blueprint));
        Some(id)
    }

    /// Replaces a blueprint's design, for loading, where ids are given out before designs are
    /// known.
    pub fn set_design(&mut self, id: BlueprintId, design: Design) {
        if let Some(b) = self.get_mut(id) {
            b.design = design;
        }
    }

    /// `name`, or the first of `name 2`, `name 3` and so on that's free.
    pub fn free_name(&self, name: &str) -> String {
        let name = name.trim();
        let name = if name.is_empty() { "blueprint" } else { name };
        if self.find(name).is_none() {
            return name.to_owned();
        }
        (2..).map(|n| format!("{name} {n}")).find(|n| self.find(n).is_none()).expect("some number is free")
    }

    /// The blueprints `id` uses directly: in its layers, or as its base, each once.
    pub fn uses(&self, id: BlueprintId) -> Vec<BlueprintId> {
        let mut out = Vec::new();
        match self.get(id).map(|b| &b.design) {
            Some(Design::Recipe(recipe)) => {
                for piece in recipe.cells.iter().flatten().flatten().flatten() {
                    if let Thing::Blueprint(b) = piece.thing
                        && !out.contains(&b)
                    {
                        out.push(b);
                    }
                }
            }
            Some(Design::Variant { base, .. }) => out.push(*base),
            None => {}
        }
        out
    }

    /// The blueprints shown under `id` in the menu's tree: those its layers use, then its
    /// variants.
    pub fn children(&self, id: BlueprintId) -> Vec<BlueprintId> {
        let mut out: Vec<BlueprintId> = match self.get(id).map(|b| &b.design) {
            Some(Design::Recipe(_)) => self.uses(id),
            _ => Vec::new(),
        };
        out.extend(self.iter().filter(|(_, b)| matches!(b.design, Design::Variant { base, .. } if base == id)).map(|(i, _)| i));
        out
    }

    /// The blueprints at the top of the tree: every one that isn't in another's layer and
    /// isn't a variant (those fold under their base).
    pub fn roots(&self) -> Vec<BlueprintId> {
        let in_layers: Vec<BlueprintId> = self
            .iter()
            .filter(|(_, b)| matches!(b.design, Design::Recipe(_)))
            .flat_map(|(i, _)| self.uses(i))
            .collect();
        self.iter()
            .filter(|(i, b)| matches!(b.design, Design::Recipe(_)) && !in_layers.contains(i))
            .map(|(i, _)| i)
            .collect()
    }

    /// What `id` makes: each material and the texels it fills, as it stands in the world.
    pub fn resolve(&self, blocks: &BlockRegistry, id: BlueprintId) -> Result<Vec<(MaterialId, ShapeGrid)>, CraftRefusal> {
        self.resolve_from(blocks, id, &mut Vec::new())
    }

    /// [`Library::resolve`], with the blueprints being worked out on the way down to `id`, so
    /// one that uses itself is caught.
    pub(crate) fn resolve_from(
        &self,
        blocks: &BlockRegistry,
        id: BlueprintId,
        path: &mut Vec<BlueprintId>,
    ) -> Result<Vec<(MaterialId, ShapeGrid)>, CraftRefusal> {
        if path.contains(&id) {
            return Err(CraftRefusal::Loop);
        }
        let blueprint = self.get(id).ok_or(CraftRefusal::NoBlueprint)?;
        path.push(id);
        let made = match &blueprint.design {
            Design::Recipe(recipe) => design(blocks, self, recipe, path).map(|(pieces, _)| pieces),
            Design::Variant { base, changes } => self
                .resolve_from(blocks, *base, path)
                .and_then(|pieces| changes.iter().try_fold(pieces, |pieces, change| change.apply(pieces))),
        };
        path.pop();
        made
    }

    /// Makes `edit`, if it leaves every blueprint it touches making something. Gives the
    /// blueprint it made or changed.
    pub fn edit(&mut self, blocks: &BlockRegistry, edit: &BlueprintEdit) -> Result<BlueprintId, BlueprintRefusal> {
        let before = self.clone();
        let result = self.try_edit(blocks, edit);
        let result = result.and_then(|id| match self.get(id) {
            None => Ok(id),
            Some(_) => self.resolve(blocks, id).map(|_| id).map_err(BlueprintRefusal::Makes),
        });
        if result.is_err() {
            *self = before;
        }
        result
    }

    fn try_edit(&mut self, blocks: &BlockRegistry, edit: &BlueprintEdit) -> Result<BlueprintId, BlueprintRefusal> {
        match edit {
            BlueprintEdit::Save { name, recipe } => {
                let name = self.free_name(name);
                Ok(self.insert(Blueprint { name, design: Design::Recipe(*recipe) }).expect("a free name"))
            }
            BlueprintEdit::Overwrite { id, recipe } => {
                let b = self.get_mut(*id).ok_or(BlueprintRefusal::Missing)?;
                match &mut b.design {
                    Design::Recipe(r) => *r = *recipe,
                    Design::Variant { .. } => return Err(BlueprintRefusal::NotRecipe),
                }
                Ok(*id)
            }
            BlueprintEdit::Vary { id, variation } => {
                let b = self.get(*id).ok_or(BlueprintRefusal::Missing)?;
                let change = match *variation {
                    Variation::Change(c) => c,
                    Variation::Center { axis } => {
                        let pieces = self.resolve(blocks, *id).map_err(BlueprintRefusal::Makes)?;
                        let whole = pieces.iter().fold(ShapeGrid::EMPTY, |a, (_, s)| a.union(s));
                        let (lo, hi) = whole.bounds().ok_or(BlueprintRefusal::Makes(CraftRefusal::Pattern(api::CraftError::Empty)))?;
                        let axis = axis as usize % 3;
                        let mut by = [0i8; 3];
                        by[axis] = ((16 - lo[axis] as i32 - hi[axis] as i32) / 2) as i8;
                        Change::Shift(by)
                    }
                };
                match &b.design {
                    Design::Recipe(_) => {
                        let name = self.free_name(&format!("{} {}", b.name, change.describe(blocks)));
                        let design = Design::Variant { base: *id, changes: vec![change] };
                        Ok(self.insert(Blueprint { name, design }).expect("a free name"))
                    }
                    Design::Variant { .. } => {
                        let Some(Blueprint { design: Design::Variant { changes, .. }, .. }) = self.get_mut(*id) else {
                            unreachable!("just matched a variant");
                        };
                        match changes.last().and_then(|last| last.merged(change)) {
                            Some(merged) => {
                                changes.pop();
                                changes.extend(merged);
                            }
                            None => changes.push(change),
                        }
                        Ok(*id)
                    }
                }
            }
            BlueprintEdit::Undo { id } => {
                let b = self.get_mut(*id).ok_or(BlueprintRefusal::Missing)?;
                match &mut b.design {
                    Design::Variant { changes, .. } if !changes.is_empty() => {
                        changes.pop();
                        Ok(*id)
                    }
                    _ => Err(BlueprintRefusal::NoChanges),
                }
            }
            BlueprintEdit::Copy { id } => {
                let b = self.get(*id).ok_or(BlueprintRefusal::Missing)?.clone();
                let name = self.free_name(&b.name);
                Ok(self.insert(Blueprint { name, design: b.design }).expect("a free name"))
            }
            BlueprintEdit::Rename { id, name } => {
                let name = name.trim();
                if name.is_empty() || self.find(name).is_some_and(|other| other != *id) {
                    return Err(BlueprintRefusal::Name);
                }
                self.get_mut(*id).ok_or(BlueprintRefusal::Missing)?.name = name.to_owned();
                Ok(*id)
            }
            BlueprintEdit::Delete { id } => {
                self.get(*id).ok_or(BlueprintRefusal::Missing)?;
                if let Some((_, user)) = self.iter().find(|(other, _)| self.uses(*other).contains(id)) {
                    return Err(BlueprintRefusal::InUse(user.name.clone()));
                }
                self.entries.retain(|(i, _)| i != id);
                Ok(*id)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crafting::{Cells, Piece};
    use api::BlockDef;

    fn setup() -> (BlockRegistry, MaterialId, MaterialId) {
        let mut blocks = BlockRegistry::default();
        blocks.register(BlockDef::new("test:stone", true, [0; 3]).with_material("test:stone")).unwrap();
        blocks.register(BlockDef::new("test:wood", true, [0; 3]).with_material("test:wood")).unwrap();
        let (stone, wood) = (blocks.material("test:stone").unwrap(), blocks.material("test:wood").unwrap());
        (blocks, stone, wood)
    }

    fn cells(rows: [&str; 4], thing: impl Into<Thing> + Copy) -> Cells {
        rows.map(|r| {
            let v: Vec<_> = r
                .chars()
                .map(|c| {
                    let h = c.to_digit(10).unwrap_or(0) as usize;
                    std::array::from_fn(|layer| (layer < h).then_some(Piece::new(thing)))
                })
                .collect();
            v.try_into().unwrap()
        })
    }

    fn recipe(cells: Cells) -> Recipe {
        Recipe { cells, side: false, flip: false }
    }

    fn whole(pieces: &[(MaterialId, ShapeGrid)]) -> ShapeGrid {
        pieces.iter().fold(ShapeGrid::EMPTY, |a, (_, s)| a.union(s))
    }

    /// A post in the far right quarter column, one quarter high.
    const CORNER: [&str; 4] = ["...1", "....", "....", "...."];

    fn save(lib: &mut Library, blocks: &BlockRegistry, name: &str, cells: Cells) -> BlueprintId {
        lib.edit(blocks, &BlueprintEdit::Save { name: name.into(), recipe: recipe(cells) }).unwrap()
    }

    fn vary(lib: &mut Library, blocks: &BlockRegistry, id: BlueprintId, change: Change) -> Result<BlueprintId, BlueprintRefusal> {
        lib.edit(blocks, &BlueprintEdit::Vary { id, variation: Variation::Change(change) })
    }

    #[test]
    fn four_turns_or_two_mirrors_get_back_the_start() {
        let (blocks, stone, _) = setup();
        let mut lib = Library::default();
        let base = save(&mut lib, &blocks, "corner", cells(CORNER, stone));
        let start = lib.resolve(&blocks, base).unwrap();
        for axis in 0..3 {
            let v = vary(&mut lib, &blocks, base, Change::Turn { axis, quarters: 1 }).unwrap();
            assert_ne!(lib.resolve(&blocks, v).unwrap(), start, "one turn about {axis} moves it");
            for _ in 0..3 {
                assert_eq!(vary(&mut lib, &blocks, v, Change::Turn { axis, quarters: 1 }), Ok(v), "changed in place");
            }
            assert_eq!(lib.resolve(&blocks, v).unwrap(), start);
            let Design::Variant { changes, .. } = &lib.get(v).unwrap().design else { panic!() };
            assert!(changes.is_empty(), "four quarter turns merge to nothing");
            let m = vary(&mut lib, &blocks, base, Change::Mirror { axis }).unwrap();
            vary(&mut lib, &blocks, m, Change::Mirror { axis }).unwrap();
            assert_eq!(lib.resolve(&blocks, m).unwrap(), start);
        }
    }

    #[test]
    fn a_variant_follows_its_base() {
        let (blocks, stone, _) = setup();
        let mut lib = Library::default();
        let base = save(&mut lib, &blocks, "corner", cells(CORNER, stone));
        let mirrored = vary(&mut lib, &blocks, base, Change::Mirror { axis: 0 }).unwrap();
        assert_eq!(lib.get(mirrored).unwrap().name, "corner mirrored X");
        assert!(whole(&lib.resolve(&blocks, mirrored).unwrap()).get(0, 15, 0), "now at the far left");
        lib.edit(&blocks, &BlueprintEdit::Overwrite { id: base, recipe: recipe(cells(["1...", "....", "....", "...."], stone)) })
            .unwrap();
        assert!(whole(&lib.resolve(&blocks, mirrored).unwrap()).get(15, 15, 0), "the base moved left, so its mirror moved right");
    }

    #[test]
    fn shifts_center_and_refuse_to_leave_the_boxel() {
        let (blocks, stone, _) = setup();
        let mut lib = Library::default();
        // The upper right quadrant, two quarters each way: centered, it moves a quarter.
        let base = save(&mut lib, &blocks, "block", cells(["..11", "..11", "....", "...."], stone));
        let v = lib.edit(&blocks, &BlueprintEdit::Vary { id: base, variation: Variation::Center { axis: 0 } }).unwrap();
        lib.edit(&blocks, &BlueprintEdit::Vary { id: v, variation: Variation::Center { axis: 1 } }).unwrap();
        let Design::Variant { changes, .. } = &lib.get(v).unwrap().design else { panic!() };
        assert_eq!(changes, &[Change::Shift([-4, -4, 0])], "both shifts merged");
        assert_eq!(whole(&lib.resolve(&blocks, v).unwrap()).bounds(), Some(([4, 4, 0], [12, 12, 4])));
        let before = lib.clone();
        assert_eq!(
            vary(&mut lib, &blocks, v, Change::Shift([0, 0, -1])),
            Err(BlueprintRefusal::Makes(CraftRefusal::OutOfBoxel))
        );
        assert_eq!(lib, before, "a refused edit changes nothing");
        assert_eq!(lib.edit(&blocks, &BlueprintEdit::Undo { id: v }), Ok(v));
        assert_eq!(lib.resolve(&blocks, v).unwrap(), lib.resolve(&blocks, base).unwrap());
    }

    #[test]
    fn swapping_into_a_material_already_there_merges_the_pieces() {
        let (blocks, stone, wood) = setup();
        let mut lib = Library::default();
        let mut mixed = cells(["1111"; 4], stone);
        mixed[0][0][0] = Some(Piece::new(wood));
        let base = save(&mut lib, &blocks, "floor", mixed);
        assert_eq!(lib.resolve(&blocks, base).unwrap().len(), 2);
        let v = vary(&mut lib, &blocks, base, Change::Swap { from: wood, to: stone }).unwrap();
        let swapped = lib.resolve(&blocks, v).unwrap();
        assert_eq!(swapped.len(), 1);
        assert_eq!((swapped[0].0, swapped[0].1.volume()), (stone, 16 * 16 * 4));
    }

    #[test]
    fn a_blueprint_in_a_layer_is_made_without_carrying_it() {
        let (blocks, stone, wood) = setup();
        let mut lib = Library::default();
        let slab = save(&mut lib, &blocks, "slab", cells(["2222"; 4], wood));
        let mut outer = cells(["1111"; 4], stone);
        outer[0][0][1] = Some(Piece::new(slab));
        let door = save(&mut lib, &blocks, "door", outer);
        let made = lib.resolve(&blocks, door).unwrap();
        assert_eq!(made.iter().map(|(m, s)| (*m, s.volume())).collect::<Vec<_>>(), [(stone, 1024), (wood, 32)]);
        assert_eq!(lib.uses(door), [slab]);
        assert_eq!(lib.roots(), [door], "the slab folds under the door");
        assert_eq!(lib.children(door), [slab]);
        assert_eq!(
            lib.edit(&blocks, &BlueprintEdit::Delete { id: slab }),
            Err(BlueprintRefusal::InUse("door".into()))
        );
        assert_eq!(lib.edit(&blocks, &BlueprintEdit::Delete { id: door }), Ok(door));
        assert_eq!(lib.roots(), [slab]);
    }

    #[test]
    fn a_blueprint_that_uses_itself_is_refused() {
        let (blocks, stone, _) = setup();
        let mut lib = Library::default();
        let a = save(&mut lib, &blocks, "a", cells(CORNER, stone));
        let b = save(&mut lib, &blocks, "b", cells(CORNER, a));
        let before = lib.clone();
        let looped = lib.edit(&blocks, &BlueprintEdit::Overwrite { id: a, recipe: recipe(cells(CORNER, b)) });
        assert_eq!(looped, Err(BlueprintRefusal::Makes(CraftRefusal::Loop)));
        assert_eq!(lib, before);
    }

    #[test]
    fn variants_fold_under_their_base_and_names_stay_unique() {
        let (blocks, stone, _) = setup();
        let mut lib = Library::default();
        let a = save(&mut lib, &blocks, "corner", cells(CORNER, stone));
        let again = save(&mut lib, &blocks, "corner", cells(CORNER, stone));
        assert_eq!(lib.get(again).unwrap().name, "corner 2");
        let v = vary(&mut lib, &blocks, a, Change::Turn { axis: 2, quarters: 1 }).unwrap();
        let copy = lib.edit(&blocks, &BlueprintEdit::Copy { id: v }).unwrap();
        assert_eq!(lib.roots(), [a, again]);
        assert_eq!(lib.children(a), [v, copy]);
        assert_eq!(lib.edit(&blocks, &BlueprintEdit::Rename { id: copy, name: "corner".into() }), Err(BlueprintRefusal::Name));
        assert_eq!(lib.edit(&blocks, &BlueprintEdit::Rename { id: copy, name: "left corner".into() }), Ok(copy));
    }
}
