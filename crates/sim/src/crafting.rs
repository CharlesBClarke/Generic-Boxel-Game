//! Crafting at a table: what the player stacked on it, turned into parts. The shape rule
//! itself is the API's ([`api::craft_shape`]); this adds materials and room. Every layer holds
//! one material, or a part made of it. Crafting is free for now (paying for it is deferred):
//! a part costs nothing to make, but takes its volume of the capacity to carry.

use api::crafting::GRID;
use api::{BoxelPos, CraftError, Layer, Orientation, ShapeGrid, Stacks, craft_shape};

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
    /// The most parts to make; as many as there's room to carry, up to this.
    pub count: u32,
}

/// What a craft would make.
#[derive(Clone, Debug, PartialEq)]
pub struct CraftPlan {
    /// The material it's made of.
    pub material: MaterialId,
    /// The part's shape, in the table's frame.
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
    /// The cells hold more than one material.
    Mixed,
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
            CraftRefusal::Mixed => write!(f, "one material per pattern"),
            CraftRefusal::Pattern(e) => write!(f, "{e}"),
            CraftRefusal::FullBlock => write!(f, "that's a full block, nothing to shape"),
            CraftRefusal::NotShapeable => write!(f, "that material can't be shaped"),
            CraftRefusal::Full => write!(f, "no room to carry it"),
        }
    }
}

/// What `cells` make, flipped if `flip`, and how many fit in `inventory` under `capacity`, up
/// to `count`.
pub fn plan(
    blocks: &BlockRegistry,
    inventory: &Inventory,
    capacity: Option<u32>,
    cells: &Cells,
    flip: bool,
    count: u32,
) -> Result<CraftPlan, CraftRefusal> {
    let mut material = None;
    let mut stacks: Stacks = Default::default();
    for (row, line) in cells.iter().enumerate() {
        for (col, stack) in line.iter().enumerate() {
            for (layer, &piece) in stack.iter().enumerate() {
                let Some(piece) = piece.filter(|p| p.thing.index() < blocks.material_count()) else {
                    continue;
                };
                let id = piece.thing;
                let base = blocks.part_of(id).unwrap_or(id);
                if material.is_some_and(|m| m != base) {
                    return Err(CraftRefusal::Mixed);
                }
                material = Some(base);
                stacks[row][col][layer] = Some(match blocks.part_of(id) {
                    Some(_) => {
                        let turn = Orientation::quarter_turns(2, piece.turns as i32);
                        Layer::Shape(Box::new(blocks.shape(blocks.material_block(id)).turned(turn)))
                    }
                    None => Layer::Solid,
                });
            }
        }
    }
    let shape = craft_shape(&stacks, flip).map_err(CraftRefusal::Pattern)?;
    let material = material.expect("a pattern that made something holds a material");
    if shape.is_full() {
        return Err(CraftRefusal::FullBlock);
    }
    if !blocks.get(blocks.material_block(material)).is_some_and(|def| def.is_full()) {
        return Err(CraftRefusal::NotShapeable);
    }
    let volume = shape.volume();
    let count = count.min(inventory.room(capacity) / volume);
    if count == 0 {
        return Err(CraftRefusal::Full);
    }
    Ok(CraftPlan { material, shape, volume, count })
}

/// Makes what `plan` says: registers the part's block if it's new and adds the parts. Returns
/// the part.
pub fn make(
    blocks: &mut BlockRegistry,
    inventory: &mut Inventory,
    capacity: Option<u32>,
    plan: &CraftPlan,
) -> Result<MaterialId, CraftRefusal> {
    let block = blocks.shaped(plan.material, &plan.shape).map_err(|_| CraftRefusal::NotShapeable)?;
    let part = blocks.drop(block).expect("a shaped block drops itself").material;
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
        let plan = plan(&blocks, &inventory, CAPACITY, &cells(SLAB, stone), false, 1).unwrap();
        assert_eq!((plan.material, plan.volume, plan.count), (stone, FULL_VOLUME / 2, 1));
        let part = make(&mut blocks, &mut inventory, CAPACITY, &plan).unwrap();
        assert_eq!(inventory.amount(stone), 4 * FULL_VOLUME, "no stone taken");
        assert_eq!(inventory.amount(part), FULL_VOLUME / 2, "one slab");
        assert_eq!(blocks.part_of(part), Some(stone));
        // None of a material is needed to shape it.
        let wood_slab = super::plan(&blocks, &inventory, CAPACITY, &cells(SLAB, wood), false, 1).unwrap();
        assert_eq!(wood_slab.material, wood);
    }

    #[test]
    fn asking_for_many_makes_what_fits() {
        let (blocks, inventory, stone, _) = setup();
        let many = plan(&blocks, &inventory, CAPACITY, &cells(SLAB, stone), false, 100).unwrap();
        assert_eq!(many.count, 8, "4 boxels of room left holds 8 slabs");
        let unlimited = plan(&blocks, &inventory, None, &cells(SLAB, stone), false, 100).unwrap();
        assert_eq!(unlimited.count, 100);
        let full = Some(4 * FULL_VOLUME);
        assert_eq!(plan(&blocks, &inventory, full, &cells(SLAB, stone), false, 1), Err(CraftRefusal::Full));
    }

    #[test]
    fn patterns_hold_one_material() {
        let (blocks, inventory, stone, wood) = setup();
        let mut mixed = cells(SLAB, stone);
        mixed[3][0][0] = Some(Piece::new(wood));
        assert_eq!(plan(&blocks, &inventory, CAPACITY, &mixed, false, 1), Err(CraftRefusal::Mixed));
        let full = cells(["4444"; 4], stone);
        assert_eq!(plan(&blocks, &inventory, CAPACITY, &full, false, 1), Err(CraftRefusal::FullBlock));
        let empty = cells(["...."; 4], stone);
        assert_eq!(
            plan(&blocks, &inventory, CAPACITY, &empty, false, 1),
            Err(CraftRefusal::Pattern(CraftError::Empty))
        );
    }

    #[test]
    fn a_part_in_a_layer_is_a_pattern_and_is_not_used_up() {
        let (mut blocks, mut inventory, stone, wood) = setup();
        let slab = plan(&blocks, &inventory, CAPACITY, &cells(SLAB, stone), false, 1).unwrap();
        let part = make(&mut blocks, &mut inventory, CAPACITY, &slab).unwrap();
        let mut with_part = cells(["...."; 4], stone);
        with_part[0][0][0] = Some(Piece::new(part));
        with_part[3][3][0] = Some(Piece::new(stone));
        let p = plan(&blocks, &inventory, CAPACITY, &with_part, false, 1).unwrap();
        // A whole quarter cube, plus a quarter cube holding a slab shrunk: half of one.
        assert_eq!(p.volume, FULL_VOLUME / 64 + FULL_VOLUME / 128);
        assert_eq!(p.material, stone, "made of the part's material");
        let made = make(&mut blocks, &mut inventory, CAPACITY, &p).unwrap();
        assert_eq!(inventory.amount(part), FULL_VOLUME / 2, "the slab is still there");
        assert_eq!(inventory.amount(made), p.volume);
        // A part of another material on the table mixes materials.
        let mut mixed = with_part;
        mixed[3][3][0] = Some(Piece::new(wood));
        assert_eq!(plan(&blocks, &inventory, CAPACITY, &mixed, false, 1), Err(CraftRefusal::Mixed));
        // And the finer part can't go in a layer.
        let mut finer = cells(["...."; 4], stone);
        finer[1][1][0] = Some(Piece::new(made));
        assert_eq!(
            plan(&blocks, &inventory, CAPACITY, &finer, false, 1),
            Err(CraftRefusal::Pattern(CraftError::TooFine { row: 1, col: 1, layer: 0 }))
        );
    }

    #[test]
    fn a_turned_part_goes_in_turned_and_flipping_turns_the_result() {
        let (mut blocks, mut inventory, stone, _) = setup();
        // A wall along the far side.
        let wall = plan(&blocks, &inventory, CAPACITY, &cells(["4444", "....", "....", "...."], stone), false, 1).unwrap();
        let wall = make(&mut blocks, &mut inventory, CAPACITY, &wall).unwrap();
        let mut one = cells(["...."; 4], stone);
        one[3][0][0] = Some(Piece::new(wall));
        let plain = plan(&blocks, &inventory, CAPACITY, &one, false, 1).unwrap().shape;
        assert!(plain.get(0, 3, 0) && !plain.get(0, 0, 0), "the wall on the far side of its quarter");
        one[3][0][0] = Some(Piece { thing: wall, turns: 1 });
        let turned = plan(&blocks, &inventory, CAPACITY, &one, false, 1).unwrap().shape;
        assert!(turned.get(0, 0, 0) && !turned.get(3, 0, 0), "a quarter turn counterclockwise puts it at the left");
        let flipped = plan(&blocks, &inventory, CAPACITY, &cells(SLAB, stone), true, 1).unwrap().shape;
        assert!(flipped.get(0, 0, 15) && !flipped.get(0, 0, 0), "a slab flipped is a top slab");
    }
}
