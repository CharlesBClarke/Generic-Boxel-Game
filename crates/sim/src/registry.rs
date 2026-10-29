//! The block and texture registries: names to runtime IDs and definitions.

use std::collections::HashMap;

use api::block::is_valid_name;
use api::{ApiError, BlockDef, TextureDef};

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

/// Textures by name, in registration order. The simulation never draws them; it keeps them
/// so clients, and later the network, can get every mod's textures from one place.
#[derive(Default)]
pub struct TextureRegistry {
    defs: Vec<TextureDef>,
    by_name: HashMap<String, usize>,
}

impl TextureRegistry {
    pub fn register(&mut self, def: TextureDef) -> Result<(), ApiError> {
        if !is_valid_name(&def.name) {
            return Err(ApiError::InvalidName(def.name));
        }
        if self.by_name.contains_key(&def.name) {
            return Err(ApiError::DuplicateName(def.name));
        }
        let expected = def.width as usize * def.height as usize * 4;
        if def.width == 0 || def.height == 0 || def.rgba.len() != expected {
            return Err(ApiError::InvalidDef(format!(
                "texture {:?} is {}×{} but has {} bytes, expected {expected}",
                def.name,
                def.width,
                def.height,
                def.rgba.len()
            )));
        }
        self.by_name.insert(def.name.clone(), self.defs.len());
        self.defs.push(def);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&TextureDef> {
        self.by_name.get(name).map(|&i| &self.defs[i])
    }

    /// Every texture, in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &TextureDef> {
        self.defs.iter()
    }

    /// Checks that every texture a block names is registered.
    pub fn check_blocks(&self, blocks: &BlockRegistry) -> Result<(), ApiError> {
        for (_, def) in blocks.iter() {
            for name in def.textures.iter().flat_map(|t| t.names()) {
                if self.get(name).is_none() {
                    return Err(ApiError::UnknownTexture(name.to_owned()));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::FaceTextures;

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

    fn texture(name: &str) -> TextureDef {
        TextureDef { name: name.into(), width: 2, height: 1, rgba: vec![255; 8] }
    }

    #[test]
    fn textures_look_up_by_name_and_check_their_size() {
        let mut reg = TextureRegistry::default();
        reg.register(texture("test:stone")).unwrap();
        assert_eq!(reg.get("test:stone").unwrap().width, 2);
        assert!(reg.get("test:dirt").is_none());
        assert_eq!(reg.register(texture("test:stone")), Err(ApiError::DuplicateName("test:stone".into())));
        assert_eq!(reg.register(texture("stone")), Err(ApiError::InvalidName("stone".into())));
        let short = TextureDef { rgba: vec![0; 7], ..texture("test:short") };
        assert!(matches!(reg.register(short), Err(ApiError::InvalidDef(_))));
        let empty = TextureDef { width: 0, rgba: Vec::new(), ..texture("test:empty") };
        assert!(matches!(reg.register(empty), Err(ApiError::InvalidDef(_))));
    }

    #[test]
    fn blocks_must_name_registered_textures() {
        let mut textures = TextureRegistry::default();
        textures.register(texture("test:stone")).unwrap();
        let mut blocks = BlockRegistry::default();
        blocks.register(stone().with_textures(FaceTextures::all("test:stone"))).unwrap();
        blocks.register(BlockDef::new("test:plain", true, [1, 2, 3])).unwrap();
        assert_eq!(textures.check_blocks(&blocks), Ok(()));

        let grass = FaceTextures { top: "test:grass".into(), ..FaceTextures::all("test:stone") };
        blocks.register(BlockDef::new("test:grass", true, [0, 0, 0]).with_textures(grass)).unwrap();
        assert_eq!(textures.check_blocks(&blocks), Err(ApiError::UnknownTexture("test:grass".into())));
    }
}
