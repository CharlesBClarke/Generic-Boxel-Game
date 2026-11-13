//! Generated terrain: generators that fill chunks from nothing but where the chunk is, and
//! placements that say where in the world a generator's terrain goes.
//!
//! A generator is pure: the same chunk comes out whatever order chunks are asked for, and on
//! whichever thread, so the engine can fill many at once (and later stream them).

use crate::{BoxelPos, CHUNK_SIZE, CHUNK_VOLUME, I64Vec3, Tunables};

/// Fills chunks of terrain. One per kind of terrain; it keeps no state between chunks, and its
/// tunables (scales, heights, its seed) shape every chunk it fills.
pub trait Generator: Tunables + Send + Sync {
    /// Fills the chunk whose lowest corner is `origin`, measured in boxels from the terrain's
    /// center (see [`TerrainDef`]). `out` comes in empty.
    fn fill(&self, origin: I64Vec3, out: &mut ChunkFill);

    /// Fills a coarse chunk for far terrain: still [`CHUNK_SIZE`] cells a side, but each cell
    /// stands for `step` boxels a side (`step` a power of two), so the chunk spans
    /// `CHUNK_SIZE * step` boxels from its lowest corner `origin` (relative to the terrain's
    /// center, as for [`Generator::fill`]). A cell holds what fills most of the boxels it stands
    /// for, or stays empty. Only how it looks from afar matters: nobody walks on it.
    ///
    /// Returns `false` if the generator can't, as the default does; its terrain then has no far
    /// terrain. At `step` 1 it should fill exactly what [`Generator::fill`] does.
    fn fill_coarse(&self, origin: I64Vec3, step: u32, out: &mut ChunkFill) -> bool {
        let _ = (origin, step, out);
        false
    }

    /// The top of the terrain over one chunk's worth of ground: for each of the
    /// [`CHUNK_SIZE`] × [`CHUNK_SIZE`] cells from `x, y` (boxels from the terrain's center, x
    /// fastest in `out`), the Z of the highest boxel the generator fills there, measured from
    /// the terrain's center, or [`NO_TOP`] where it fills nothing at any height. The engine
    /// knows from this where the open sky starts without loading what's above, so the answer
    /// may be too high (the sky starts later than it could) but never too low.
    ///
    /// Each cell is `step` boxels a side, as for [`Generator::fill_coarse`]. At `step` 1 it
    /// should be exact; above that it only says roughly how high far terrain reaches, and the
    /// top at each cell's middle will do.
    fn tops(&self, x: i64, y: i64, step: u32, out: &mut [i64]);
}

/// What [`Generator::tops`] gives where a generator fills nothing at any height.
pub const NO_TOP: i64 = i64::MIN;

/// How many cells [`Generator::tops`] fills: one for each boxel column of a chunk.
pub const TOPS_LEN: usize = (CHUNK_SIZE * CHUNK_SIZE) as usize;

/// One chunk's worth of blocks, as plain data: a palette of block names, and each boxel's
/// entry in it. Entry 0 is always empty.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkFill {
    /// Block names, by entry, starting at entry 1.
    pub palette: Vec<String>,
    /// Each boxel's entry, in [`crate::LocalPos::index`] order.
    pub cells: Vec<u16>,
}

impl Default for ChunkFill {
    fn default() -> Self {
        Self { palette: Vec::new(), cells: vec![0; CHUNK_VOLUME] }
    }
}

impl ChunkFill {
    /// Empties it, keeping its memory, to fill another chunk.
    pub fn clear(&mut self) {
        self.palette.clear();
        self.cells.fill(0);
    }

    /// The entry for a block name, adding it to the palette if it's new.
    pub fn entry(&mut self, name: &str) -> u16 {
        match self.palette.iter().position(|n| n == name) {
            Some(i) => i as u16 + 1,
            None => {
                self.palette.push(name.to_owned());
                self.palette.len() as u16
            }
        }
    }

    /// Sets the boxel at `x, y, z` inside the chunk (each in `0..CHUNK_SIZE`) to `entry`.
    pub fn set(&mut self, x: usize, y: usize, z: usize, entry: u16) {
        let size = CHUNK_SIZE as usize;
        self.cells[x + y * size + z * size * size] = entry;
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> u16 {
        let size = CHUNK_SIZE as usize;
        self.cells[x + y * size + z * size * size]
    }

    /// The block name at `x, y, z`, or `None` for empty.
    pub fn name(&self, x: usize, y: usize, z: usize) -> Option<&str> {
        match self.get(x, y, z) {
            0 => None,
            e => self.palette.get(e as usize - 1).map(String::as_str),
        }
    }
}

/// Where a generator's terrain goes. It goes on forever from `center`, sideways, up and down,
/// loaded around the players as they move; the generator sees positions relative to `center`.
/// A world takes one terrain: what's deep down or high up is its generator's business, and
/// other worlds are other grids.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainDef {
    /// The registered generator's name.
    pub generator: String,
    /// The terrain's center. Must sit on a chunk corner (each axis a multiple of
    /// [`CHUNK_SIZE`]), so chunks line up with the generator's.
    pub center: BoxelPos,
}

impl TerrainDef {
    pub fn validate(&self) -> Result<(), String> {
        if self.center.0.rem_euclid(I64Vec3::splat(CHUNK_SIZE)) != I64Vec3::ZERO {
            return Err(format!("terrain center {:?} isn't on a chunk corner", self.center.0.to_array()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_start_at_one_and_repeat_for_the_same_name() {
        let mut fill = ChunkFill::default();
        let stone = fill.entry("base:stone");
        let dirt = fill.entry("base:dirt");
        assert_eq!((stone, dirt, fill.entry("base:stone")), (1, 2, 1));
        fill.set(3, 4, 5, dirt);
        assert_eq!(fill.name(3, 4, 5), Some("base:dirt"));
        assert_eq!(fill.name(0, 0, 0), None);
        fill.clear();
        assert_eq!((fill.palette.len(), fill.get(3, 4, 5)), (0, 0));
    }

    #[test]
    fn centers_must_sit_on_chunk_corners() {
        let def = |x| TerrainDef { generator: "a:b".into(), center: BoxelPos::new(x, 64, -32) };
        assert!(def(32).validate().is_ok());
        assert!(def(33).validate().is_err());
    }
}
