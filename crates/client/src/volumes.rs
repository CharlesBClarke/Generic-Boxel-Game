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
    /// Each volume's sheen as strength and sharpness, by volume; zero strength for matte ones.
    pub sheens: Vec<[f32; 2]>,
    /// Which volume each block uses and whether its look turns with it, by block ID; `None`
    /// for blocks without a look.
    of_block: Vec<Option<(u32, bool)>>,
    /// Each look's volume and whether it turns, by name, for blocks registered later.
    by_look: std::collections::HashMap<String, (u32, bool)>,
}

impl Volumes {
    /// The plain white volume.
    pub const WHITE: u32 = 0;

    pub fn build(looks: &LookRegistry, blocks: &BlockRegistry) -> Self {
        // A look's texels run X, then Y, then Z, so stacking along Z is just one after another.
        let mut rgba = vec![255; LookDef::VOLUME * 4];
        let mut by_look = std::collections::HashMap::new();
        let mut sheens = vec![[0.0, 1.0]];
        for (i, look) in looks.iter().enumerate() {
            rgba.extend_from_slice(&look.rgba);
            by_look.insert(look.name.clone(), (i as u32 + 1, look.turns));
            sheens.push(look.sheen.map_or([0.0, 1.0], |s| [s.strength, s.sharpness]));
        }
        let mut volumes = Volumes { count: looks.iter().count() as u32 + 1, rgba, sheens, of_block: vec![None], by_look };
        volumes.sync(blocks);
        volumes
    }

    /// Catches up with blocks registered since: crafting registers shaped blocks while the
    /// game runs. Looks are fixed at startup, so this only looks new blocks' up.
    pub fn sync(&mut self, blocks: &BlockRegistry) {
        for (id, def) in blocks.iter().skip(self.of_block.len() - 1) {
            debug_assert_eq!(id.index(), self.of_block.len());
            // The simulation has checked every name exists; a missing one would draw white.
            let volume = def.look.as_ref().map(|l| self.by_look.get(l).copied().unwrap_or((Self::WHITE, false)));
            self.of_block.push(volume);
        }
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
        LookDef { name: name.into(), turns, rgba: color.repeat(LookDef::VOLUME), sheen: None }
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
        assert_eq!(volumes.sheens, [[0.0, 1.0]; 3], "matte looks, white too");

        // A shiny look's sheen sits at its volume.
        let mut shiny = LookRegistry::default();
        shiny.register(look("test:matte", [1, 1, 1, 255], false)).unwrap();
        shiny.register(LookDef { sheen: Some(api::Sheen { strength: 0.5, sharpness: 40.0 }), ..look("test:ice", [9, 9, 9, 255], false) }).unwrap();
        assert_eq!(Volumes::build(&shiny, &blocks).sheens, [[0.0, 1.0], [0.0, 1.0], [0.5, 40.0]]);

        // A block registered later, as crafting does, finds its look once synced.
        let mut volumes = volumes;
        let later = blocks.register(BlockDef::new("test:later", true, [0; 3]).with_look("test:red")).unwrap();
        assert_eq!(volumes.of(later), None);
        volumes.sync(&blocks);
        assert_eq!((volumes.of(later), volumes.of(grass)), (Some(1), Some(2)));

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
