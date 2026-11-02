//! The block and texture registries: names to runtime IDs and definitions.

use std::collections::HashMap;

use api::block::is_valid_name;
use api::{ApiError, BlockBox, BlockDef, LookDef};

/// Volume units in a full boxel: one per sixteenth cubed. Amounts of material count in these,
/// so a slab's worth is a whole number.
pub const FULL_VOLUME: u32 = (BlockBox::SIZE as u32).pow(3);

/// A runtime material ID, like [`BlockId`]: assigned in the order blocks first name each
/// material, never saved. Names are the identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MaterialId(u16);

impl MaterialId {
    /// A small dense index, for tables by material: 0, 1, 2… in the order they were first named.
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: usize) -> Self {
        MaterialId(index as u16)
    }
}

/// What breaking a block drops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drop {
    pub material: MaterialId,
    /// In [`FULL_VOLUME`] units.
    pub amount: u32,
}

/// A runtime block ID. Assigned at startup in registration order and never saved or
/// sent anywhere; names are the identity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(u16);

impl BlockId {
    /// No block. Not a registered definition, so the engine defines no blocks itself.
    pub const EMPTY: BlockId = BlockId(0);

    pub fn is_empty(self) -> bool {
        self == Self::EMPTY
    }

    /// A small dense index, for tables by block: 0 for empty, then 1, 2… in registration order.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Default)]
pub struct BlockRegistry {
    defs: Vec<BlockDef>,
    by_name: HashMap<String, BlockId>,
    /// What each block drops, by its index less one.
    drops: Vec<Option<Drop>>,
    /// Each material's name and the block it's drawn as.
    materials: Vec<(String, BlockId)>,
}

impl BlockRegistry {
    pub fn register(&mut self, def: BlockDef) -> Result<BlockId, ApiError> {
        if !is_valid_name(&def.name) {
            return Err(ApiError::InvalidName(def.name));
        }
        if self.by_name.contains_key(&def.name) {
            return Err(ApiError::DuplicateName(def.name));
        }
        def.validate().map_err(ApiError::InvalidDef)?;
        let index = u16::try_from(self.defs.len() + 1)
            .map_err(|_| ApiError::InvalidDef("too many block types".into()))?;
        let id = BlockId(index);
        let drop = match &def.material {
            Some(name) => Some(Drop { material: self.add_material(name, id, def.is_full())?, amount: volume(&def.shape) }),
            None => None,
        };
        self.by_name.insert(def.name.clone(), id);
        self.defs.push(def);
        self.drops.push(drop);
        Ok(id)
    }

    /// The ID for material `name`, adding it if it's new. A material is drawn as the first full
    /// block made of it, or the first block if none is full.
    fn add_material(&mut self, name: &str, block: BlockId, full: bool) -> Result<MaterialId, ApiError> {
        if let Some(i) = self.materials.iter().position(|(n, _)| n == name) {
            if full && !self.get(self.materials[i].1).is_some_and(BlockDef::is_full) {
                self.materials[i].1 = block;
            }
            return Ok(MaterialId(i as u16));
        }
        let index = u16::try_from(self.materials.len()).map_err(|_| ApiError::InvalidDef("too many materials".into()))?;
        self.materials.push((name.to_owned(), block));
        Ok(MaterialId(index))
    }

    /// What breaking `id` drops, if anything.
    pub fn drop(&self, id: BlockId) -> Option<Drop> {
        (id.0 as usize).checked_sub(1).and_then(|i| self.drops.get(i).copied().flatten())
    }

    /// The material's namespaced name.
    pub fn material_name(&self, id: MaterialId) -> &str {
        &self.materials[id.index()].0
    }

    /// The block a loose item of the material is drawn as.
    pub fn material_block(&self, id: MaterialId) -> BlockId {
        self.materials[id.index()].1
    }

    pub fn material(&self, name: &str) -> Option<MaterialId> {
        self.materials.iter().position(|(n, _)| n == name).map(|i| MaterialId(i as u16))
    }

    /// The material with dense index `index`, if there is one.
    pub fn material_by_index(&self, index: usize) -> Option<MaterialId> {
        (index < self.materials.len()).then_some(MaterialId(index as u16))
    }

    /// How many materials blocks name.
    pub fn material_count(&self) -> usize {
        self.materials.len()
    }

    pub fn id(&self, name: &str) -> Option<BlockId> {
        self.by_name.get(name).copied()
    }

    /// The definition for `id`, or `None` for [`BlockId::EMPTY`].
    pub fn get(&self, id: BlockId) -> Option<&BlockDef> {
        (id.0 as usize).checked_sub(1).and_then(|i| self.defs.get(i))
    }

    /// Every definition with its ID, in registration order.
    pub fn iter(&self) -> impl Iterator<Item = (BlockId, &BlockDef)> {
        self.defs.iter().enumerate().map(|(i, def)| (BlockId(i as u16 + 1), def))
    }

    pub fn is_solid(&self, id: BlockId) -> bool {
        self.get(id).is_some_and(|def| def.solid)
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

/// The space a shape fills, in [`FULL_VOLUME`] units. Where boxes overlap, the space counts once.
pub fn volume(shape: &[BlockBox]) -> u32 {
    let n = BlockBox::SIZE as usize;
    let mut filled = vec![false; n * n * n];
    for b in shape {
        let [x0, y0, z0] = b.min.map(usize::from);
        let [x1, y1, z1] = b.max.map(usize::from);
        for z in z0..z1 {
            for y in y0..y1 {
                filled[(z * n + y) * n + x0..(z * n + y) * n + x1].fill(true);
            }
        }
    }
    filled.iter().filter(|&&f| f).count() as u32
}

/// Looks by name, in registration order. The simulation never draws them; it keeps them so
/// clients, and later the network, can get every mod's looks from one place.
#[derive(Default)]
pub struct LookRegistry {
    defs: Vec<LookDef>,
    by_name: HashMap<String, usize>,
}

impl LookRegistry {
    pub fn register(&mut self, def: LookDef) -> Result<(), ApiError> {
        if !is_valid_name(&def.name) {
            return Err(ApiError::InvalidName(def.name));
        }
        if self.by_name.contains_key(&def.name) {
            return Err(ApiError::DuplicateName(def.name));
        }
        let expected = LookDef::VOLUME * 4;
        if def.rgba.len() != expected {
            let size = LookDef::SIZE;
            return Err(ApiError::InvalidDef(format!(
                "look {:?} has {} bytes, expected {expected} for {size}×{size}×{size} texels",
                def.name,
                def.rgba.len()
            )));
        }
        self.by_name.insert(def.name.clone(), self.defs.len());
        self.defs.push(def);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&LookDef> {
        self.by_name.get(name).map(|&i| &self.defs[i])
    }

    /// Every look, in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &LookDef> {
        self.defs.iter()
    }

    /// Checks that every look a block names is registered.
    pub fn check_blocks(&self, blocks: &BlockRegistry) -> Result<(), ApiError> {
        for (_, def) in blocks.iter() {
            if let Some(name) = &def.look
                && self.get(name).is_none()
            {
                return Err(ApiError::UnknownLook(name.clone()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone() -> BlockDef {
        BlockDef::new("test:stone", true, [128, 128, 128])
    }

    #[test]
    fn looks_up_by_name() {
        let mut reg = BlockRegistry::default();
        let stone_id = reg.register(stone()).unwrap();
        let glass_id = reg.register(BlockDef::new("test:glass", false, [200, 220, 255])).unwrap();
        assert_ne!(stone_id, glass_id);
        assert!(!stone_id.is_empty());
        assert_eq!(reg.id("test:stone"), Some(stone_id));
        assert_eq!(reg.get(stone_id).unwrap().name, "test:stone");
        assert!(reg.is_solid(stone_id));
        assert!(!reg.is_solid(glass_id));
        assert_eq!(reg.id("test:missing"), None);
        let listed: Vec<_> = reg.iter().map(|(id, def)| (id, def.name.as_str())).collect();
        assert_eq!(listed, [(stone_id, "test:stone"), (glass_id, "test:glass")]);
    }

    #[test]
    fn empty_is_not_a_definition() {
        let mut reg = BlockRegistry::default();
        reg.register(stone()).unwrap();
        assert!(reg.get(BlockId::EMPTY).is_none());
        assert!(!reg.is_solid(BlockId::EMPTY));
    }

    #[test]
    fn rejects_duplicates_and_bad_names() {
        let mut reg = BlockRegistry::default();
        reg.register(stone()).unwrap();
        assert_eq!(reg.register(stone()), Err(ApiError::DuplicateName("test:stone".into())));
        assert_eq!(
            reg.register(BlockDef::new("stone", true, [0, 0, 0])),
            Err(ApiError::InvalidName("stone".into()))
        );
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn rejects_bad_shapes() {
        let mut reg = BlockRegistry::default();
        let none = BlockDef::new("test:none", true, [0; 3]).with_shape(Vec::new());
        assert!(matches!(reg.register(none), Err(ApiError::InvalidDef(_))));
        let flat = BlockDef::new("test:flat", true, [0; 3]).with_shape(vec![api::BlockBox::new([0, 0, 4], [16, 16, 4])]);
        assert!(matches!(reg.register(flat), Err(ApiError::InvalidDef(_))));
        let big = BlockDef::new("test:big", true, [0; 3]).with_shape(vec![api::BlockBox::new([0, 0, 0], [16, 16, 17])]);
        assert!(matches!(reg.register(big), Err(ApiError::InvalidDef(_))));
        assert!(reg.is_empty());
    }

    #[test]
    fn volume_counts_overlaps_once() {
        let slab = BlockBox::new([0, 0, 0], [16, 16, 8]);
        let step = BlockBox::new([0, 8, 8], [16, 16, 16]);
        assert_eq!(volume(&[BlockBox::FULL]), FULL_VOLUME);
        assert_eq!(volume(&[slab]), FULL_VOLUME / 2);
        assert_eq!(volume(&[slab, step]), FULL_VOLUME * 3 / 4);
        assert_eq!(volume(&[slab, slab]), FULL_VOLUME / 2);
        assert_eq!(volume(&[BlockBox::FULL, step]), FULL_VOLUME);
    }

    #[test]
    fn blocks_drop_their_volume_of_their_material() {
        let mut reg = BlockRegistry::default();
        let slab_shape = vec![BlockBox::new([0, 0, 0], [16, 16, 8])];
        let slab = reg.register(stone().with_shape(slab_shape.clone()).with_material("test:stone")).unwrap();
        let block = reg.register(BlockDef::new("test:block", true, [0; 3]).with_material("test:stone")).unwrap();
        let grass = reg.register(BlockDef::new("test:grass", true, [0; 3]).with_material("test:dirt")).unwrap();
        let plain = reg.register(BlockDef::new("test:plain", true, [0; 3])).unwrap();

        let stone_id = reg.material("test:stone").unwrap();
        assert_eq!(reg.drop(slab), Some(Drop { material: stone_id, amount: FULL_VOLUME / 2 }));
        assert_eq!(reg.drop(block), Some(Drop { material: stone_id, amount: FULL_VOLUME }));
        assert_eq!(reg.drop(grass).map(|d| reg.material_name(d.material)), Some("test:dirt"));
        assert_eq!(reg.drop(plain), None, "no material drops nothing");
        assert_eq!(reg.drop(BlockId::EMPTY), None);
        assert_eq!(reg.material_count(), 2);
        // Drawn as the first full block, though the slab named it first.
        assert_eq!(reg.material_block(stone_id), block);
        assert_eq!(reg.material_block(reg.material("test:dirt").unwrap()), grass);

        let bad = BlockDef::new("test:bad", true, [0; 3]).with_material("stone");
        assert!(matches!(reg.register(bad), Err(ApiError::InvalidDef(_))));
    }

    fn look(name: &str) -> LookDef {
        LookDef { name: name.into(), turns: false, rgba: vec![255; LookDef::VOLUME * 4] }
    }

    #[test]
    fn looks_look_up_by_name_and_check_their_size() {
        let mut reg = LookRegistry::default();
        reg.register(look("test:stone")).unwrap();
        assert!(reg.get("test:stone").is_some());
        assert!(reg.get("test:dirt").is_none());
        assert_eq!(reg.register(look("test:stone")), Err(ApiError::DuplicateName("test:stone".into())));
        assert_eq!(reg.register(look("stone")), Err(ApiError::InvalidName("stone".into())));
        let short = LookDef { rgba: vec![0; 7], ..look("test:short") };
        assert!(matches!(reg.register(short), Err(ApiError::InvalidDef(_))));
    }

    #[test]
    fn blocks_must_name_registered_looks() {
        let mut looks = LookRegistry::default();
        looks.register(look("test:stone")).unwrap();
        let mut blocks = BlockRegistry::default();
        blocks.register(stone().with_look("test:stone")).unwrap();
        blocks.register(BlockDef::new("test:plain", true, [1, 2, 3])).unwrap();
        assert_eq!(looks.check_blocks(&blocks), Ok(()));

        blocks.register(BlockDef::new("test:grass", true, [0, 0, 0]).with_look("test:grass")).unwrap();
        assert_eq!(looks.check_blocks(&blocks), Err(ApiError::UnknownLook("test:grass".into())));
    }
}
