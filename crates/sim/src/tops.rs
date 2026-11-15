//! Tops: for every X and Y of terrain, the Z of the highest boxel that stops light falling from
//! the sky (one whose pieces cover its top or bottom face), so the sky is known without
//! loading what's above. Sky light only goes far straight down (sideways it's gone in a few
//! boxels), so one height for each X and Y is all a chunk needs to know about the world above
//! it: over the top is open sky, and below it a chunk that isn't loaded is a dark wall (see
//! [`crate::light`]).
//!
//! Tops are kept by chunk X and Y, a [`TopsArray`] for the 32 × 32 boxel columns there. They
//! start as the generator's ([`api::Generator::tops`]) and follow edits; ones that edits moved
//! go to the chunk store when they unload, under a place no chunk uses ([`store_key`]), and
//! come back from it. Where no tops are known (around what the mods built, in a client's far
//! corners) a missing chunk is open sky all the way down, as before there were any.

use std::collections::HashMap;
use std::sync::Arc;

use api::{CHUNK_SIZE, ChunkPos, I64Vec3, NO_TOP, TOPS_LEN};

/// The top of each of a chunk's boxel columns, x fastest: a Z in boxels, or [`NO_TOP`].
pub type TopsArray = [i64; TOPS_LEN];

/// Every known column's tops, by chunk X and Y. Columns are shared with the terrain threads
/// and with messages, so changing one copies it first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tops {
    columns: HashMap<(i64, i64), Arc<TopsArray>>,
}

/// Where the boxel column at `x, y` is in its chunk column's array.
fn index(x: i64, y: i64) -> usize {
    (x.rem_euclid(CHUNK_SIZE) + y.rem_euclid(CHUNK_SIZE) * CHUNK_SIZE) as usize
}

impl Tops {
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// The tops at chunk `x, y`, if known.
    pub fn column(&self, x: i64, y: i64) -> Option<&Arc<TopsArray>> {
        self.columns.get(&(x, y))
    }

    /// Every known column, by chunk X and Y, in no particular order.
    pub fn columns(&self) -> impl Iterator<Item = ((i64, i64), &Arc<TopsArray>)> {
        self.columns.iter().map(|(&at, tops)| (at, tops))
    }

    pub fn set_column(&mut self, x: i64, y: i64, tops: Arc<TopsArray>) {
        self.columns.insert((x, y), tops);
    }

    pub fn remove_column(&mut self, x: i64, y: i64) -> Option<Arc<TopsArray>> {
        self.columns.remove(&(x, y))
    }

    pub fn clear(&mut self) {
        self.columns.clear();
    }

    /// The top of the boxel column at boxel `x, y`: `None` where no tops are known, and
    /// [`NO_TOP`] where nothing stops the sky at any height.
    pub fn top(&self, x: i64, y: i64) -> Option<i64> {
        let column = self.columns.get(&(x.div_euclid(CHUNK_SIZE), y.div_euclid(CHUNK_SIZE)))?;
        Some(column[index(x, y)])
    }

    /// Sets the top of the boxel column at boxel `x, y`, if its tops are known.
    pub fn set_top(&mut self, x: i64, y: i64, top: i64) {
        if let Some(column) = self.columns.get_mut(&(x.div_euclid(CHUNK_SIZE), y.div_euclid(CHUNK_SIZE))) {
            Arc::make_mut(column)[index(x, y)] = top;
        }
    }

    /// Whether the boxel at `pos`, in a chunk that isn't loaded, is open sky: over its column's
    /// top, or anywhere at all where no tops are known.
    pub fn is_sky(&self, pos: I64Vec3) -> bool {
        self.top(pos.x, pos.y).is_none_or(|top| pos.z > top)
    }

    /// The tops at chunk `x, y` and the four columns beside it: all that lighting a chunk there
    /// on its own reads.
    pub fn around(&self, x: i64, y: i64) -> Tops {
        let mut out = Tops::default();
        for (dx, dy) in AROUND {
            if let Some(column) = self.column(x + dx, y + dy) {
                out.set_column(x + dx, y + dy, Arc::clone(column));
            }
        }
        out
    }
}

/// A chunk column and the four beside it, as offsets.
pub const AROUND: [(i64, i64); 5] = [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)];

/// The chunk row no chunk is at, where a column's tops are kept in the chunk store.
const STORE_ROW: i64 = i64::MIN;

/// Where the chunk store keeps the tops at chunk `x, y`.
pub fn store_key(x: i64, y: i64) -> ChunkPos {
    ChunkPos(I64Vec3::new(x, y, STORE_ROW))
}

/// Whether `pos` is a place the chunk store keeps tops at, rather than a chunk.
pub fn is_store_key(pos: ChunkPos) -> bool {
    pos.0.z == STORE_ROW
}

/// A column's tops as bytes, for the chunk store: a kind byte, then either the lowest top and
/// each top's height over it in two bytes (`0xffff` for [`NO_TOP`]), or every top whole where
/// they're too far apart for that.
pub fn to_bytes(tops: &TopsArray) -> Vec<u8> {
    let known = || tops.iter().copied().filter(|&t| t != NO_TOP);
    let low = known().min().unwrap_or(0);
    let high = known().max().unwrap_or(0);
    let mut out = Vec::with_capacity(9 + TOPS_LEN * 2);
    if high.saturating_sub(low) < 0xffff {
        out.push(1);
        out.extend_from_slice(&low.to_le_bytes());
        for &t in tops.iter() {
            let over: u16 = if t == NO_TOP { 0xffff } else { (t - low) as u16 };
            out.extend_from_slice(&over.to_le_bytes());
        }
    } else {
        out.push(2);
        for &t in tops.iter() {
            out.extend_from_slice(&t.to_le_bytes());
        }
    }
    out
}

/// A column's tops back from [`to_bytes`].
pub fn from_bytes(bytes: &[u8]) -> Result<TopsArray, String> {
    let mut tops = [NO_TOP; TOPS_LEN];
    match bytes.split_first() {
        Some((1, rest)) if rest.len() == 8 + TOPS_LEN * 2 => {
            let (low, rest) = rest.split_at(8);
            let low = i64::from_le_bytes(low.try_into().expect("eight bytes"));
            for (top, over) in tops.iter_mut().zip(rest.as_chunks::<2>().0) {
                let over = u16::from_le_bytes(*over);
                if over != 0xffff {
                    *top = low + over as i64;
                }
            }
        }
        Some((2, rest)) if rest.len() == TOPS_LEN * 8 => {
            for (top, whole) in tops.iter_mut().zip(rest.as_chunks::<8>().0) {
                *top = i64::from_le_bytes(*whole);
            }
        }
        Some((kind, rest)) => return Err(format!("stored tops of kind {kind}, {} bytes long", rest.len())),
        None => return Err("stored tops with nothing in them".into()),
    }
    Ok(tops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_boxel_is_sky_over_its_top_and_wherever_no_tops_are_known() {
        let mut tops = Tops::default();
        assert!(tops.is_sky(I64Vec3::new(5, 5, -1000)), "no tops known");
        let mut column = [10; TOPS_LEN];
        column[index(-3, -40)] = NO_TOP;
        tops.set_column(-1, -2, Arc::new(column));
        assert_eq!(tops.top(-1, -33), Some(10));
        assert!(tops.is_sky(I64Vec3::new(-1, -33, 11)));
        assert!(!tops.is_sky(I64Vec3::new(-1, -33, 10)));
        assert!(tops.is_sky(I64Vec3::new(-3, -40, -5000)), "nothing stops the sky there");
        assert!(tops.is_sky(I64Vec3::new(0, -33, -5000)), "another column, not known");
        tops.set_top(-1, -33, 99);
        assert_eq!((tops.top(-1, -33), tops.top(-2, -33)), (Some(99), Some(10)));
        tops.set_top(500, 500, 1);
        assert_eq!(tops.top(500, 500), None, "tops that aren't known stay unknown");
    }

    #[test]
    fn changing_a_shared_column_leaves_the_others_copy_alone() {
        let mut tops = Tops::default();
        tops.set_column(0, 0, Arc::new([7; TOPS_LEN]));
        let shared = Arc::clone(tops.column(0, 0).unwrap());
        tops.set_top(1, 1, 8);
        assert_eq!((shared[index(1, 1)], tops.top(1, 1)), (7, Some(8)));
    }

    #[test]
    fn tops_come_back_from_their_bytes() {
        let mut near = [NO_TOP; TOPS_LEN];
        near[3] = -70_000;
        near[900] = -70_000 + 0xfffe;
        let mut apart = near;
        apart[4] = 1 << 40;
        for tops in [near, apart, [NO_TOP; TOPS_LEN], [5; TOPS_LEN]] {
            assert_eq!(from_bytes(&to_bytes(&tops)).unwrap(), tops);
        }
        assert!(to_bytes(&near).len() < to_bytes(&apart).len());
        assert!(from_bytes(&[9, 0, 0]).is_err());
        assert!(from_bytes(&[]).is_err());
    }

    #[test]
    fn store_keys_are_no_chunks_place() {
        assert!(is_store_key(store_key(4, -9)));
        assert!(!is_store_key(ChunkPos(I64Vec3::new(4, -9, -1_000_000))));
    }
}
