//! The chunk store: where edited terrain goes when its column unloads, and where it comes back
//! from when the column loads again, so it never has to stay in memory. The simulation only
//! sees this trait; where the chunks really live is the server's call (`server::worlds` keeps
//! them in region files next to the world's file). No file access here.
//!
//! A store keeps two layers, so a save stays a snapshot to come back to:
//! - **saved**: the chunks as of the last save;
//! - **unsaved**: chunks put since, which win over saved ones.
//!
//! [`ChunkStore::commit`] makes the unsaved layer the saved one (saving); [`ChunkStore::discard`]
//! throws it away (going back to the last save). [`ChunkStore::forget`] hides every chunk, as
//! when the terrain is made again from scratch, until the next commit makes that stick or a
//! discard brings the saved chunks back.
//!
//! Chunks travel as bytes (see [`crate::save::SavedChunk::to_bytes`]), so a store never needs
//! the block registry and can be read from the terrain threads.

use std::collections::HashMap;
use std::sync::Mutex;

use api::ChunkPos;

/// Where edited terrain chunks are kept while unloaded. Shared with the terrain threads, which
/// read from it, so every call takes `&self`.
pub trait ChunkStore: Send + Sync {
    /// The chunk at `pos` as it stands (unsaved over saved), if one is stored.
    fn read(&self, pos: ChunkPos) -> Result<Option<Vec<u8>>, String>;
    /// Puts chunks in the unsaved layer, replacing any stored at the same places. Reads see
    /// them at once; writing them out may come later.
    fn put(&self, chunks: Vec<(ChunkPos, Vec<u8>)>);
    /// Hides every stored chunk and throws away the unsaved ones, until the next commit or
    /// discard.
    fn forget(&self) -> Result<(), String>;
    /// Makes the chunks as they stand the saved ones. Fails if anything put earlier couldn't be
    /// written.
    fn commit(&self) -> Result<(), String>;
    /// Throws away the unsaved chunks (and a forget), leaving the saved ones.
    fn discard(&self) -> Result<(), String>;
    /// Every chunk stored as it stands, in no particular order.
    fn positions(&self) -> Result<Vec<ChunkPos>, String>;
}

/// Copies every chunk of `from`, as it stands, into `to`, which then holds those and nothing
/// else, saved.
pub fn copy(from: &dyn ChunkStore, to: &dyn ChunkStore) -> Result<(), String> {
    to.forget()?;
    let positions = from.positions()?;
    for batch in positions.chunks(256) {
        let mut chunks = Vec::with_capacity(batch.len());
        for &pos in batch {
            if let Some(bytes) = from.read(pos)? {
                chunks.push((pos, bytes));
            }
        }
        to.put(chunks);
    }
    to.commit()
}

/// A store in memory, for a simulation with nowhere to save, and for tests.
#[derive(Default)]
pub struct MemoryStore {
    layers: Mutex<Layers>,
}

#[derive(Default)]
struct Layers {
    saved: HashMap<ChunkPos, Vec<u8>>,
    unsaved: HashMap<ChunkPos, Vec<u8>>,
    /// The saved layer is hidden (see [`ChunkStore::forget`]).
    forgotten: bool,
}

impl MemoryStore {
    fn layers(&self) -> std::sync::MutexGuard<'_, Layers> {
        // Nothing holding the lock can panic halfway through a change.
        self.layers.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl ChunkStore for MemoryStore {
    fn read(&self, pos: ChunkPos) -> Result<Option<Vec<u8>>, String> {
        let l = self.layers();
        let saved = if l.forgotten { None } else { l.saved.get(&pos) };
        Ok(l.unsaved.get(&pos).or(saved).cloned())
    }

    fn put(&self, chunks: Vec<(ChunkPos, Vec<u8>)>) {
        self.layers().unsaved.extend(chunks);
    }

    fn forget(&self) -> Result<(), String> {
        let mut l = self.layers();
        l.unsaved.clear();
        l.forgotten = true;
        Ok(())
    }

    fn commit(&self) -> Result<(), String> {
        let mut l = self.layers();
        if std::mem::take(&mut l.forgotten) {
            l.saved.clear();
        }
        let unsaved = std::mem::take(&mut l.unsaved);
        l.saved.extend(unsaved);
        Ok(())
    }

    fn discard(&self) -> Result<(), String> {
        let mut l = self.layers();
        l.unsaved.clear();
        l.forgotten = false;
        Ok(())
    }

    fn positions(&self) -> Result<Vec<ChunkPos>, String> {
        let l = self.layers();
        let mut out: Vec<ChunkPos> = l.unsaved.keys().copied().collect();
        if !l.forgotten {
            out.extend(l.saved.keys().filter(|p| !l.unsaved.contains_key(p)));
        }
        Ok(out)
    }
}

/// Checks a store keeps to the rules in this module's docs. Public so other stores (the
/// server's files) can be held to the same rules.
pub fn check_store(store: &dyn ChunkStore) {
    let (a, b, c) = (ChunkPos::new(1, 2, 3), ChunkPos::new(-40, 7, 0), ChunkPos::new(600, -600, -2));
    let bytes = |n: u8| vec![n; 3 + n as usize];
    let read = |pos| store.read(pos).expect("reading");
    let sorted = |mut v: Vec<ChunkPos>| {
        v.sort_by_key(|p| p.0.to_array());
        v
    };
    assert_eq!(read(a), None, "an empty store");

    store.put(vec![(a, bytes(1)), (b, bytes(2))]);
    assert_eq!(read(a), Some(bytes(1)), "a put is read at once");
    store.commit().unwrap();
    assert_eq!(read(a), Some(bytes(1)), "and after it's saved");

    store.put(vec![(a, bytes(3)), (c, bytes(4))]);
    assert_eq!(read(a), Some(bytes(3)), "unsaved wins over saved");
    assert_eq!(sorted(store.positions().unwrap()), sorted(vec![a, b, c]));
    store.discard().unwrap();
    assert_eq!((read(a), read(c)), (Some(bytes(1)), None), "a discard goes back to the save");

    store.put(vec![(c, bytes(5))]);
    store.forget().unwrap();
    assert_eq!((read(a), read(b), read(c)), (None, None, None), "forgetting hides everything");
    assert_eq!(store.positions().unwrap(), vec![]);
    store.discard().unwrap();
    assert_eq!((read(a), read(c)), (Some(bytes(1)), None), "until a discard brings the save back");

    store.forget().unwrap();
    store.put(vec![(c, bytes(6))]);
    assert_eq!(read(c), Some(bytes(6)), "a put after forgetting is seen");
    store.commit().unwrap();
    assert_eq!(sorted(store.positions().unwrap()), vec![c], "a commit after forgetting keeps only what came after");
    store.discard().unwrap();
    assert_eq!((read(a), read(c)), (None, Some(bytes(6))));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memory_store_keeps_to_the_rules() {
        check_store(&MemoryStore::default());
    }

    #[test]
    fn copying_gives_the_chunks_as_they_stand() {
        let (from, to) = (MemoryStore::default(), MemoryStore::default());
        let (a, b) = (ChunkPos::new(0, 0, 0), ChunkPos::new(1, 0, 0));
        from.put(vec![(a, vec![1])]);
        from.commit().unwrap();
        from.put(vec![(b, vec![2])]);
        to.put(vec![(ChunkPos::new(9, 9, 9), vec![9])]);
        to.commit().unwrap();
        copy(&from, &to).unwrap();
        let mut got = to.positions().unwrap();
        got.sort_by_key(|p| p.0.to_array());
        assert_eq!(got, vec![a, b], "the copy holds only the source's chunks");
        to.discard().unwrap();
        assert_eq!(to.read(b).unwrap(), Some(vec![2]), "saved");
    }
}
