//! What fills a boxel: a short list of occupants, each a block and how it's turned, in the
//! order they were placed. Chunks store a small ID per boxel; the grid keeps one shared table
//! from IDs to occupant lists, so identical lists are stored once.

use std::collections::HashMap;

use api::{BlockBox, Orientation, Vec3};

use crate::{BlockId, BlockRegistry};

/// The most occupants one boxel can hold: room for a design of [`MAX_MATERIALS`] pieces, and
/// for each piece to be cut in two by hiding (see [`crate::signal`]).
pub const MAX_OCCUPANTS: usize = 2 * MAX_MATERIALS;

/// The most materials one crafted design holds, each placed as an occupant of its own.
pub const MAX_MATERIALS: usize = 8;

/// One thing in a boxel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Occupant {
    pub block: BlockId,
    pub orientation: Orientation,
}

impl Occupant {
    /// A block, unturned.
    pub fn new(block: BlockId) -> Self {
        Self { block, orientation: Orientation::IDENTITY }
    }

    pub fn turned(block: BlockId, orientation: Orientation) -> Self {
        Self { block, orientation }
    }
}

/// Points at one occupant list in a grid's [`CellTable`]. Runtime only, like [`BlockId`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CellId(u32);

impl CellId {
    /// Nothing in the boxel.
    pub const EMPTY: CellId = CellId(0);

    pub fn is_empty(self) -> bool {
        self == Self::EMPTY
    }
}

/// An occupant list stored in place: how many, then the occupants in order, padded with empties.
type List = (u8, [Occupant; MAX_OCCUPANTS]);

const NO_OCCUPANT: Occupant = Occupant { block: BlockId::EMPTY, orientation: Orientation::IDENTITY };

/// Every occupant list in use, each stored once. Order matters: it's the order the occupants
/// were placed, oldest first, and where two faces coincide the oldest one shows, so the same
/// occupants in another order are another list. Entries are never removed while the game runs.
#[derive(Debug)]
pub struct CellTable {
    lists: Vec<List>,
    ids: HashMap<List, CellId>,
    /// The last single occupant interned, since filling a region interns the same one over
    /// and over.
    last_single: Option<(Occupant, CellId)>,
}

impl Default for CellTable {
    fn default() -> Self {
        let empty = (0, [NO_OCCUPANT; MAX_OCCUPANTS]);
        Self { lists: vec![empty], ids: HashMap::from([(empty, CellId::EMPTY)]), last_single: None }
    }
}

impl CellTable {
    pub fn get(&self, id: CellId) -> &[Occupant] {
        let (len, items) = &self.lists[id.0 as usize];
        &items[..*len as usize]
    }

    /// The ID for these occupants, in this order, adding the list if it's new. Empty occupants
    /// are dropped.
    /// Panics with more than [`MAX_OCCUPANTS`], which the sharing rule never allows.
    pub fn intern(&mut self, occupants: &[Occupant]) -> CellId {
        let mut list: List = (0, [NO_OCCUPANT; MAX_OCCUPANTS]);
        for &o in occupants.iter().filter(|o| !o.block.is_empty()) {
            assert!((list.0 as usize) < MAX_OCCUPANTS, "more than {MAX_OCCUPANTS} occupants in one boxel");
            list.1[list.0 as usize] = o;
            list.0 += 1;
        }
        if list.0 == 1
            && let Some((single, id)) = self.last_single
            && single == list.1[0]
        {
            return id;
        }
        let id = match self.ids.get(&list) {
            Some(&id) => id,
            None => {
                let id = CellId(u32::try_from(self.lists.len()).expect("fewer than 4 billion distinct boxel contents"));
                self.lists.push(list);
                self.ids.insert(list, id);
                id
            }
        };
        if list.0 == 1 {
            self.last_single = Some((list.1[0], id));
        }
        id
    }

    /// How many distinct lists are stored, the empty one included.
    pub fn len(&self) -> usize {
        self.lists.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }
}

/// Why a block can't join a boxel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareError {
    /// The boxel already holds the most it can.
    Crowded,
    /// A full block shares its boxel with nothing.
    Full,
    /// Another block already fills exactly the same space.
    Duplicate,
    /// Not a registered block.
    Unknown,
}

impl std::fmt::Display for ShareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ShareError::Crowded => "the boxel is full of other blocks",
            ShareError::Full => "a full block shares its boxel with nothing",
            ShareError::Duplicate => "another block already fills exactly the same space",
            ShareError::Unknown => "not a registered block",
        })
    }
}

/// Whether `new` may join a boxel that holds `existing`. Loose for now: up to
/// [`MAX_OCCUPANTS`], no full block shares, and no two blocks fill exactly the same space,
/// whatever they're made of (a slab turned about its own axis still fills the same space).
/// Overlap is allowed, so a fence post can poke through a slab. Hidden pieces count toward
/// the limit but take up no space.
pub fn can_share(existing: &[Occupant], new: Occupant, blocks: &BlockRegistry) -> Result<(), ShareError> {
    let def = blocks.get(new.block).ok_or(ShareError::Unknown)?;
    if existing.is_empty() {
        return Ok(());
    }
    if existing.len() >= MAX_OCCUPANTS {
        return Err(ShareError::Crowded);
    }
    let existing: Vec<Occupant> = existing.iter().copied().filter(|o| !blocks.is_hidden(o.block)).collect();
    if existing.is_empty() || blocks.is_hidden(new.block) {
        return Ok(());
    }
    if def.is_full() || existing.iter().any(|o| blocks.get(o.block).is_some_and(|d| d.is_full())) {
        return Err(ShareError::Full);
    }
    let space = |o: Occupant| {
        let shape = blocks.get(o.block).map_or(&[][..], |d| &d.shape[..]);
        let mut boxes: Vec<_> = shape.iter().map(|&b| o.orientation.apply_box(b)).collect();
        boxes.sort_by_key(|b| (b.min, b.max));
        boxes
    };
    let filled = space(new);
    if existing.iter().any(|&o| space(o) == filled) {
        return Err(ShareError::Duplicate);
    }
    Ok(())
}

/// A box in boxel space: 0 to 1 on each axis from the boxel's minimum corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellBox {
    pub min: Vec3,
    pub max: Vec3,
}

impl CellBox {
    pub const FULL: CellBox = CellBox { min: Vec3::ZERO, max: Vec3::ONE };

    pub fn from_block_box(b: BlockBox) -> Self {
        let scale = 1.0 / BlockBox::SIZE as f32;
        let v = |p: [u8; 3]| Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32) * scale;
        Self { min: v(b.min), max: v(b.max) }
    }

    pub fn translate(self, by: Vec3) -> Self {
        Self { min: self.min + by, max: self.max + by }
    }

    /// The smallest box holding all of `boxes`, or `None` if there are none.
    pub fn bounds(boxes: &[CellBox]) -> Option<CellBox> {
        boxes.iter().copied().reduce(|a, b| CellBox { min: a.min.min(b.min), max: a.max.max(b.max) })
    }
}

/// The boxes an occupant fills, turned, in boxel space.
pub fn occupant_boxes(occupant: Occupant, blocks: &BlockRegistry) -> impl Iterator<Item = CellBox> + '_ {
    let shape = blocks.get(occupant.block).map_or(&[][..], |def| &def.shape[..]);
    shape.iter().map(move |&b| CellBox::from_block_box(occupant.orientation.apply_box(b)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::BlockDef;

    fn setup() -> (BlockRegistry, BlockId, BlockId, BlockId) {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(BlockDef::new("test:stone", true, [0; 3])).unwrap();
        let slab = blocks.register(BlockDef::new("test:slab", true, [0; 3]).with_shape(vec![SLAB])).unwrap();
        let post = blocks.register(BlockDef::new("test:post", true, [0; 3]).with_shape(vec![POST])).unwrap();
        (blocks, stone, slab, post)
    }

    const SLAB: BlockBox = BlockBox::new([0, 0, 0], [16, 16, 8]);
    const POST: BlockBox = BlockBox::new([6, 6, 0], [10, 10, 16]);

    fn flipped() -> Orientation {
        Orientation::quarter_turns(0, 2)
    }

    #[test]
    fn the_same_occupants_in_the_same_order_share_one_id() {
        let (_, stone, slab, post) = setup();
        let mut table = CellTable::default();
        assert_eq!(table.intern(&[]), CellId::EMPTY);
        let a = table.intern(&[Occupant::new(slab), Occupant::new(post)]);
        assert_eq!(table.intern(&[Occupant::new(slab), Occupant::new(post)]), a);
        let b = table.intern(&[Occupant::new(post), Occupant::new(slab)]);
        assert_ne!(a, b, "placement order is kept");
        assert_eq!(table.get(a), [Occupant::new(slab), Occupant::new(post)]);
        assert_eq!(table.get(b), [Occupant::new(post), Occupant::new(slab)]);
        let c = table.intern(&[Occupant::new(stone)]);
        assert_ne!(a, c);
        assert_eq!(table.intern(&[Occupant::new(stone)]), c);
        assert_eq!(table.len(), 4);
        // Empty occupants are dropped, so an empty list is the empty cell.
        assert_eq!(table.intern(&[Occupant::new(BlockId::EMPTY)]), CellId::EMPTY);
    }

    #[test]
    fn partial_blocks_share_and_full_ones_do_not() {
        let (blocks, stone, slab, post) = setup();
        let ok = |existing: &[Occupant], new| can_share(existing, new, &blocks);
        assert_eq!(ok(&[], Occupant::new(stone)), Ok(()));
        assert_eq!(ok(&[Occupant::new(slab)], Occupant::new(post)), Ok(()), "a post pokes through a slab");
        assert_eq!(ok(&[Occupant::new(slab)], Occupant::turned(slab, flipped())), Ok(()), "a double slab");
        assert_eq!(ok(&[Occupant::new(slab)], Occupant::new(slab)), Err(ShareError::Duplicate));
        let spun = Occupant::turned(slab, Orientation::quarter_turns(2, 1));
        assert_eq!(ok(&[Occupant::new(slab)], spun), Err(ShareError::Duplicate), "the same space, turned");
        let mut blocks = blocks;
        let other = blocks.register(BlockDef::new("test:other_slab", true, [0; 3]).with_shape(vec![SLAB])).unwrap();
        let ok = |existing: &[Occupant], new| can_share(existing, new, &blocks);
        assert_eq!(ok(&[Occupant::new(slab)], Occupant::new(other)), Err(ShareError::Duplicate), "another material");
        assert_eq!(ok(&[Occupant::new(slab)], Occupant::turned(other, flipped())), Ok(()));
        assert_eq!(ok(&[Occupant::new(slab)], Occupant::new(stone)), Err(ShareError::Full));
        assert_eq!(ok(&[Occupant::new(stone)], Occupant::new(slab)), Err(ShareError::Full));

        // Slabs against four different sides of the boxel.
        let sides = [Orientation::IDENTITY, flipped(), Orientation::z_along(0), Orientation::z_along(1)];
        let four: Vec<_> = sides.iter().map(|&o| Occupant::turned(slab, o)).collect();
        assert_eq!(ok(&four, Occupant::new(post)), Ok(()), "room for more than four");
        let crowded = vec![Occupant::new(slab); MAX_OCCUPANTS];
        assert_eq!(ok(&crowded, Occupant::new(post)), Err(ShareError::Crowded));
    }

    #[test]
    fn occupant_boxes_are_turned() {
        let (blocks, stone, slab, _) = setup();
        let boxes: Vec<_> = occupant_boxes(Occupant::turned(slab, flipped()), &blocks).collect();
        assert_eq!(boxes, [CellBox { min: Vec3::new(0.0, 0.0, 0.5), max: Vec3::ONE }]);
        assert_eq!(occupant_boxes(Occupant::new(stone), &blocks).collect::<Vec<_>>(), [CellBox::FULL]);
        assert_eq!(occupant_boxes(Occupant::new(BlockId::EMPTY), &blocks).count(), 0);
    }
}
