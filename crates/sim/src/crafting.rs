//! Crafting at a table: what the player put in the grid, checked against what they carry, and
//! turned into parts. The shape rule itself is the API's ([`api::craft_shape`]); this adds
//! materials and costs. Every cell holds one material, or a part made of it, and making a part
//! costs its volume of that material.

use api::crafting::GRID;
use api::{BoxelPos, Cell, CraftError, ShapeGrid, TableFace, craft_shape};

use crate::{BlockRegistry, Inventory, MaterialId};

/// A request to craft at a table, as the player makes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Craft {
    /// Where the table is.
    pub table: BoxelPos,
    /// The face used.
    pub face: TableFace,
    /// What's in each cell: a material or a part, by row from the top and column from the left
    /// as seen at that face.
    pub cells: [[Option<MaterialId>; GRID]; GRID],
    /// The most parts to make; as many as can be paid for, up to this.
    pub count: u32,
}

/// What a craft would make.
#[derive(Clone, Debug, PartialEq)]
pub struct CraftPlan {
    /// The material it's made of, and pays with.
    pub material: MaterialId,
    /// The part's shape, in the table's frame.
    pub shape: ShapeGrid,
    /// Each part's volume, in [`crate::FULL_VOLUME`] units, which is also its cost.
    pub cost: u32,
    /// How many it would make: as many as asked for, or fewer if the material runs out.
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
    /// Not enough material carried for one part.
    Short,
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
            CraftRefusal::Short => write!(f, "not enough material"),
        }
    }
}

/// What `cells` make on `face`, and how many `inventory` can pay for, up to `count`.
pub fn plan(
    blocks: &BlockRegistry,
    inventory: &Inventory,
    face: TableFace,
    cells: &[[Option<MaterialId>; GRID]; GRID],
    count: u32,
) -> Result<CraftPlan, CraftRefusal> {
    let mut material = None;
    let mut pattern: [[Cell; GRID]; GRID] = Default::default();
    for (row, line) in cells.iter().enumerate() {
        for (col, &cell) in line.iter().enumerate() {
            let Some(id) = cell.filter(|id| id.index() < blocks.material_count()) else {
                continue;
            };
            let base = blocks.part_of(id).unwrap_or(id);
            if material.is_some_and(|m| m != base) {
                return Err(CraftRefusal::Mixed);
            }
            material = Some(base);
            pattern[row][col] = match blocks.part_of(id) {
                Some(_) => Cell::Shape(Box::new(blocks.shape(blocks.material_block(id)))),
                None => Cell::Solid,
            };
        }
    }
    let shape = craft_shape(face, &pattern).map_err(CraftRefusal::Pattern)?;
    let material = material.expect("a pattern that made something holds a material");
    if shape.is_full() {
        return Err(CraftRefusal::FullBlock);
    }
    if !blocks.get(blocks.material_block(material)).is_some_and(|def| def.is_full()) {
        return Err(CraftRefusal::NotShapeable);
    }
    let cost = shape.volume();
    let count = count.min(inventory.amount(material) / cost);
    if count == 0 {
        return Err(CraftRefusal::Short);
    }
    Ok(CraftPlan { material, shape, cost, count })
}

/// Makes what `plan` says: registers the part's block if it's new, takes the material and adds
/// the parts. Returns the part. The volume carried stays the same, so it always fits.
pub fn make(blocks: &mut BlockRegistry, inventory: &mut Inventory, plan: &CraftPlan) -> Result<MaterialId, CraftRefusal> {
    let block = blocks.shaped(plan.material, &plan.shape).map_err(|_| CraftRefusal::NotShapeable)?;
    let part = blocks.drop(block).expect("a shaped block drops itself").material;
    let total = plan.cost * plan.count;
    if !inventory.take(plan.material, total) {
        return Err(CraftRefusal::Short);
    }
    inventory.add(part, total, None);
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
        inventory.add(wood, FULL_VOLUME, None);
        (blocks, inventory, stone, wood)
    }

    /// Cells from text rows, top first: `#` is `m`, `.` empty.
    fn cells(rows: [&str; 4], m: MaterialId) -> [[Option<MaterialId>; 4]; 4] {
        rows.map(|r| {
            let v: Vec<_> = r.chars().map(|c| (c == '#').then_some(m)).collect();
            v.try_into().unwrap()
        })
    }

    const SLAB: [&str; 4] = ["....", "....", "####", "####"];

    #[test]
    fn crafting_pays_the_volume_and_makes_parts() {
        let (mut blocks, mut inventory, stone, _) = setup();
        let plan = plan(&blocks, &inventory, TableFace::Front, &cells(SLAB, stone), 1).unwrap();
        assert_eq!((plan.material, plan.cost, plan.count), (stone, FULL_VOLUME / 2, 1));
        let part = make(&mut blocks, &mut inventory, &plan).unwrap();
        assert_eq!(inventory.amount(stone), 4 * FULL_VOLUME - FULL_VOLUME / 2);
        assert_eq!(inventory.amount(part), FULL_VOLUME / 2, "one slab");
        assert_eq!(inventory.total(), 5 * FULL_VOLUME, "the same volume carried, wood included");
        assert_eq!(blocks.part_of(part), Some(stone));
    }

    #[test]
    fn asking_for_many_makes_what_can_be_paid_for() {
        let (blocks, inventory, stone, wood) = setup();
        let many = plan(&blocks, &inventory, TableFace::Front, &cells(SLAB, stone), 100).unwrap();
        assert_eq!(many.count, 8, "4 stone makes 8 slabs");
        let wood_slabs = plan(&blocks, &inventory, TableFace::Front, &cells(SLAB, wood), 100).unwrap();
        assert_eq!(wood_slabs.count, 2);
        let mut poor = Inventory::default();
        poor.add(wood, FULL_VOLUME / 4, None);
        assert_eq!(plan(&blocks, &poor, TableFace::Front, &cells(SLAB, wood), 1), Err(CraftRefusal::Short));
    }

    #[test]
    fn patterns_hold_one_material() {
        let (blocks, inventory, stone, wood) = setup();
        let mut mixed = cells(SLAB, stone);
        mixed[3][0] = Some(wood);
        assert_eq!(plan(&blocks, &inventory, TableFace::Front, &mixed, 1), Err(CraftRefusal::Mixed));
        let full = cells(["####"; 4], stone);
        assert_eq!(plan(&blocks, &inventory, TableFace::Front, &full, 1), Err(CraftRefusal::FullBlock));
        let empty = cells(["...."; 4], stone);
        assert_eq!(
            plan(&blocks, &inventory, TableFace::Front, &empty, 1),
            Err(CraftRefusal::Pattern(CraftError::Empty))
        );
    }

    #[test]
    fn a_part_in_a_cell_is_a_pattern_and_is_not_used_up() {
        let (mut blocks, mut inventory, stone, wood) = setup();
        let slab = plan(&blocks, &inventory, TableFace::Front, &cells(SLAB, stone), 1).unwrap();
        let part = make(&mut blocks, &mut inventory, &slab).unwrap();
        let mut with_part = cells(["...."; 4], stone);
        with_part[0][0] = Some(part);
        with_part[3][3] = Some(stone);
        let p = plan(&blocks, &inventory, TableFace::Front, &with_part, 1).unwrap();
        // A whole quarter column, plus a quarter column holding a slab shrunk: half of one.
        assert_eq!(p.cost, FULL_VOLUME / 16 + FULL_VOLUME / 32);
        assert_eq!(p.material, stone, "paid in the part's material");
        let made = make(&mut blocks, &mut inventory, &p).unwrap();
        assert_eq!(inventory.amount(part), FULL_VOLUME / 2, "the slab is still there");
        assert_eq!(inventory.amount(made), p.cost);
        // A part of another material in the pattern mixes materials.
        let mut mixed = with_part;
        mixed[3][3] = Some(wood);
        assert_eq!(plan(&blocks, &inventory, TableFace::Front, &mixed, 1), Err(CraftRefusal::Mixed));
        // And the finer part can't go in a cell.
        let mut finer = cells(["...."; 4], stone);
        finer[1][1] = Some(made);
        assert_eq!(
            plan(&blocks, &inventory, TableFace::Front, &finer, 1),
            Err(CraftRefusal::Pattern(CraftError::TooFine { row: 1, col: 1 }))
        );
    }
}
