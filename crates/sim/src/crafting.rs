//! Crafting at a table: what the player stacked on it, turned into parts. The shape rule
//! itself is the API's ([`api::crafting`]); this adds materials and room. Every layer holds
//! a material, or a part (a bundle's pieces each bring their own). Layers of one material make
//! a part of it; several materials make a bundle, one piece per material. Crafting is free for
//! now (paying for it is deferred): a part costs nothing to make, but takes its volume of the
//! capacity to carry.

use api::crafting::GRID;
use api::{BoxelPos, CraftError, Layer, Orientation, ShapeGrid, Stacks, finished, stack_fill};

use crate::{BlockRegistry, Inventory, MaterialId};

/// One layer on the table: a material, or a part turned some quarter turns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub thing: MaterialId,
    /// Quarter turns counterclockwise seen from above, 0 to 3. A material ignores it.
    pub turns: u8,
}

impl Piece {
    pub fn new(thing: MaterialId) -> Piece {
        Piece { thing, turns: 0 }
    }
}

/// What's on the table: each cell's stack, bottom first, by row from the far side and column
/// from the left.
pub type Cells = [[[Option<Piece>; GRID]; GRID]; GRID];

/// A request to craft at a table, as the player makes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Craft {
    /// Where the table is.
    pub table: BoxelPos,
    pub cells: Cells,
    /// Turn the made shape upside down.
    pub flip: bool,
    /// Built on the table's side: the grid is the wall, and stacks grow toward the player.
    pub side: bool,
    /// The most parts to make; as many as there's room to carry, up to this.
    pub count: u32,
}

/// What a craft would make.
#[derive(Clone, Debug, PartialEq)]
pub struct CraftPlan {
    /// Each material it's made of and the texels it fills, in the order first stacked:
    /// one makes a part, several a bundle.
    pub pieces: Vec<(MaterialId, ShapeGrid)>,
    /// The whole shape, in the table's frame.
    pub shape: ShapeGrid,
    /// Each part's volume, in [`crate::FULL_VOLUME`] units: the room it takes.
    pub volume: u32,
    /// How many it would make: as many as asked for, or fewer if there's no room for more.
    pub count: u32,
}

/// Why crafting makes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CraftRefusal {
    /// No mod defines crafting.
    NoCrafting,
    /// There's no crafting table there.
    NoTable,
    /// The table is out of reach.
    TooFar,
    /// The cells hold more materials than a boxel holds blocks.
    TooManyMaterials,
    /// The pattern makes nothing.
    Pattern(CraftError),
    /// The pattern fills the whole boxel, which is just the material's own block.
    FullBlock,
    /// The material can't take shapes: it has no full block to copy.
    NotShapeable,
    /// No room to carry even one.
    Full,
}

impl std::fmt::Display for CraftRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CraftRefusal::NoCrafting => write!(f, "nothing can be crafted"),
            CraftRefusal::NoTable => write!(f, "no crafting table there"),
            CraftRefusal::TooFar => write!(f, "the table is out of reach"),
            CraftRefusal::TooManyMaterials => write!(f, "at most {} materials in one shape", crate::MAX_MATERIALS),
            CraftRefusal::Pattern(e) => write!(f, "{e}"),
            CraftRefusal::FullBlock => write!(f, "that's a full block, nothing to shape"),
            CraftRefusal::NotShapeable => write!(f, "that material can't be shaped"),
            CraftRefusal::Full => write!(f, "no room to carry it"),
        }
    }
}

/// What `cells` make, built on the side if `side`, flipped if `flip`, and how many fit in
/// `inventory` under `capacity`, up to `count`.
#[allow(clippy::too_many_arguments)]
pub fn plan(
    blocks: &BlockRegistry,
    inventory: &Inventory,
    capacity: Option<u32>,
    cells: &Cells,
    side: bool,
    flip: bool,
    count: u32,
) -> Result<CraftPlan, CraftRefusal> {
    // The whole, and each material's own layers, by material in the order first met.
    let mut whole: Stacks = Default::default();
    let mut by_material: Vec<(MaterialId, Stacks)> = Vec::new();
    for (row, line) in cells.iter().enumerate() {
        for (col, stack) in line.iter().enumerate() {
            for (layer_index, &piece) in stack.iter().enumerate() {
                let Some(piece) = piece.filter(|p| p.thing.index() < blocks.material_count()) else {
                    continue;
                };
                // A part goes in as it stands in the world: on the side, tipped back into the
                // wall's frame first, so the craft's tip brings it upright again. Then the
                // brush's turn, about the stacks' axis.
                let turn = Orientation::quarter_turns(2, piece.turns as i32);
                let framed = |shape: ShapeGrid| {
                    let shape = if side { api::crafting::untipped(&shape) } else { shape };
                    Layer::Shape(Box::new(shape.turned(turn)))
                };
                let id = piece.thing;
                let layers: Vec<(MaterialId, Layer)> = match (blocks.pieces(id), blocks.part_of(id)) {
                    ([], None) => vec![(id, Layer::Solid)],
                    ([], Some(of)) => vec![(of, framed(blocks.shape(blocks.material_block(id))))],
                    (pieces, _) => pieces.iter().filter_map(|&b| Some((blocks.made_of(b)?, framed(blocks.shape(b))))).collect(),
                };
                let mut shape = ShapeGrid::EMPTY;
                for (material, layer) in layers {
                    shape = shape.union(&match &layer {
                        Layer::Solid => ShapeGrid::FULL,
                        Layer::Shape(s) => (**s).clone(),
                    });
                    let i = match by_material.iter().position(|(m, _)| *m == material) {
                        Some(i) => i,
                        None => {
                            by_material.push((material, Default::default()));
                            by_material.len() - 1
                        }
                    };
                    // A bundle holds one piece per material, so each material has one share here.
                    by_material[i].1[row][col][layer_index] = Some(layer);
                }
                whole[row][col][layer_index] = Some(if shape.is_full() { Layer::Solid } else { Layer::Shape(Box::new(shape)) });
            }
        }
    }
    let shape = api::crafting::stack_shape(&whole).map_err(CraftRefusal::Pattern)?;
    if shape.is_full() && by_material.len() == 1 {
        return Err(CraftRefusal::FullBlock);
    }
    if by_material.len() > crate::MAX_MATERIALS {
        return Err(CraftRefusal::TooManyMaterials);
    }
    let mut pieces = Vec::new();
    for (material, stacks) in &by_material {
        if !blocks.get(blocks.material_block(*material)).is_some_and(|def| def.is_full()) {
            return Err(CraftRefusal::NotShapeable);
        }
        let own = stack_fill(stacks).map_err(CraftRefusal::Pattern)?;
        pieces.push((*material, finished(&own, side, flip)));
    }
    let shape = finished(&shape, side, flip);
    let volume = shape.volume();
    let count = count.min(inventory.room(capacity) / volume);
    if count == 0 {
        return Err(CraftRefusal::Full);
    }
    Ok(CraftPlan { pieces, shape, volume, count })
}

/// Makes what `plan` says: registers the part's block, or the bundle, if it's new and adds
/// them. Returns the part or bundle.
pub fn make(
    blocks: &mut BlockRegistry,
    inventory: &mut Inventory,
    capacity: Option<u32>,
    plan: &CraftPlan,
) -> Result<MaterialId, CraftRefusal> {
    let part = match plan.pieces.as_slice() {
        [(material, shape)] => {
            let block = blocks.shaped(*material, shape).map_err(|_| CraftRefusal::NotShapeable)?;
            blocks.drop(block).expect("a shaped block drops itself").material
        }
        pieces => blocks.bundle(pieces).map_err(|_| CraftRefusal::NotShapeable)?,
    };
    let total = plan.volume * plan.count;
    if inventory.room(capacity) < total {
        return Err(CraftRefusal::Full);
    }
    inventory.add(part, total, capacity);
    Ok(part)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FULL_VOLUME;
    use api::BlockDef;

    fn setup() -> (BlockRegistry, Inventory, MaterialId, MaterialId) {
        let mut blocks = BlockRegistry::default();
        blocks.register(BlockDef::new("test:stone", true, [0; 3]).with_material("test:stone")).unwrap();
        blocks.register(BlockDef::new("test:wood", true, [0; 3]).with_material("test:wood")).unwrap();
        let (stone, wood) = (blocks.material("test:stone").unwrap(), blocks.material("test:wood").unwrap());
        let mut inventory = Inventory::default();
        inventory.add(stone, 4 * FULL_VOLUME, None);
        (blocks, inventory, stone, wood)
    }

    /// Room for 8 boxels in all.
    const CAPACITY: Option<u32> = Some(8 * FULL_VOLUME);

    /// Cells from text rows, far side first: each character a stack's height in `m`, `.` for
    /// none.
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

    #[test]
    fn crafting_is_free_but_parts_take_room() {
        let (mut blocks, mut inventory, stone, wood) = setup();
        let plan = plan(&blocks, &inventory, CAPACITY, &cells(SLAB, stone), false, false, 1).unwrap();
        assert_eq!((plan.pieces[0].0, plan.volume, plan.count), (stone, FULL_VOLUME / 2, 1));
        let part = make(&mut blocks, &mut inventory, CAPACITY, &plan).unwrap();
        assert_eq!(inventory.amount(stone), 4 * FULL_VOLUME, "no stone taken");
        assert_eq!(inventory.amount(part), FULL_VOLUME / 2, "one slab");
        assert_eq!(blocks.part_of(part), Some(stone));
        // None of a material is needed to shape it.
        let wood_slab = super::plan(&blocks, &inventory, CAPACITY, &cells(SLAB, wood), false, false, 1).unwrap();
        assert_eq!(wood_slab.pieces[0].0, wood);
    }

    #[test]
    fn asking_for_many_makes_what_fits() {
        let (blocks, inventory, stone, _) = setup();
        let many = plan(&blocks, &inventory, CAPACITY, &cells(SLAB, stone), false, false, 100).unwrap();
        assert_eq!(many.count, 8, "4 boxels of room left holds 8 slabs");
        let unlimited = plan(&blocks, &inventory, None, &cells(SLAB, stone), false, false, 100).unwrap();
        assert_eq!(unlimited.count, 100);
        let full = Some(4 * FULL_VOLUME);
        assert_eq!(plan(&blocks, &inventory, full, &cells(SLAB, stone), false, false, 1), Err(CraftRefusal::Full));
    }

    #[test]
    fn several_materials_make_a_bundle_of_pieces() {
        let (mut blocks, mut inventory, stone, wood) = setup();
        // A stone slab with a wood quarter cube on it, at the near left.
        let mut mixed = cells(SLAB, stone);
        mixed[3][0][2] = Some(Piece::new(wood));
        let p = plan(&blocks, &inventory, CAPACITY, &mixed, false, false, 1).unwrap();
        let slab = api::craft_shape(&api::crafting::heights(SLAB), false).unwrap();
        assert_eq!(p.pieces.len(), 2);
        assert_eq!((p.pieces[0].0, &p.pieces[0].1), (stone, &slab));
        assert_eq!(p.pieces[1].0, wood);
        assert!(p.pieces[1].1.get(0, 0, 8) && p.pieces[1].1.volume() == 64, "the cube on the slab");
        assert_eq!(p.volume, FULL_VOLUME / 2 + 64);
        let bundle = make(&mut blocks, &mut inventory, CAPACITY, &p).unwrap();
        assert_eq!(inventory.amount(bundle), p.volume, "carried whole, as one");
        assert_eq!(blocks.unit(bundle), p.volume);
        assert_eq!(blocks.part_of(bundle), Some(stone));
        let pieces = blocks.pieces(bundle).to_vec();
        assert_eq!(pieces.len(), 2);
        assert_eq!(blocks.material_block(bundle), pieces[0], "held and placed as its first piece");
        assert_eq!(blocks.companions(pieces[0]), &pieces[1..]);
        assert!(blocks.companions(pieces[1]).is_empty());
        assert_eq!(blocks.cost(pieces[0]), Some(crate::Drop { material: bundle, amount: p.volume }));
        // Each piece breaks into an ordinary part of its material.
        for (&piece, (material, shape)) in pieces.iter().zip(&p.pieces) {
            let drop = blocks.drop(piece).unwrap();
            assert_eq!(blocks.part_of(drop.material), Some(*material));
            assert_eq!(blocks.shape(piece), *shape);
            assert_eq!(blocks.made_of(piece), Some(*material));
        }
        let again = make(&mut blocks, &mut inventory, CAPACITY, &p).unwrap();
        assert_eq!(again, bundle, "the same design is the same bundle");
        // Filling the boxel is fine once there's more than one material.
        let mut full = cells(["4444"; 4], stone);
        full[0][0][3] = Some(Piece::new(wood));
        assert!(plan(&blocks, &inventory, CAPACITY, &full, false, false, 1).is_ok());
    }

    #[test]
    fn a_bundle_in_a_layer_shrinks_each_piece() {
        let (mut blocks, mut inventory, stone, wood) = setup();
        let mut mixed = cells(SLAB, stone);
        mixed[3][0][2] = Some(Piece::new(wood));
        let p = plan(&blocks, &inventory, CAPACITY, &mixed, false, false, 1).unwrap();
        let bundle = make(&mut blocks, &mut inventory, CAPACITY, &p).unwrap();
        let mut one = cells(["...."; 4], stone);
        one[0][0][0] = Some(Piece::new(bundle));
        let small = plan(&blocks, &inventory, CAPACITY, &one, false, false, 1).unwrap();
        let volumes: Vec<_> = small.pieces.iter().map(|(m, s)| (*m, s.volume())).collect();
        assert_eq!(volumes, [(stone, 32), (wood, 1)]);
    }

    #[test]
    fn at_most_eight_materials() {
        let (mut blocks, inventory, stone, wood) = setup();
        let more: Vec<MaterialId> = ["a", "b", "c", "d", "e", "f", "g"]
            .iter()
            .map(|n| {
                blocks.register(BlockDef::new(format!("test:{n}"), true, [0; 3]).with_material(format!("test:{n}"))).unwrap();
                blocks.material(&format!("test:{n}")).unwrap()
            })
            .collect();
        // Stone, wood and seven more: nine.
        let mut nine = cells(["1111"; 4], stone);
        for (i, &m) in std::iter::once(&wood).chain(&more).enumerate() {
            nine[i / 4][i % 4][0] = Some(Piece::new(m));
        }
        assert_eq!(plan(&blocks, &inventory, CAPACITY, &nine, false, false, 1), Err(CraftRefusal::TooManyMaterials));
        nine[1][3][0] = Some(Piece::new(stone));
        assert_eq!(plan(&blocks, &inventory, CAPACITY, &nine, false, false, 1).unwrap().pieces.len(), 8);
    }

    #[test]
    fn full_and_empty_patterns_make_nothing() {
        let (blocks, inventory, stone, _) = setup();
        let full = cells(["4444"; 4], stone);
        assert_eq!(plan(&blocks, &inventory, CAPACITY, &full, false, false, 1), Err(CraftRefusal::FullBlock));
        let empty = cells(["...."; 4], stone);
        assert_eq!(
            plan(&blocks, &inventory, CAPACITY, &empty, false, false, 1),
            Err(CraftRefusal::Pattern(CraftError::Empty))
        );
    }

    #[test]
    fn a_part_in_a_layer_is_a_pattern_and_is_not_used_up() {
        let (mut blocks, mut inventory, stone, wood) = setup();
        let slab = plan(&blocks, &inventory, CAPACITY, &cells(SLAB, stone), false, false, 1).unwrap();
        let part = make(&mut blocks, &mut inventory, CAPACITY, &slab).unwrap();
        let mut with_part = cells(["...."; 4], stone);
        with_part[0][0][0] = Some(Piece::new(part));
        with_part[3][3][0] = Some(Piece::new(stone));
        let p = plan(&blocks, &inventory, CAPACITY, &with_part, false, false, 1).unwrap();
        // A whole quarter cube, plus a quarter cube holding a slab shrunk: half of one.
        assert_eq!(p.volume, FULL_VOLUME / 64 + FULL_VOLUME / 128);
        assert_eq!(p.pieces[0].0, stone, "made of the part's material");
        let made = make(&mut blocks, &mut inventory, CAPACITY, &p).unwrap();
        assert_eq!(inventory.amount(part), FULL_VOLUME / 2, "the slab is still there");
        assert_eq!(inventory.amount(made), p.volume);
        // A part of one material beside another material makes a bundle.
        let mut mixed = with_part;
        mixed[3][3][0] = Some(Piece::new(wood));
        assert_eq!(plan(&blocks, &inventory, CAPACITY, &mixed, false, false, 1).unwrap().pieces.len(), 2);
        // And the finer part can't go in a layer.
        let mut finer = cells(["...."; 4], stone);
        finer[1][1][0] = Some(Piece::new(made));
        assert_eq!(
            plan(&blocks, &inventory, CAPACITY, &finer, false, false, 1),
            Err(CraftRefusal::Pattern(CraftError::TooFine { row: 1, col: 1, layer: 0 }))
        );
    }

    #[test]
    fn a_turned_part_goes_in_turned_and_flipping_turns_the_result() {
        let (mut blocks, mut inventory, stone, _) = setup();
        // A wall along the far side.
        let wall = plan(&blocks, &inventory, CAPACITY, &cells(["4444", "....", "....", "...."], stone), false, false, 1).unwrap();
        let wall = make(&mut blocks, &mut inventory, CAPACITY, &wall).unwrap();
        let mut one = cells(["...."; 4], stone);
        one[3][0][0] = Some(Piece::new(wall));
        let plain = plan(&blocks, &inventory, CAPACITY, &one, false, false, 1).unwrap().shape;
        assert!(plain.get(0, 3, 0) && !plain.get(0, 0, 0), "the wall on the far side of its quarter");
        one[3][0][0] = Some(Piece { thing: wall, turns: 1 });
        let turned = plan(&blocks, &inventory, CAPACITY, &one, false, false, 1).unwrap().shape;
        assert!(turned.get(0, 0, 0) && !turned.get(3, 0, 0), "a quarter turn counterclockwise puts it at the left");
        let flipped = plan(&blocks, &inventory, CAPACITY, &cells(SLAB, stone), false, true, 1).unwrap().shape;
        assert!(flipped.get(0, 0, 15) && !flipped.get(0, 0, 0), "a slab flipped is a top slab");
    }

    #[test]
    fn a_part_goes_into_a_layer_as_it_was_built_in_either_view() {
        // A standing L: two cells side by side, one stacked four high.
        let (mut blocks, mut inventory, stone, _) = setup();
        let l = cells(["....", "....", "....", "41.."], stone);
        for side in [false, true] {
            let made = plan(&blocks, &inventory, CAPACITY, &l, side, false, 1).unwrap();
            let part = make(&mut blocks, &mut inventory, CAPACITY, &made).unwrap();
            let mut again = cells(["...."; 4], stone);
            again[3][0][0] = Some(Piece::new(part));
            let small = plan(&blocks, &inventory, CAPACITY, &again, side, false, 1);
            let small = small.unwrap_or_else(|e| panic!("side {side}: the tiny L rests where the big one did: {e}"));
            assert_eq!(small.volume, made.volume / 64);
            // Shrunk into the near left quarter, standing the way the big one stood.
            let expected = if side {
                // Its column out from the wall at the bottom left, its foot along the wall.
                [(0, 12, 0, true), (0, 15, 0, true), (1, 15, 0, true), (1, 12, 0, false), (0, 15, 1, false)]
            } else {
                [(0, 0, 3, true), (0, 0, 0, true), (1, 0, 0, true), (1, 0, 1, false), (0, 1, 0, false)]
            };
            for (x, y, z, filled) in expected {
                assert_eq!(small.shape.get(x, y, z), filled, "side {side}: ({x}, {y}, {z})");
            }
        }
    }
}
