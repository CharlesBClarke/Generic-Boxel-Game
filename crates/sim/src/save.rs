//! Saved worlds as plain data, and the bytes they're written as. Blocks are named, never
//! numbered: a save loads into any run of the game whose mods register the same names, in any
//! order. Blocks the game makes while it runs (crafted shapes, bundles, pieces cut by hiding)
//! are named from hashes, so a save keeps how to make each again instead. No file access here:
//! the server decides where saves go.

use api::{CHUNK_VOLUME, ChunkPos, I64Vec3, Orientation, ShapeGrid, Vec3, WorldPos};

use crate::Layer;

/// Everything a save keeps. Creatures aren't kept: the herd starts fresh on load.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SaveData {
    /// Ticks run when it was saved.
    pub tick: u64,
    /// The values the terrain was generated with: the `world` section and each generator's
    /// (`terrain <name>`), as (section, name, value).
    pub terrain: Vec<(String, String, f32)>,
    /// Blocks made while the game ran, in the order they were made, so each one's ingredients
    /// come before it.
    pub made: Vec<Made>,
    /// Chunks that differ from what loading makes and aren't in the chunk store: every chunk
    /// outside the generated terrain. Saves before version 4 also hold the generated chunks
    /// that were edited, which now go to the store (see [`crate::store`]).
    pub chunks: Vec<SavedChunk>,
    pub player: Option<SavedPlayer>,
    /// What the player carries: each material or gadget by name, with how much.
    pub inventory: Vec<(String, u32)>,
    pub items: Vec<SavedItem>,
    /// The player's blueprints, in the order they were made. Added in version 2.
    pub blueprints: Vec<SavedBlueprint>,
    /// The time of day, in hours. Added in version 3; older saves have none.
    pub hours: Option<f64>,
}

/// A blueprint by name, naming the materials and blueprints it uses.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedBlueprint {
    pub name: String,
    pub design: SavedDesign,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SavedDesign {
    /// Every layer of every stack, row by row from the far side, then column, then layer from
    /// the bottom: what it holds and its quarter turns.
    Recipe { layers: Vec<Option<(SavedThing, u8)>>, side: bool, flip: bool },
    Variant { base: String, changes: Vec<SavedChange> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum SavedThing {
    /// A material, part or bundle, by its material name.
    Material(String),
    Blueprint(String),
}

/// A variant's change (see [`crate::Change`]), naming materials.
#[derive(Clone, Debug, PartialEq)]
pub enum SavedChange {
    Turn { axis: u8, quarters: u8 },
    Mirror { axis: u8 },
    Shift([i8; 3]),
    Swap { from: String, to: String },
}

/// How to make again a block the game made while it ran.
#[derive(Clone, Debug, PartialEq)]
pub enum Made {
    /// A material crafted into a shape (see [`crate::BlockRegistry::shaped`]).
    Shaped { material: String, shape: ShapeGrid },
    /// Several materials crafted into one design (see [`crate::BlockRegistry::bundle`]).
    Bundle { pieces: Vec<(String, ShapeGrid)> },
    /// Part of an occupant, cut by hiding (see [`crate::BlockRegistry::derive`]).
    Derived { from: String, orientation: Orientation, shape: ShapeGrid, hidden: bool },
}

/// A chunk by name: its occupant lists name their blocks.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedChunk {
    pub pos: ChunkPos,
    /// Every occupant list the chunk uses, each a block's name and how it's turned.
    pub lists: Vec<Vec<(String, Orientation)>>,
    /// Each boxel's list, as an index into `lists`.
    pub cells: Layer<u16>,
    /// The boxels with records (storage and memory, see [`crate::mend::Record`]). Added in chunk
    /// version 2 and save version 5.
    pub records: Vec<SavedRecord>,
}

/// A boxel's record by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedRecord {
    /// The boxel's index in its chunk.
    pub index: u16,
    /// What its storage holds: each material by name, with how much.
    pub stored: Vec<(String, u32)>,
    /// What menders remember of it: each piece's block by name, how it's turned, and its texels.
    pub memory: Vec<(String, Orientation, ShapeGrid)>,
}

/// The first bytes of every stored chunk.
const CHUNK_MAGIC: &[u8; 4] = b"BXCH";
/// Goes up whenever a stored chunk's layout changes. Version 2 added records.
const CHUNK_VERSION: u8 = 2;
/// The most a stored chunk may unpack to: far more than any real one (a chunk of every boxel
/// different), so a damaged one can't fill memory.
const CHUNK_UNPACKED_MAX: u64 = 16 << 20;

impl SavedChunk {
    /// The chunk on its own, as stored apart from the world's file (see [`crate::store`]): its
    /// lists naming their blocks, and its cells in runs, packed with zstd. No light: it's worked
    /// out again when the chunk loads.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.ivec(self.pos.0);
        w.len(self.lists.len());
        for list in &self.lists {
            w.len(list.len());
            for (name, orientation) in list {
                w.str(name);
                w.orientation(*orientation);
            }
        }
        w.cells(&self.cells);
        w.records(&self.records);
        let mut out = Vec::with_capacity(w.0.len() / 4 + 16);
        out.extend_from_slice(CHUNK_MAGIC);
        out.push(CHUNK_VERSION);
        ruzstd::encoding::compress(&w.0[..], &mut out, ruzstd::encoding::CompressionLevel::Fastest);
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<SavedChunk, String> {
        use std::io::Read;
        let Some(packed) = bytes.strip_prefix(CHUNK_MAGIC) else { return Err("not a stored chunk".into()) };
        let Some((&version, packed)) = packed.split_first() else { return Err("a stored chunk with no version".into()) };
        if !(1..=CHUNK_VERSION).contains(&version) {
            return Err(format!("a stored chunk from version {version}; this game reads version {CHUNK_VERSION}"));
        }
        let decoder = ruzstd::decoding::StreamingDecoder::new(packed).map_err(|e| format!("a stored chunk that won't unpack: {e}"))?;
        let mut unpacked = Vec::new();
        decoder
            .take(CHUNK_UNPACKED_MAX + 1)
            .read_to_end(&mut unpacked)
            .map_err(|e| format!("a stored chunk that won't unpack: {e}"))?;
        if unpacked.len() as u64 > CHUNK_UNPACKED_MAX {
            return Err("a stored chunk too big to be real".into());
        }
        let mut r = Reader { bytes: &unpacked, at: 0 };
        let pos = ChunkPos(r.ivec()?);
        let mut lists = Vec::new();
        for _ in 0..r.len()? {
            let mut list = Vec::new();
            for _ in 0..r.len()? {
                list.push((r.str()?, r.orientation()?));
            }
            lists.push(list);
        }
        let cells = r.cells(lists.len(), pos)?;
        let records = if version >= 2 { r.records()? } else { Vec::new() };
        if r.at != unpacked.len() {
            return Err(format!("extra bytes after stored chunk {pos:?}"));
        }
        Ok(SavedChunk { pos, lists, cells, records })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SavedPlayer {
    pub pos: WorldPos,
    pub yaw: f32,
    pub pitch: f32,
    pub flying: bool,
    pub health: f32,
}

/// A loose item lying in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedItem {
    /// What picking it up gives, by name, and how much.
    pub material: String,
    pub amount: u32,
    /// The block it's drawn as, by name.
    pub block: String,
    pub pos: WorldPos,
    pub velocity: Vec3,
}

/// The first bytes of every save.
const MAGIC: &[u8; 8] = b"BOXELSAV";
/// Goes up whenever the layout changes. Version 4 kept the layout but moved edited terrain out
/// of the file into the chunk store (see [`crate::store`]), so a version 4 save holds only the
/// chunks outside the terrain; older ones hold edited terrain too. Version 5 added each chunk's
/// records. Version 6 kept the layout again: its world loads terrain by chunk with no height
/// limit, and its chunk store also holds the tops edits moved (see [`crate::tops`]), which a
/// game from before wouldn't know what to make of.
pub const VERSION: u32 = 6;

impl SaveData {
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_bytes_as(VERSION)
    }

    /// The bytes as `version` wrote them, as far as records go (older versions had none), for
    /// testing reading old saves. Fields added later are still written; tests cut them off.
    pub fn to_bytes_as(&self, version: u32) -> Vec<u8> {
        let mut w = Writer::default();
        w.bytes(MAGIC);
        w.u32(version);
        w.u64(self.tick);
        w.len(self.terrain.len());
        for (section, name, value) in &self.terrain {
            w.str(section);
            w.str(name);
            w.f32(*value);
        }
        w.len(self.made.len());
        for made in &self.made {
            match made {
                Made::Shaped { material, shape } => {
                    w.u8(0);
                    w.str(material);
                    w.shape(shape);
                }
                Made::Bundle { pieces } => {
                    w.u8(1);
                    w.len(pieces.len());
                    for (material, shape) in pieces {
                        w.str(material);
                        w.shape(shape);
                    }
                }
                Made::Derived { from, orientation, shape, hidden } => {
                    w.u8(2);
                    w.str(from);
                    w.orientation(*orientation);
                    w.shape(shape);
                    w.u8(*hidden as u8);
                }
            }
        }
        // Chunks name blocks through one table, so each name is written once.
        let mut names: Vec<&str> = Vec::new();
        let mut index = std::collections::HashMap::new();
        for list in self.chunks.iter().flat_map(|c| &c.lists) {
            for (name, _) in list {
                index.entry(name.as_str()).or_insert_with(|| {
                    names.push(name);
                    names.len() - 1
                });
            }
        }
        w.len(names.len());
        for name in &names {
            w.str(name);
        }
        w.len(self.chunks.len());
        for chunk in &self.chunks {
            w.ivec(chunk.pos.0);
            w.len(chunk.lists.len());
            for list in &chunk.lists {
                w.len(list.len());
                for (name, orientation) in list {
                    w.len(index[name.as_str()]);
                    w.orientation(*orientation);
                }
            }
            w.cells(&chunk.cells);
            if version >= 5 {
                w.records(&chunk.records);
            }
        }
        match &self.player {
            None => w.u8(0),
            Some(p) => {
                w.u8(1);
                w.pos(p.pos);
                w.f32(p.yaw);
                w.f32(p.pitch);
                w.u8(p.flying as u8);
                w.f32(p.health);
            }
        }
        w.len(self.inventory.len());
        for (name, amount) in &self.inventory {
            w.str(name);
            w.u32(*amount);
        }
        w.len(self.items.len());
        for item in &self.items {
            w.str(&item.material);
            w.u32(item.amount);
            w.str(&item.block);
            w.pos(item.pos);
            w.vec3(item.velocity);
        }
        w.len(self.blueprints.len());
        for b in &self.blueprints {
            w.str(&b.name);
            match &b.design {
                SavedDesign::Recipe { layers, side, flip } => {
                    w.u8(0);
                    w.u8(*side as u8);
                    w.u8(*flip as u8);
                    w.len(layers.len());
                    for layer in layers {
                        match layer {
                            None => w.u8(0),
                            Some((SavedThing::Material(name), turns)) => {
                                w.u8(1);
                                w.str(name);
                                w.u8(*turns);
                            }
                            Some((SavedThing::Blueprint(name), turns)) => {
                                w.u8(2);
                                w.str(name);
                                w.u8(*turns);
                            }
                        }
                    }
                }
                SavedDesign::Variant { base, changes } => {
                    w.u8(1);
                    w.str(base);
                    w.len(changes.len());
                    for change in changes {
                        match change {
                            SavedChange::Turn { axis, quarters } => {
                                w.u8(0);
                                w.u8(*axis);
                                w.u8(*quarters);
                            }
                            SavedChange::Mirror { axis } => {
                                w.u8(1);
                                w.u8(*axis);
                            }
                            SavedChange::Shift(by) => {
                                w.u8(2);
                                by.iter().for_each(|&b| w.u8(b as u8));
                            }
                            SavedChange::Swap { from, to } => {
                                w.u8(3);
                                w.str(from);
                                w.str(to);
                            }
                        }
                    }
                }
            }
        }
        match self.hours {
            None => w.u8(0),
            Some(hours) => {
                w.u8(1);
                w.bytes(&hours.to_le_bytes());
            }
        }
        w.0
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<SaveData, String> {
        let mut r = Reader { bytes, at: 0 };
        if r.take(MAGIC.len())? != MAGIC {
            return Err("not a boxel save".into());
        }
        let version = r.u32()?;
        if !(1..=VERSION).contains(&version) {
            return Err(format!("a save from version {version}; this game reads up to version {VERSION}"));
        }
        let mut save = SaveData { tick: r.u64()?, ..SaveData::default() };
        for _ in 0..r.len()? {
            save.terrain.push((r.str()?, r.str()?, r.f32()?));
        }
        for _ in 0..r.len()? {
            let made = match r.u8()? {
                0 => Made::Shaped { material: r.str()?, shape: r.shape()? },
                1 => {
                    let n = r.len()?;
                    let pieces = (0..n).map(|_| Ok((r.str()?, r.shape()?))).collect::<Result<_, String>>()?;
                    Made::Bundle { pieces }
                }
                2 => Made::Derived { from: r.str()?, orientation: r.orientation()?, shape: r.shape()?, hidden: r.u8()? != 0 },
                kind => return Err(format!("unknown kind of made block {kind}")),
            };
            save.made.push(made);
        }
        let names: Vec<String> = (0..r.len()?).map(|_| r.str()).collect::<Result<_, _>>()?;
        for _ in 0..r.len()? {
            let pos = ChunkPos(r.ivec()?);
            let mut lists = Vec::new();
            for _ in 0..r.len()? {
                let mut list = Vec::new();
                for _ in 0..r.len()? {
                    let name = names.get(r.len()?).ok_or("a block name out of range")?;
                    list.push((name.clone(), r.orientation()?));
                }
                lists.push(list);
            }
            let cells = r.cells(lists.len(), pos)?;
            let records = if version >= 5 { r.records()? } else { Vec::new() };
            save.chunks.push(SavedChunk { pos, lists, cells, records });
        }
        save.player = match r.u8()? {
            0 => None,
            _ => Some(SavedPlayer { pos: r.pos()?, yaw: r.f32()?, pitch: r.f32()?, flying: r.u8()? != 0, health: r.f32()? }),
        };
        for _ in 0..r.len()? {
            save.inventory.push((r.str()?, r.u32()?));
        }
        for _ in 0..r.len()? {
            save.items.push(SavedItem {
                material: r.str()?,
                amount: r.u32()?,
                block: r.str()?,
                pos: r.pos()?,
                velocity: r.vec3()?,
            });
        }
        if version >= 2 {
            for _ in 0..r.len()? {
                let name = r.str()?;
                let design = match r.u8()? {
                    0 => {
                        let (side, flip) = (r.u8()? != 0, r.u8()? != 0);
                        let mut layers = Vec::new();
                        for _ in 0..r.len()? {
                            layers.push(match r.u8()? {
                                0 => None,
                                1 => Some((SavedThing::Material(r.str()?), r.u8()?)),
                                2 => Some((SavedThing::Blueprint(r.str()?), r.u8()?)),
                                kind => return Err(format!("unknown kind of layer {kind}")),
                            });
                        }
                        SavedDesign::Recipe { layers, side, flip }
                    }
                    1 => {
                        let base = r.str()?;
                        let mut changes = Vec::new();
                        for _ in 0..r.len()? {
                            changes.push(match r.u8()? {
                                0 => SavedChange::Turn { axis: r.u8()?, quarters: r.u8()? },
                                1 => SavedChange::Mirror { axis: r.u8()? },
                                2 => SavedChange::Shift([r.u8()? as i8, r.u8()? as i8, r.u8()? as i8]),
                                3 => SavedChange::Swap { from: r.str()?, to: r.str()? },
                                kind => return Err(format!("unknown kind of change {kind}")),
                            });
                        }
                        SavedDesign::Variant { base, changes }
                    }
                    kind => return Err(format!("unknown kind of blueprint {kind}")),
                };
                save.blueprints.push(SavedBlueprint { name, design });
            }
        }
        if version >= 3 {
            save.hours = match r.u8()? {
                0 => None,
                1 => Some(f64::from_le_bytes(r.array()?)),
                kind => return Err(format!("unknown kind of time {kind}")),
            };
        }
        if r.at != bytes.len() {
            return Err("extra bytes after the save".into());
        }
        Ok(save)
    }
}

/// Little-endian bytes.
#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
    fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.bytes(&v.to_le_bytes());
    }
    fn len(&mut self, n: usize) {
        self.u32(u32::try_from(n).expect("fewer than 4 billion of anything in a save"));
    }
    fn str(&mut self, s: &str) {
        self.len(s.len());
        self.bytes(s.as_bytes());
    }
    fn ivec(&mut self, v: I64Vec3) {
        for c in v.to_array() {
            self.bytes(&c.to_le_bytes());
        }
    }
    fn vec3(&mut self, v: Vec3) {
        for c in v.to_array() {
            self.f32(c);
        }
    }
    fn pos(&mut self, p: WorldPos) {
        self.ivec(p.cell());
        self.vec3(p.offset());
    }
    fn orientation(&mut self, o: Orientation) {
        let i = Orientation::all().iter().position(|&x| x == o).expect("one of the 24");
        self.u8(i as u8);
    }
    fn shape(&mut self, s: &ShapeGrid) {
        for &row in s.rows() {
            self.u16(row);
        }
    }
    fn records(&mut self, records: &[SavedRecord]) {
        self.len(records.len());
        for record in records {
            self.u16(record.index);
            self.len(record.stored.len());
            for (name, amount) in &record.stored {
                self.str(name);
                self.u32(*amount);
            }
            self.len(record.memory.len());
            for (name, orientation, shape) in &record.memory {
                self.str(name);
                self.orientation(*orientation);
                self.shape(shape);
            }
        }
    }
    fn cells(&mut self, cells: &Layer<u16>) {
        match cells {
            Layer::Uniform(i) => {
                self.u8(0);
                self.u16(*i);
            }
            Layer::Dense(values) => {
                // Runs of the same list, since most of a chunk is a few big runs.
                self.u8(1);
                let mut runs: Vec<(u16, u32)> = Vec::new();
                for &v in values.iter() {
                    match runs.last_mut() {
                        Some((last, n)) if *last == v => *n += 1,
                        _ => runs.push((v, 1)),
                    }
                }
                self.len(runs.len());
                for (v, n) in runs {
                    self.u16(v);
                    self.u32(n);
                }
            }
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.bytes.len()).ok_or("the save ends early")?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        Ok(self.take(N)?.try_into().expect("took N"))
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.array()?))
    }
    fn len(&mut self) -> Result<usize, String> {
        Ok(self.u32()? as usize)
    }
    fn str(&mut self) -> Result<String, String> {
        let n = self.len()?;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| "a name that isn't text".into())
    }
    fn ivec(&mut self) -> Result<I64Vec3, String> {
        Ok(I64Vec3::new(i64::from_le_bytes(self.array()?), i64::from_le_bytes(self.array()?), i64::from_le_bytes(self.array()?)))
    }
    fn vec3(&mut self) -> Result<Vec3, String> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    fn pos(&mut self) -> Result<WorldPos, String> {
        let cell = self.ivec()?;
        Ok(WorldPos::new(cell, self.vec3()?))
    }
    fn orientation(&mut self) -> Result<Orientation, String> {
        let i = self.u8()? as usize;
        Orientation::all().get(i).copied().ok_or_else(|| format!("orientation {i} isn't one of the 24"))
    }
    fn shape(&mut self) -> Result<ShapeGrid, String> {
        let mut rows = [0u16; 256];
        for row in &mut rows {
            *row = self.u16()?;
        }
        Ok(ShapeGrid::from_rows(rows))
    }
    fn records(&mut self) -> Result<Vec<SavedRecord>, String> {
        let mut out = Vec::new();
        for _ in 0..self.len()? {
            let index = self.u16()?;
            if index as usize >= CHUNK_VOLUME {
                return Err(format!("a record for boxel {index}, past the chunk's end"));
            }
            let mut record = SavedRecord { index, ..SavedRecord::default() };
            for _ in 0..self.len()? {
                record.stored.push((self.str()?, self.u32()?));
            }
            for _ in 0..self.len()? {
                record.memory.push((self.str()?, self.orientation()?, self.shape()?));
            }
            out.push(record);
        }
        Ok(out)
    }
    /// A chunk's cells, each an index below `lists`.
    fn cells(&mut self, lists: usize, pos: ChunkPos) -> Result<Layer<u16>, String> {
        let in_range = |i: u16| match (i as usize) < lists {
            true => Ok(i),
            false => Err(format!("a cell list out of range in chunk {pos:?}")),
        };
        Ok(match self.u8()? {
            0 => Layer::Uniform(in_range(self.u16()?)?),
            1 => {
                let mut values = Vec::with_capacity(CHUNK_VOLUME);
                for _ in 0..self.len()? {
                    let (v, n) = (in_range(self.u16()?)?, self.u32()? as usize);
                    if values.len() + n > CHUNK_VOLUME {
                        return Err(format!("too many cells in chunk {pos:?}"));
                    }
                    values.extend(std::iter::repeat_n(v, n));
                }
                if values.len() != CHUNK_VOLUME {
                    return Err(format!("too few cells in chunk {pos:?}"));
                }
                Layer::Dense(values.into_boxed_slice())
            }
            kind => return Err(format!("unknown kind of cell layer {kind}")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SaveData {
        let mut shape = ShapeGrid::EMPTY;
        shape.set(1, 2, 3, true);
        shape.set(15, 15, 15, true);
        let mut dense = vec![0u16; CHUNK_VOLUME];
        dense[5] = 1;
        dense[CHUNK_VOLUME - 1] = 1;
        SaveData {
            tick: 12345,
            terrain: vec![("world".into(), "terrain_width".into(), 11.0), ("terrain base:hills".into(), "seed".into(), 120.0)],
            made: vec![
                Made::Shaped { material: "base:wood".into(), shape: shape.clone() },
                Made::Bundle { pieces: vec![("base:wood".into(), shape.clone()), ("base:stone".into(), ShapeGrid::FULL)] },
                Made::Derived { from: "base:wood".into(), orientation: Orientation::quarter_turns(2, 1), shape: shape.clone(), hidden: true },
            ],
            chunks: vec![
                SavedChunk {
                    pos: ChunkPos::new(-3, 4, 0),
                    lists: vec![vec![], vec![("base:stone".into(), Orientation::IDENTITY), ("base:wood".into(), Orientation::z_along(0))]],
                    cells: Layer::Dense(dense.into_boxed_slice()),
                    records: vec![SavedRecord {
                        index: 5,
                        stored: vec![("base:stone".into(), 300)],
                        memory: vec![("base:wood".into(), Orientation::z_along(0), shape.clone())],
                    }],
                },
                SavedChunk { pos: ChunkPos::new(0, 0, 9), lists: vec![vec![("base:dirt".into(), Orientation::IDENTITY)]], cells: Layer::Uniform(0), records: Vec::new() },
            ],
            player: Some(SavedPlayer {
                pos: WorldPos::new(I64Vec3::new(-5_000_000_000, 7, 2), Vec3::new(0.25, 0.5, 0.75)),
                yaw: 1.5,
                pitch: -0.25,
                flying: true,
                health: 80.0,
            }),
            inventory: vec![("base:wood".into(), 4096), ("base:bundle_00ff".into(), 2)],
            items: vec![SavedItem {
                material: "base:stone".into(),
                amount: 100,
                block: "base:stone".into(),
                pos: WorldPos::new(I64Vec3::new(1, 2, 3), Vec3::splat(0.5)),
                velocity: Vec3::new(0.0, 0.0, -3.0),
            }],
            blueprints: vec![
                SavedBlueprint {
                    name: "corner".into(),
                    design: SavedDesign::Recipe {
                        layers: (0..64)
                            .map(|i| match i {
                                0 => Some((SavedThing::Material("base:wood".into()), 0)),
                                5 => Some((SavedThing::Blueprint("post".into()), 3)),
                                _ => None,
                            })
                            .collect(),
                        side: true,
                        flip: false,
                    },
                },
                SavedBlueprint {
                    name: "corner mirrored".into(),
                    design: SavedDesign::Variant {
                        base: "corner".into(),
                        changes: vec![
                            SavedChange::Turn { axis: 2, quarters: 3 },
                            SavedChange::Mirror { axis: 0 },
                            SavedChange::Shift([-4, 0, 6]),
                            SavedChange::Swap { from: "base:phasite".into(), to: "base:antiphasite".into() },
                        ],
                    },
                },
            ],
            hours: Some(17.25),
        }
    }

    fn without_records(mut save: SaveData) -> SaveData {
        for chunk in &mut save.chunks {
            chunk.records.clear();
        }
        save
    }

    #[test]
    fn a_version_4_save_reads_with_no_records() {
        let save = without_records(sample());
        assert_eq!(SaveData::from_bytes(&sample().to_bytes_as(4)), Ok(save));
    }

    #[test]
    fn a_save_reads_back_the_same() {
        let save = sample();
        assert_eq!(SaveData::from_bytes(&save.to_bytes()), Ok(save));
        let empty = SaveData::default();
        assert_eq!(SaveData::from_bytes(&empty.to_bytes()), Ok(empty));
    }

    #[test]
    fn a_version_1_save_reads_with_no_blueprints() {
        let save = SaveData { blueprints: Vec::new(), hours: None, ..without_records(sample()) };
        let mut bytes = save.to_bytes_as(1);
        // Version 1 ended before the blueprints' count (and version 3's time).
        bytes.truncate(bytes.len() - 5);
        assert_eq!(SaveData::from_bytes(&bytes), Ok(save));
    }

    #[test]
    fn a_version_2_save_reads_with_no_time() {
        let save = SaveData { hours: None, ..without_records(sample()) };
        let mut bytes = save.to_bytes_as(2);
        // Version 2 ended before the time.
        bytes.truncate(bytes.len() - 1);
        assert_eq!(SaveData::from_bytes(&bytes), Ok(save));
    }

    #[test]
    fn a_stored_chunk_reads_back_the_same_and_packs_small() {
        for chunk in sample().chunks {
            let bytes = chunk.to_bytes();
            assert_eq!(SavedChunk::from_bytes(&bytes), Ok(chunk));
        }
        // A chunk of runs, as terrain is: ground, then air.
        let cells: Vec<u16> = (0..CHUNK_VOLUME).map(|i| (i < CHUNK_VOLUME / 3) as u16).collect();
        let lists = vec![vec![], vec![("base:stone".into(), Orientation::IDENTITY)]];
        let chunk = SavedChunk { pos: ChunkPos::new(1, 2, 3), lists, cells: Layer::Dense(cells.into_boxed_slice()), records: Vec::new() };
        let bytes = chunk.to_bytes();
        assert!(bytes.len() < 100, "{} bytes", bytes.len());
        assert_eq!(SavedChunk::from_bytes(&bytes), Ok(chunk));
    }

    #[test]
    fn a_damaged_stored_chunk_is_refused() {
        let bytes = sample().chunks[0].to_bytes();
        assert!(SavedChunk::from_bytes(b"BXCH").is_err());
        assert!(SavedChunk::from_bytes(b"nope and more").is_err());
        for cut in [3, 5, bytes.len() / 2, bytes.len() - 1] {
            assert!(SavedChunk::from_bytes(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut newer = bytes.clone();
        newer[4] = 9;
        assert!(SavedChunk::from_bytes(&newer).unwrap_err().contains("version 9"));
        let mut flipped = bytes;
        let last = flipped.len() - 6;
        flipped[last] ^= 0xff;
        assert!(SavedChunk::from_bytes(&flipped).is_err(), "a changed byte is caught");
    }

    #[test]
    fn damaged_saves_are_refused_not_misread() {
        let bytes = sample().to_bytes();
        assert!(SaveData::from_bytes(b"not a save at all").is_err());
        for cut in [0, 8, 12, bytes.len() / 2, bytes.len() - 1] {
            assert!(SaveData::from_bytes(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(SaveData::from_bytes(&longer).is_err());
        let mut newer = bytes;
        newer[8] = 99;
        assert!(SaveData::from_bytes(&newer).unwrap_err().contains("version 99"));
    }
}
