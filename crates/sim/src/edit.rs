//! Breaking and placing blocks: hold-to-repeat timing and the record of which chunks changed.

use std::collections::HashSet;

use api::{BoxelPos, CHUNK_SIZE, ChunkPos, I64Vec3};

/// Hold-to-repeat for one button: acts on the press, then every `interval` while held.
#[derive(Clone, Debug, Default)]
pub struct Repeater {
    /// The button went down and hasn't been released, so holding it repeats.
    active: bool,
    cooldown: f32,
}

impl Repeater {
    /// Advances one tick. Returns whether the action happens this tick. Holding a button that
    /// wasn't pressed while it counted (for example a click that grabbed the mouse) does nothing.
    pub fn update(&mut self, pressed: bool, held: bool, interval: f32, dt: f32) -> bool {
        if pressed {
            self.active = true;
            self.cooldown = interval;
            return true;
        }
        if !held {
            self.active = false;
            return false;
        }
        if !self.active {
            return false;
        }
        self.cooldown -= dt;
        // Slack so a quarter second of 60 Hz ticks is exactly 15 ticks despite rounding.
        if self.cooldown > 1e-5 {
            return false;
        }
        self.cooldown = (self.cooldown + interval).max(0.0);
        true
    }
}

/// Chunks whose blocks changed, plus neighbors whose border faces the change affects; and,
/// kept separately for saving, the chunks edited since they were last made from scratch; and,
/// for what reacts to changes nearby, the boxels changed; and, for relighting and for records
/// (see [`crate::mend`]), the boxels changed again, each kept apart as reactions take theirs
/// during the tick.
#[derive(Debug, Default)]
pub struct ChangedChunks(HashSet<ChunkPos>, HashSet<ChunkPos>, Vec<BoxelPos>, Vec<BoxelPos>, Vec<BoxelPos>);

impl ChangedChunks {
    /// Records a change to the boxel at `pos`. Its chunk counts as edited.
    pub fn mark(&mut self, pos: BoxelPos) {
        self.2.push(pos);
        self.3.push(pos);
        self.4.push(pos);
        let (chunk, local) = pos.split();
        self.0.insert(chunk);
        self.1.insert(chunk);
        let local = local.to_ivec().to_array();
        for axis in 0..3 {
            let mut side = [0i64; 3];
            if local[axis] == 0 {
                side[axis] = -1;
            } else if local[axis] == CHUNK_SIZE - 1 {
                side[axis] = 1;
            } else {
                continue;
            }
            self.0.insert(ChunkPos(chunk.0 + I64Vec3::from_array(side)));
        }
    }

    /// Records a change to the record of the boxel at `pos` alone (see [`crate::mend`]): its
    /// chunk counts as changed and edited, so it's sent and saved, but nothing reacts to it.
    pub fn mark_record(&mut self, pos: BoxelPos) {
        let chunk = pos.chunk();
        self.0.insert(chunk);
        self.1.insert(chunk);
    }

    /// Every boxel marked since the last call to this, for bringing records up to date; apart
    /// from [`ChangedChunks::take_boxels`].
    pub fn take_for_records(&mut self) -> Vec<BoxelPos> {
        std::mem::take(&mut self.4)
    }

    /// Records a change to a whole chunk, such as one generated again. Its neighbors aren't
    /// marked: a box of chunks generated together marks all of its own.
    pub fn mark_chunk(&mut self, pos: ChunkPos) {
        self.0.insert(pos);
    }

    /// Every boxel marked since the last call, in the order marked, perhaps more than once.
    pub fn take_boxels(&mut self) -> Vec<BoxelPos> {
        std::mem::take(&mut self.2)
    }

    /// Every boxel marked since the last call to this, for relighting; apart from
    /// [`ChangedChunks::take_boxels`].
    pub fn take_unlit(&mut self) -> Vec<BoxelPos> {
        std::mem::take(&mut self.3)
    }

    /// Every chunk changed since the last call, in no particular order.
    pub fn take(&mut self) -> Vec<ChunkPos> {
        self.0.drain().collect()
    }

    /// Every chunk a boxel changed in since the chunk was last made from scratch (see
    /// [`ChangedChunks::forget_edits`]). Unlike [`ChangedChunks::take`], it isn't drained.
    pub fn edited(&self) -> &HashSet<ChunkPos> {
        &self.1
    }

    /// Stops counting `chunk` as edited, once it's been made again from scratch.
    pub fn forget_edits(&mut self, chunk: ChunkPos) {
        self.1.remove(&chunk);
    }

    /// Stops counting any chunk as edited.
    pub fn forget_all_edits(&mut self) {
        self.1.clear();
    }

    /// Counts `chunk` as edited, as when it's loaded from a save.
    pub fn mark_edited(&mut self, chunk: ChunkPos) {
        self.1.insert(chunk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    #[test]
    fn repeats_while_held() {
        let mut r = Repeater::default();
        let interval = 0.25;
        assert!(r.update(true, true, interval, DT), "acts on the press");
        let mut acted = Vec::new();
        for tick in 1..=45 {
            if r.update(false, true, interval, DT) {
                acted.push(tick);
            }
        }
        assert_eq!(acted, [15, 30, 45]);
        assert!(!r.update(false, false, interval, DT));
    }

    #[test]
    fn holding_without_a_press_does_nothing() {
        let mut r = Repeater::default();
        for _ in 0..60 {
            assert!(!r.update(false, true, 0.25, DT));
        }
    }

    #[test]
    fn releasing_stops_the_repeat() {
        let mut r = Repeater::default();
        r.update(true, true, 0.25, DT);
        r.update(false, false, 0.25, DT);
        for _ in 0..60 {
            assert!(!r.update(false, true, 0.25, DT));
        }
    }

    #[test]
    fn a_fast_rate_acts_every_tick() {
        let mut r = Repeater::default();
        r.update(true, true, 0.001, DT);
        assert!((0..10).all(|_| r.update(false, true, 0.001, DT)));
    }

    fn sorted(mut chunks: Vec<ChunkPos>) -> Vec<ChunkPos> {
        chunks.sort_by_key(|c| c.0.to_array());
        chunks
    }

    #[test]
    fn inside_a_chunk_marks_only_that_chunk() {
        let mut changed = ChangedChunks::default();
        changed.mark(BoxelPos::new(5, 5, 5));
        assert_eq!(changed.take(), [ChunkPos::new(0, 0, 0)]);
        assert!(changed.take().is_empty(), "take drains");
        assert!(changed.edited().contains(&ChunkPos::new(0, 0, 0)), "edits are remembered");
        changed.forget_edits(ChunkPos::new(0, 0, 0));
        assert!(changed.edited().is_empty());
    }

    #[test]
    fn borders_mark_their_neighbors() {
        let mut changed = ChangedChunks::default();
        changed.mark(BoxelPos::new(31, 0, 5));
        assert_eq!(
            sorted(changed.take()),
            sorted(vec![ChunkPos::new(0, 0, 0), ChunkPos::new(1, 0, 0), ChunkPos::new(0, -1, 0)])
        );

        // A corner touches three neighbors, and negative chunks work the same.
        changed.mark(BoxelPos::new(-32, -1, -33));
        assert_eq!(
            sorted(changed.take()),
            sorted(vec![
                ChunkPos::new(-1, -1, -2),
                ChunkPos::new(-2, -1, -2),
                ChunkPos::new(-1, 0, -2),
                ChunkPos::new(-1, -1, -1),
            ])
        );
    }
}
