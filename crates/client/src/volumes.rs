//! Stacks every registered look into one 3D image, so all chunks draw with one material.
//! Plain texels; turning it into a Bevy image happens elsewhere.

use api::LookDef;
use sim::{BlockId, BlockRegistry, LookRegistry};

/// Every look's volume, stacked along Z, one after another. The first is plain white, for
/// blocks drawn in a flat vertex color.
pub struct Volumes {
    /// How many volumes are stacked.
    pub count: u32,
    /// Four bytes a texel, sRGB: X fastest, then Y, then Z through every volume in turn.
    pub rgba: Vec<u8>,
    /// Which volume each block uses and whether its look turns with it, by block ID; `None`
    /// for blocks without a look.
    of_block: Vec<Option<(u32, bool)>>,
}

impl Volumes {
    /// The plain white volume.
    pub const WHITE: u32 = 0;

    pub fn build(looks: &LookRegistry, blocks: &BlockRegistry) -> Self {
        // A look's texels run X, then Y, then Z, so stacking along Z is just one after another.
        let mut rgba = vec![255; LookDef::VOLUME * 4];
        let mut index = std::collections::HashMap::new();
        for (i, look) in looks.iter().enumerate() {
            rgba.extend_from_slice(&look.rgba);
            index.insert(look.name.as_str(), (i as u32 + 1, look.turns));
        }
        let mut of_block = vec![None; blocks.len() + 1];
        for (id, def) in blocks.iter() {
            // The simulation has checked every name exists; a missing one would draw white.
            of_block[id.index()] =
                def.look.as_ref().map(|l| index.get(l.as_str()).copied().unwrap_or((Self::WHITE, false)));
        }
        Volumes { count: looks.iter().count() as u32 + 1, rgba, of_block }
    }

    /// The volume a block is drawn from, or `None` if it has no look.
    pub fn of(&self, id: BlockId) -> Option<u32> {
        self.of_block.get(id.index()).copied().flatten().map(|(volume, _)| volume)
    }

    /// Whether a block's look turns with it.
    pub fn turns(&self, id: BlockId) -> bool {
        self.of_block.get(id.index()).copied().flatten().is_some_and(|(_, turns)| turns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::BlockDef;

    fn look(name: &str, color: [u8; 4], turns: bool) -> LookDef {
        LookDef { name: name.into(), turns, rgba: color.repeat(LookDef::VOLUME) }
    }

    #[test]
    fn blocks_find_their_volumes() {
        let mut looks = LookRegistry::default();
        looks.register(look("test:red", [255, 0, 0, 255], false)).unwrap();
        looks.register(look("test:green", [0, 255, 0, 255], true)).unwrap();
        let mut blocks = BlockRegistry::default();
        let grass = blocks.register(BlockDef::new("test:grass", true, [0; 3]).with_look("test:green")).unwrap();
        let slab = blocks.register(BlockDef::new("test:slab", true, [0; 3]).with_look("test:green")).unwrap();
        let plain = blocks.register(BlockDef::new("test:plain", true, [9; 3])).unwrap();
        let volumes = Volumes::build(&looks, &blocks);
        assert_eq!(volumes.count, 3);
        assert_eq!(volumes.rgba.len(), 3 * LookDef::VOLUME * 4);
        assert_eq!((volumes.of(grass), volumes.of(slab)), (Some(2), Some(2)), "blocks share their look's volume");
        assert_eq!((volumes.of(plain), volumes.of(BlockId::EMPTY)), (None, None));
        assert!(volumes.turns(grass) && volumes.turns(slab), "every shape follows its look's rule");
        assert!(!volumes.turns(plain));

        // Texel (x, y, z) of volume v is at z + v * SIZE in the stack.
        let texel = |v: usize, x: usize, y: usize, z: usize| {
            let at = 4 * LookDef::index(x, y, z + v * LookDef::SIZE);
            &volumes.rgba[at..at + 4]
        };
        for (x, y, z) in [(0, 0, 0), (15, 15, 15), (3, 9, 12)] {
            assert_eq!(texel(0, x, y, z), [255; 4], "white first");
            assert_eq!(texel(1, x, y, z), [255, 0, 0, 255]);
            assert_eq!(texel(2, x, y, z), [0, 255, 0, 255]);
        }
    }
}
