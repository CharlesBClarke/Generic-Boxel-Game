//! The block registry: names to runtime IDs.

use std::collections::HashMap;

use api::block::is_valid_name;
use api::{ApiError, BlockDef};

/// A runtime block ID. Assigned at startup in registration order and never saved or
/// sent anywhere; names are the identity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BlockId(u16);

impl BlockId {
    /// No block. Not a registered definition, so the engine defines no blocks itself.
    pub const EMPTY: BlockId = BlockId(0);

    pub fn is_empty(self) -> bool {
        self == Self::EMPTY
    }
}

#[derive(Default)]
pub struct BlockRegistry {
    defs: Vec<BlockDef>,
    by_name: HashMap<String, BlockId>,
}

impl BlockRegistry {
    pub fn register(&mut self, def: BlockDef) -> Result<BlockId, ApiError> {
        if !is_valid_name(&def.name) {
            return Err(ApiError::InvalidName(def.name));
        }
        if self.by_name.contains_key(&def.name) {
            return Err(ApiError::DuplicateName(def.name));
        }
        let index = u16::try_from(self.defs.len() + 1)
            .map_err(|_| ApiError::InvalidDef("too many block types".into()))?;
        let id = BlockId(index);
        self.by_name.insert(def.name.clone(), id);
        self.defs.push(def);
        Ok(id)
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
}
