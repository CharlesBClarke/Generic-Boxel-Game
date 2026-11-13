//! Keeping the tops (see [`crate::tops`]) with the world: following edits, sending what moved
//! to clients, and putting columns edits moved in the chunk store when they're left behind.

use std::sync::Arc;

use api::{BoxelPos, CHUNK_SIZE, ChunkPos, I64Vec3, NO_TOP, WorldPos};

use super::Simulation;
use crate::light::Unloaded;
use crate::stream::UNLOAD_MARGIN;
use crate::tops::{TopsArray, store_key, to_bytes};

/// The tops that changed since a client was last told.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TopsNews {
    /// Columns dropped, by chunk X and Y: to forget first.
    pub dropped: Vec<(i64, i64)>,
    /// Columns that are new or changed, whole, in no particular order.
    pub set: Vec<((i64, i64), Arc<TopsArray>)>,
}

impl TopsNews {
    pub fn is_empty(&self) -> bool {
        self.dropped.is_empty() && self.set.is_empty()
    }
}

impl Simulation {
    /// Brings the tops up to date after the boxels at `changed` changed (see
    /// [`crate::Grid::follow_tops`]). Where the search for a new top runs into a chunk that
    /// isn't loaded, the store and the generator say what's there: an edited chunk may hold
    /// anything, so the top is taken to be at its very top until it loads; an untouched one
    /// holds what the generator put there. Returns more boxels to relight.
    pub(super) fn follow_tops(&mut self, changed: &[BoxelPos]) -> Vec<BoxelPos> {
        if changed.is_empty() || self.universe.main().tops.is_empty() {
            return Vec::new();
        }
        let Some((generator, terrain)) = self.terrain() else { return Vec::new() };
        let store = Arc::clone(self.stream.store());
        let mut generated: Option<((i64, i64), TopsArray)> = None;
        let mut unloaded = |x: i64, y: i64, row: i64| {
            let column = (x.div_euclid(CHUNK_SIZE), y.div_euclid(CHUNK_SIZE));
            let highest = (row + 1) * CHUNK_SIZE - 1;
            if !matches!(store.read(ChunkPos(I64Vec3::new(column.0, column.1, row))), Ok(None)) {
                return Unloaded::Top(highest);
            }
            if generated.as_ref().is_none_or(|(at, _)| *at != column) {
                let Ok(generator) = generator.read() else { return Unloaded::Top(highest) };
                generated = Some((column, crate::stream::world_tops(&**generator, terrain.center.0, column.0, column.1)));
            }
            let top = generated.as_ref().map_or(NO_TOP, |(_, tops)| tops[(x.rem_euclid(CHUNK_SIZE) + y.rem_euclid(CHUNK_SIZE) * CHUNK_SIZE) as usize]);
            if top == NO_TOP || top < row * CHUNK_SIZE { Unloaded::Nothing } else { Unloaded::Top(top.min(highest)) }
        };
        let moved = self.universe.main_mut().follow_tops(&self.blocks, changed, &mut unloaded);
        for column in moved.columns {
            self.tops_dirty.insert(column);
            self.tops_set.insert(column);
        }
        moved.relight
    }

    /// Drops the tops of columns well past every one of `centers`: past where any chunk is
    /// loaded or on its way, and the columns beside those. Ones edits moved go to the chunk
    /// store.
    pub(super) fn prune_tops(&mut self, centers: &[WorldPos]) {
        let limit = self.stream.radius + UNLOAD_MARGIN + 3.0 * CHUNK_SIZE as f32;
        let away = |x: i64, y: i64| {
            let middle = I64Vec3::new(x * CHUNK_SIZE + CHUNK_SIZE / 2, y * CHUNK_SIZE + CHUNK_SIZE / 2, 0);
            centers.iter().all(|c| ((middle - c.cell()).as_vec3() - c.offset()).truncate().length() > limit)
        };
        let far: Vec<(i64, i64)> = self.universe.main().tops.columns().map(|(at, _)| at).filter(|&(x, y)| away(x, y)).collect();
        for (x, y) in far {
            let Some(tops) = self.universe.main_mut().tops.remove_column(x, y) else { continue };
            if self.tops_dirty.remove(&(x, y)) {
                self.stream.store().put(vec![(store_key(x, y), to_bytes(&tops))]);
            }
            self.tops_set.remove(&(x, y));
            self.tops_dropped.push((x, y));
            self.stream.forget_tops(x, y);
        }
    }

    /// Forgets every column's tops, as when the terrain is made again or another world loads.
    pub(super) fn forget_tops(&mut self) {
        let tops = &mut self.universe.main_mut().tops;
        self.tops_dropped.extend(tops.columns().map(|(at, _)| at));
        tops.clear();
        self.tops_dirty.clear();
        self.tops_set.clear();
    }

    /// Puts the tops edits have moved in the store's unsaved layer, for a save.
    pub(super) fn store_tops(&self) {
        let tops = &self.universe.main().tops;
        let moved: Vec<(ChunkPos, Vec<u8>)> =
            self.tops_dirty.iter().filter_map(|&(x, y)| Some((store_key(x, y), to_bytes(tops.column(x, y)?)))).collect();
        if !moved.is_empty() {
            self.stream.store().put(moved);
        }
    }

    /// The tops that are new, changed or dropped since the last call, for clients.
    pub fn take_tops(&mut self) -> TopsNews {
        let tops = &self.universe.main().tops;
        let mut set: Vec<((i64, i64), Arc<TopsArray>)> =
            self.tops_set.drain().filter_map(|at| Some((at, Arc::clone(tops.column(at.0, at.1)?)))).collect();
        set.sort_unstable_by_key(|(at, _)| *at);
        TopsNews { dropped: std::mem::take(&mut self.tops_dropped), set }
    }
}
