//! The block and texture registries: names to runtime IDs and definitions.

use std::collections::HashMap;

use api::block::is_valid_name;
use api::{ApiError, BlockBox, BlockDef, LookDef, ShapeGrid, Turning};

use crate::Occupant;

/// Volume units in a full boxel: one per sixteenth cubed. Amounts of material count in these,
/// so a slab's worth is a whole number.
pub const FULL_VOLUME: u32 = (BlockBox::SIZE as u32).pow(3);

/// A runtime ID for something carried, like [`BlockId`]: assigned in the order blocks first
/// name each, never saved. Names are the identity.
///
/// Most are materials, carried by volume in any amount. A crafted shape is carried too, as a
/// part: its own kind, named after its block, carried in whole parts and made of a material
/// (see [`BlockRegistry::shaped`]). A shape of several materials is a bundle: a part whose
/// pieces are placed together, one block each (see [`BlockRegistry::bundle`]). All count toward
/// the capacity by volume, so inventories, loose items and costs treat them alike.
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

#[derive(Clone, Default)]
pub struct BlockRegistry {
    defs: Vec<BlockDef>,
    by_name: HashMap<String, BlockId>,
    /// What each block drops, by its index less one.
    drops: Vec<Option<Drop>>,
    /// Each material's and part's name and the block it's drawn as, and what a part is made of.
    materials: Vec<Material>,
    /// Each bundle piece's bundle.
    bundle_of: HashMap<BlockId, MaterialId>,
    /// Each derived kind: a cut piece's kept part, or a hidden piece (see [`Derived`]).
    derived: HashMap<BlockId, Derived>,
}

/// A piece made from part of another occupant: what's left shown of a piece cut by hiding, or
/// the hidden part of one. Its block fills `shape` unturned, so it's placed unturned.
#[derive(Clone, Debug, PartialEq)]
pub struct Derived {
    /// The occupant it was cut from. Never itself derived: cutting a derived piece again
    /// derives from the same occupant, so all the parts of one piece find each other.
    pub from: Occupant,
    /// The sixteenths it fills in its boxel.
    pub shape: ShapeGrid,
    /// Hidden pieces aren't solid, aren't drawn, and are aimed at only by someone holding a
    /// phase material.
    pub hidden: bool,
}

#[derive(Clone)]
struct Material {
    name: String,
    block: BlockId,
    /// For a part, the material it's made of and its volume, carried only in whole parts. A
    /// bundle's is its first piece's material.
    part: Option<(MaterialId, u32)>,
    /// For a bundle, its pieces' blocks, first piece first; empty otherwise.
    pieces: Vec<BlockId>,
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
        if let Some(i) = self.materials.iter().position(|m| m.name == name) {
            if full && !self.get(self.materials[i].block).is_some_and(BlockDef::is_full) {
                self.materials[i].block = block;
            }
            return Ok(MaterialId(i as u16));
        }
        let index = u16::try_from(self.materials.len()).map_err(|_| ApiError::InvalidDef("too many materials".into()))?;
        self.materials.push(Material { name: name.to_owned(), block, part: None, pieces: Vec::new() });
        Ok(MaterialId(index))
    }

    /// The block of `material` in `shape`, registering it the first time it's needed: crafting
    /// makes these, so every shape of every material can exist without being listed anywhere.
    ///
    /// It copies the material's full block (its look, solidity and friction), turns freely, and
    /// is named from the material and a hash of the shape, like `base:wood_s0123456789abcdef`.
    /// Breaking it gives back one part: the block is its own kind of carried thing, made of
    /// `material`. A full shape is the material's full block itself.
    pub fn shaped(&mut self, material: MaterialId, shape: &ShapeGrid) -> Result<BlockId, ApiError> {
        let entry = self.materials.get(material.index()).ok_or_else(|| ApiError::InvalidDef("no such material".into()))?;
        if entry.part.is_some() {
            return Err(ApiError::InvalidDef(format!("{} is a part, not a material", entry.name)));
        }
        let full = self.get(entry.block).filter(|def| def.is_full()).cloned().ok_or_else(|| {
            ApiError::InvalidDef(format!("material {} has no full block to shape", entry.name))
        })?;
        if shape.is_empty() {
            return Err(ApiError::InvalidDef("an empty shape".into()));
        }
        if shape.is_full() {
            return Ok(entry.block);
        }
        let name = shaped_name(&entry.name, shape);
        if let Some(id) = self.id(&name) {
            return match self.shape(id) == *shape {
                true => Ok(id),
                false => Err(ApiError::DuplicateName(name)),
            };
        }
        let def = BlockDef {
            name: name.clone(),
            shape: shape.to_boxes(),
            turning: Turning::Any,
            placeable: true,
            material: Some(name),
            ..full
        };
        let id = self.register(def)?;
        let part = self.drop(id).expect("a shaped block drops itself").material;
        self.materials[part.index()].part = Some((material, shape.volume()));
        Ok(id)
    }

    /// The block of `material` in `shape`, if it's registered already: the one
    /// [`BlockRegistry::shaped`] gives, without registering anything.
    pub fn find_shaped(&self, material: MaterialId, shape: &ShapeGrid) -> Option<BlockId> {
        let entry = self.materials.get(material.index())?;
        if shape.is_full() {
            return self.get(entry.block).filter(|def| def.is_full()).map(|_| entry.block);
        }
        self.id(&shaped_name(&entry.name, shape)).filter(|&id| self.shape(id) == *shape)
    }

    /// A bundle of `pieces`, each a material and the sixteenths it fills, registering it the
    /// first time: one carried kind whose pieces are placed together, each as a block of its
    /// own. Each piece block copies the material's part in that shape (see
    /// [`BlockRegistry::shaped`]) and breaks into it, so pieces come off one at a time as
    /// ordinary parts. Named from the first material's namespace and a hash of the pieces, like
    /// `base:bundle_0123456789abcdef`, its pieces with `_0`, `_1`… after that.
    pub fn bundle(&mut self, pieces: &[(MaterialId, ShapeGrid)]) -> Result<MaterialId, ApiError> {
        if !(2..=crate::MAX_OCCUPANTS).contains(&pieces.len()) {
            return Err(ApiError::InvalidDef(format!("a bundle has 2 to {} pieces", crate::MAX_OCCUPANTS)));
        }
        let mut hash = Fnv::default();
        for (material, shape) in pieces {
            let entry = self.materials.get(material.index()).ok_or_else(|| ApiError::InvalidDef("no such material".into()))?;
            hash.add(entry.name.as_bytes());
            hash.add(&shape.hash64().to_le_bytes());
        }
        let first = &self.materials[pieces[0].0.index()].name;
        let namespace = first.split_once(':').map_or("bundle", |(ns, _)| ns);
        let name = format!("{namespace}:bundle_{:016x}", hash.0);
        if let Some(id) = self.material(&name) {
            return Ok(id);
        }
        let parts = pieces.iter().map(|(m, shape)| self.shaped(*m, shape)).collect::<Result<Vec<_>, _>>()?;
        let mut blocks = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            let def = BlockDef { name: format!("{name}_{i}"), placeable: true, ..self.get(*part).expect("just made").clone() };
            blocks.push(self.register(def)?);
        }
        let volume = pieces.iter().map(|(_, shape)| shape.volume()).sum();
        let id = MaterialId(
            u16::try_from(self.materials.len()).map_err(|_| ApiError::InvalidDef("too many materials".into()))?,
        );
        let part = Some((pieces[0].0, volume));
        self.materials.push(Material { name, block: blocks[0], part, pieces: blocks.clone() });
        for b in blocks {
            self.bundle_of.insert(b, id);
        }
        Ok(id)
    }

    /// A bundle's piece blocks, first piece first; empty for anything else.
    pub fn pieces(&self, id: MaterialId) -> &[BlockId] {
        self.materials.get(id.index()).map_or(&[], |m| &m.pieces)
    }

    /// The bundle a piece block belongs to, if it's one.
    pub fn bundle_of(&self, block: BlockId) -> Option<MaterialId> {
        self.bundle_of.get(&block).copied()
    }

    /// The blocks placed along with `block`: the rest of its bundle when it's a bundle's first
    /// piece, the block a bundle is held and placed as. Empty otherwise.
    pub fn companions(&self, block: BlockId) -> &[BlockId] {
        match self.bundle_of(block).map(|b| self.pieces(b)) {
            Some([first, rest @ ..]) if *first == block => rest,
            _ => &[],
        }
    }

    /// What placing `block` uses up: its whole bundle when it's a bundle's first piece,
    /// otherwise what breaking it gives back.
    pub fn cost(&self, block: BlockId) -> Option<Drop> {
        if !self.companions(block).is_empty() {
            let bundle = self.bundle_of(block)?;
            return Some(Drop { material: bundle, amount: self.unit(bundle) });
        }
        self.drop(block)
    }

    /// The material a block is made of: what it breaks into, or what that part is made of.
    pub fn made_of(&self, block: BlockId) -> Option<MaterialId> {
        let drop = self.drop(block)?;
        Some(self.part_of(drop.material).unwrap_or(drop.material))
    }

    /// The derived kind of `from` filling `shape` in its boxel, registering it the first time:
    /// the shown or hidden part of a piece cut by hiding (see [`Derived`]). It copies `from`'s
    /// block, but breaks into its volume of the material `from` is made of, isn't placed by
    /// players, and isn't solid when hidden. `from` may itself be derived; the new kind derives
    /// from what that one came from.
    pub fn derive(&mut self, from: Occupant, shape: &ShapeGrid, hidden: bool) -> Result<BlockId, ApiError> {
        let from = self.derived(from.block).map_or(from, |d| d.from);
        let def = self.get(from.block).ok_or_else(|| ApiError::InvalidDef("no such block".into()))?;
        if shape.is_empty() {
            return Err(ApiError::InvalidDef("an empty shape".into()));
        }
        let mut hash = Fnv::default();
        hash.add(&from.orientation.apply([1, 2, 3]).map(|c| c as i8 as u8));
        hash.add(&shape.hash64().to_le_bytes());
        let name = format!("{}_{}{:016x}", def.name, if hidden { 'h' } else { 'c' }, hash.0);
        if let Some(id) = self.id(&name) {
            return Ok(id);
        }
        let material = self.made_of(from.block).map(|m| self.material_name(m).to_owned());
        let def = BlockDef {
            name,
            shape: shape.to_boxes(),
            turning: Turning::Any,
            placeable: false,
            solid: def.solid && !hidden,
            material,
            ..def.clone()
        };
        let id = self.register(def)?;
        self.derived.insert(id, Derived { from, shape: shape.clone(), hidden });
        Ok(id)
    }

    /// What a derived kind was cut from, and which part of it it is.
    pub fn derived(&self, block: BlockId) -> Option<&Derived> {
        self.derived.get(&block)
    }

    /// Whether `block` is a hidden piece.
    pub fn is_hidden(&self, block: BlockId) -> bool {
        self.derived(block).is_some_and(|d| d.hidden)
    }

    /// The occupant a piece was cut from (itself if it wasn't), and the sixteenths the piece
    /// fills in its boxel.
    pub fn origin(&self, occupant: Occupant) -> (Occupant, ShapeGrid) {
        match self.derived(occupant.block) {
            Some(d) => (d.from, d.shape.clone()),
            None => (occupant, self.shape(occupant.block).turned(occupant.orientation)),
        }
    }

    /// The sixteenths a block fills, unturned.
    pub fn shape(&self, id: BlockId) -> ShapeGrid {
        self.get(id).map_or(ShapeGrid::EMPTY, |def| ShapeGrid::from_boxes(&def.shape))
    }

    /// For a part, the material it's made of; `None` for a material.
    pub fn part_of(&self, id: MaterialId) -> Option<MaterialId> {
        self.materials.get(id.index()).and_then(|m| m.part).map(|(of, _)| of)
    }

    /// The smallest amount that can be carried or moved, in [`FULL_VOLUME`] units: one part's
    /// volume for a part, 1 for a material.
    pub fn unit(&self, id: MaterialId) -> u32 {
        self.materials.get(id.index()).and_then(|m| m.part).map_or(1, |(_, volume)| volume.max(1))
    }

    /// What breaking `id` drops, if anything.
    pub fn drop(&self, id: BlockId) -> Option<Drop> {
        (id.0 as usize).checked_sub(1).and_then(|i| self.drops.get(i).copied().flatten())
    }

    /// The material's namespaced name.
    pub fn material_name(&self, id: MaterialId) -> &str {
        &self.materials[id.index()].name
    }

    /// The block a loose item of the material is drawn as.
    pub fn material_block(&self, id: MaterialId) -> BlockId {
        self.materials[id.index()].block
    }

    pub fn material(&self, name: &str) -> Option<MaterialId> {
        self.materials.iter().position(|m| m.name == name).map(|i| MaterialId(i as u16))
    }

    /// The material with dense index `index`, if there is one.
    pub fn material_by_index(&self, index: usize) -> Option<MaterialId> {
        (index < self.materials.len()).then_some(MaterialId(index as u16))
    }

    /// How many materials and parts blocks name.
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

/// 64-bit FNV-1a, the same on every machine, for naming.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn add(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// A shaped block's name: its material's and a hash of the shape.
fn shaped_name(material: &str, shape: &ShapeGrid) -> String {
    format!("{material}_s{:016x}", shape.hash64())
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
#[derive(Clone, Default)]
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

    fn slab() -> ShapeGrid {
        ShapeGrid::from_boxes(&[BlockBox::new([0, 0, 0], [16, 16, 8])])
    }

    #[test]
    fn shaped_blocks_are_registered_once_per_shape_and_material() {
        let mut reg = BlockRegistry::default();
        reg.register(stone().with_look("test:stone").with_friction(0.5).with_material("test:stone")).unwrap();
        reg.register(BlockDef::new("test:wood", true, [0; 3]).with_material("test:wood")).unwrap();
        let (stone, wood) = (reg.material("test:stone").unwrap(), reg.material("test:wood").unwrap());

        assert_eq!(reg.find_shaped(stone, &slab()), None);
        let slab_id = reg.shaped(stone, &slab()).unwrap();
        assert_eq!(reg.shaped(stone, &slab()), Ok(slab_id), "one shape, one block");
        assert_eq!(reg.find_shaped(stone, &slab()), Some(slab_id));
        let def = reg.get(slab_id).unwrap().clone();
        assert!(def.name.starts_with("test:stone_s") && def.name.len() == "test:stone_s".len() + 16);
        assert_eq!((def.look.as_deref(), def.friction, def.turning), (Some("test:stone"), 0.5, Turning::Any));
        assert_eq!(reg.shape(slab_id), slab());
        let wood_slab = reg.shaped(wood, &slab()).unwrap();
        assert_ne!(wood_slab, slab_id, "another material, another block");
        let turned = reg.shaped(stone, &slab().turned(api::Orientation::quarter_turns(0, 2))).unwrap();
        assert_ne!(turned, slab_id, "a shape turned in the table's frame is another part");

        // Breaking one gives back one part, made of stone, carried whole.
        let part = reg.drop(slab_id).unwrap();
        assert_eq!(part.amount, FULL_VOLUME / 2);
        assert_eq!(reg.part_of(part.material), Some(stone));
        assert_eq!(reg.unit(part.material), FULL_VOLUME / 2);
        assert_eq!((reg.part_of(stone), reg.unit(stone)), (None, 1));
        assert_eq!(reg.material_block(part.material), slab_id);

        assert_eq!(reg.shaped(stone, &ShapeGrid::FULL), reg.id("test:stone").ok_or(ApiError::UnknownBlock(String::new())));
        assert!(reg.shaped(stone, &ShapeGrid::EMPTY).is_err());
        assert!(reg.shaped(part.material, &slab()).is_err(), "parts aren't shaped again");
    }

    #[test]
    fn only_materials_with_a_full_block_take_shapes() {
        let mut reg = BlockRegistry::default();
        let only = vec![BlockBox::new([0, 0, 0], [16, 16, 8])];
        reg.register(BlockDef::new("test:moss", true, [0; 3]).with_shape(only).with_material("test:moss")).unwrap();
        let moss = reg.material("test:moss").unwrap();
        assert!(matches!(reg.shaped(moss, &slab()), Err(ApiError::InvalidDef(_))));
    }

    fn look(name: &str) -> LookDef {
        LookDef { name: name.into(), turns: false, rgba: vec![255; LookDef::VOLUME * 4], sheen: None }
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
