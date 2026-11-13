//! Far terrain: generated terrain drawn coarsely past the full-detail columns, so the view
//! reaches far without loading the world there.
//!
//! A level-*k* chunk is a chunk's 32³ cells, each standing for 2^*k* boxels a side, so the same
//! [`ChunkData`] carries it; positions are in level-*k* chunks (a level-*k* chunk at `p` spans
//! boxels `p * 32 * 2^k` on). A far column is the terrain's level-*k* chunks at one X and Y,
//! from the lowest of its tops to the highest (see [`api::Generator::tops`]) and wherever
//! edits reach. Level 0 is the ordinary chunks [`crate::stream`] loads, taken a column at a
//! time.
//!
//! Far terrain is still in columns, while the chunks near the player load by cube: a stand-in
//! until it picks its levels by cube too.
//!
//! Which level is drawn where is a quadtree over columns ([`Reach::pick`]): a column splits into
//! its four children of the level below when it's near enough, and only once all four can be
//! drawn, so nothing is drawn twice and nothing goes missing while levels load. The server
//! builds every column the quadtree may want around the player ([`Reach::wanted`]) on the
//! terrain threads, from the generator's coarse fill ([`api::Generator::fill_coarse`]), with
//! the edited chunks under it shrunk in, and sends them; the client picks what to draw.
//! Far chunks are data only: no light worked out (open sky everywhere), never simulated.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::Arc;
use std::time::Instant;

use api::{ApiError, CHUNK_SIZE, ChunkFill, ChunkPos, I64Vec3, LocalPos, TerrainDef, WorldPos};

use crate::save::SavedChunk;
use crate::store::ChunkStore;
use crate::stream::{SharedGenerator, Stream};
use crate::{BlockRegistry, ChunkData, Occupant};

/// The coarsest level: cells 64 boxels a side, columns 2 km across.
pub const MAX_LEVEL: u8 = 6;

/// One terrain's level-`level` chunks at one X and Y, in level-`level` chunks. Level 0 is an
/// ordinary column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FarColumn {
    pub terrain: usize,
    pub level: u8,
    pub x: i64,
    pub y: i64,
}

impl FarColumn {
    /// Boxels each cell stands for, a side.
    pub fn step(self) -> i64 {
        1 << self.level
    }

    /// Boxels across the column.
    pub fn span(self) -> i64 {
        CHUNK_SIZE << self.level
    }

    /// The level-`level` column holding the ordinary column at chunk `x, y`.
    pub fn holding(terrain: usize, level: u8, x: i64, y: i64) -> Self {
        let n = 1i64 << level;
        Self { terrain, level, x: x.div_euclid(n), y: y.div_euclid(n) }
    }

    /// The column of the level above holding this one.
    pub fn parent(self) -> Self {
        Self { terrain: self.terrain, level: self.level + 1, x: self.x.div_euclid(2), y: self.y.div_euclid(2) }
    }

    /// The four columns of the level below that make this one up.
    pub fn children(self) -> [Self; 4] {
        let (x, y, level) = (self.x * 2, self.y * 2, self.level.saturating_sub(1));
        let c = |dx, dy| Self { terrain: self.terrain, level, x: x + dx, y: y + dy };
        [c(0, 0), c(1, 0), c(0, 1), c(1, 1)]
    }

    /// The ordinary columns under it, as chunk X and Y ranges.
    pub fn under(self) -> (Range<i64>, Range<i64>) {
        let n = 1i64 << self.level;
        (self.x * n..(self.x + 1) * n, self.y * n..(self.y + 1) * n)
    }

    /// Horizontal meters from `pos` to the column's middle.
    pub fn distance(self, pos: WorldPos) -> f32 {
        let span = self.span();
        let middle = I64Vec3::new(self.x * span + span / 2, self.y * span + span / 2, 0);
        let d = (middle - pos.cell()).as_vec3() - pos.offset();
        d.truncate().length()
    }

    /// Horizontal meters from `pos` to the nearest point of the column; 0 inside it.
    pub fn nearest(self, pos: WorldPos) -> f32 {
        let span = self.span();
        let lo = I64Vec3::new(self.x * span, self.y * span, 0);
        let rel = (pos.cell() - lo).as_vec3() + pos.offset();
        let out = |v: f32| if v < 0.0 { -v } else if v > span as f32 { v - span as f32 } else { 0.0 };
        (out(rel.x).powi(2) + out(rel.y).powi(2)).sqrt()
    }

    /// Its chunk rows (Z, in level-`level` chunks) for a terrain whose ordinary rows are `rows`:
    /// every level chunk any of them is in.
    pub fn rows(self, rows: &Range<i64>) -> Range<i64> {
        if rows.is_empty() {
            return 0..0;
        }
        let n = 1i64 << self.level;
        rows.start.div_euclid(n)..(rows.end - 1).div_euclid(n) + 1
    }

    /// Every chunk in the column, in level chunks, top first.
    pub fn chunks(self, rows: &Range<i64>) -> impl Iterator<Item = ChunkPos> {
        self.rows(rows).rev().map(move |z| ChunkPos(I64Vec3::new(self.x, self.y, z)))
    }
}

/// How far the view reaches: full detail out to `detail` meters, coarser and coarser out to
/// `far`, each level reaching `level_reach` times as far as the one before. At 2 a cell looks
/// about as big at the near edge of every level; less makes the levels coarse sooner, for
/// fewer triangles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reach {
    pub detail: f32,
    pub far: f32,
    pub level_reach: f32,
}

impl Default for Reach {
    fn default() -> Self {
        // 1.6: 2.0 drew ~1.4M far triangles from a hill on the base terrain, 1.6 ~0.6M
        // (2026-11-12).
        Self { detail: 256.0, far: 2048.0, level_reach: 1.6 }
    }
}

/// The range [`Reach::level_reach`] is kept in.
pub const LEVEL_REACH: std::ops::RangeInclusive<f32> = 1.25..=4.0;

impl Reach {
    /// The coarsest level used: the first whose columns never split at the far distance.
    pub fn top(&self) -> u8 {
        let mut level = 1u8;
        while level < MAX_LEVEL && self.split_distance(level) < self.far {
            level += 1;
        }
        level
    }

    /// Meters within which a level-`level` column's middle must be for it to split: the
    /// full-detail distance, times the level reach each level up, but never less than the
    /// column's width, so the column the player is in always splits however short full detail
    /// is.
    pub fn split_distance(&self, level: u8) -> f32 {
        if level == 0 {
            return 0.0;
        }
        let growth = self.level_reach.clamp(*LEVEL_REACH.start(), *LEVEL_REACH.end());
        let reach = self.detail.max(1.0) * growth.powi(level as i32 - 1);
        reach.max((CHUNK_SIZE << level) as f32)
    }

    /// Whether there's far terrain at all: the far distance past full detail.
    pub fn has_far(&self) -> bool {
        self.far > self.detail
    }

    /// Whether `column` splits into its children, seen from `pos`.
    pub fn splits(&self, column: FarColumn, pos: WorldPos) -> bool {
        column.level > 0 && column.distance(pos) < self.split_distance(column.level)
    }

    /// Whether any of `column` is within the far distance of `pos`.
    pub fn reaches(&self, column: FarColumn, pos: WorldPos) -> bool {
        column.nearest(pos) < self.far
    }

    /// The coarsest columns of `terrain` within reach of `pos`.
    fn tops(&self, terrain: usize, pos: WorldPos) -> Vec<FarColumn> {
        let level = self.top();
        let span = (CHUNK_SIZE << level) as f32;
        let (cx, cy) = (pos.cell().x as f32 / span, pos.cell().y as f32 / span);
        let r = (self.far / span).ceil() as i64 + 1;
        let (cx, cy) = (cx.floor() as i64, cy.floor() as i64);
        let mut out = Vec::new();
        for y in cy - r..=cy + r {
            for x in cx - r..=cx + r {
                let c = FarColumn { terrain, level, x, y };
                if self.reaches(c, pos) {
                    out.push(c);
                }
            }
        }
        out
    }

    /// Every far column (level 1 and up) of `terrain` the quadtree may draw from `pos`: those
    /// within reach that are the coarsest, or the child of one that splits. Coarse ones near
    /// the player stay wanted under their children, to fall back on as the player moves.
    pub fn wanted(&self, terrain: usize, pos: WorldPos) -> Vec<FarColumn> {
        let mut out = Vec::new();
        if !self.has_far() {
            return out;
        }
        let mut stack = self.tops(terrain, pos);
        while let Some(c) = stack.pop() {
            if c.level == 0 || !self.reaches(c, pos) {
                continue;
            }
            out.push(c);
            if self.splits(c, pos) {
                stack.extend(c.children());
            }
        }
        out
    }

    /// The columns of `terrain` to draw from `pos`, any level: each spot within reach once at
    /// most, as fine as what's `ready` to draw allows. A column splits only once all its
    /// children can be drawn; until then it's drawn whole if it's ready, or else its ready
    /// children are (leaving holes where nothing is ready).
    pub fn pick(&self, terrain: usize, pos: WorldPos, ready: &dyn Fn(FarColumn) -> bool) -> Vec<FarColumn> {
        let mut out = Vec::new();
        for top in self.tops(terrain, pos) {
            out.extend(self.pick_in(top, pos, ready).0);
        }
        out
    }

    /// What to draw of `column`, and whether that covers all of it within reach.
    fn pick_in(&self, column: FarColumn, pos: WorldPos, ready: &dyn Fn(FarColumn) -> bool) -> (Vec<FarColumn>, bool) {
        if !self.reaches(column, pos) {
            return (Vec::new(), true);
        }
        if self.splits(column, pos) {
            let mut all = Vec::new();
            let mut whole = true;
            for child in column.children() {
                let (list, covered) = self.pick_in(child, pos, ready);
                all.extend(list);
                whole &= covered;
            }
            if whole {
                return (all, true);
            }
            if ready(column) {
                return (vec![column], true);
            }
            return (all, false);
        }
        if ready(column) { (vec![column], true) } else { (Vec::new(), false) }
    }

    /// The reach a little wider, for dropping columns only once they're well past wanted.
    pub fn widened(&self) -> Reach {
        Reach { detail: self.detail * 1.25 + CHUNK_SIZE as f32, far: self.far + 2.0 * CHUNK_SIZE as f32, level_reach: self.level_reach }
    }
}

/// A far column's chunks, as sent: only those with something in them, and not solid all
/// through under one whose lowest layer is solid.
#[derive(Clone, Debug, PartialEq)]
pub struct FarChunks {
    pub column: FarColumn,
    pub chunks: Vec<ChunkData>,
}

/// How far terrain has gone, for the client's readout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FarStats {
    /// Far columns loaded now.
    pub columns: usize,
    /// Wanted and not loaded yet.
    pub waiting: usize,
    /// Seconds the worker threads spent building them, added up.
    pub seconds: f32,
}

/// A far column built on a terrain thread.
struct Built {
    epoch: u64,
    serial: u64,
    column: FarColumn,
    /// `None` if the generator can't fill coarsely.
    chunks: Result<Option<Vec<ChunkData>>, ApiError>,
    seconds: f32,
}

/// What a terrain thread needs to build a far column.
struct FarJob {
    column: FarColumn,
    generator: SharedGenerator,
    center: I64Vec3,
    blocks: Arc<BlockRegistry>,
    store: Arc<dyn ChunkStore>,
    /// Edited chunks under it that are loaded, as they stand.
    loaded: Vec<ChunkData>,
    /// Edited chunks under it to read from the store.
    stored: Vec<ChunkPos>,
    /// Which ordinary columns under it terrain is kept away from (see
    /// [`crate::stream::CLEAR_DISTANCE`]), x fastest; empty if none is.
    cleared: Vec<bool>,
}

/// The far columns around the player: which are loaded, wanted and on their way.
pub struct Far {
    pub reach: Reach,
    loaded: HashSet<FarColumn>,
    /// Asked of the workers, by the serial they were asked with.
    in_flight: HashMap<FarColumn, u64>,
    /// Loaded or asked-for columns an edit has changed since, by the serial a build must be
    /// newer than to count.
    stale: HashMap<FarColumn, u64>,
    serial: u64,
    epoch: u64,
    wanted: Vec<FarColumn>,
    wanted_for: Option<((i64, i64), u32, u32, u64)>,
    /// Edited chunks in the store, once read from it.
    stored: Option<HashSet<ChunkPos>>,
    /// Terrains whose generator can't fill coarsely.
    cannot: HashSet<usize>,
    outbox: Sender<Built>,
    done: Receiver<Built>,
    /// The block registry the workers name blocks from, and how many blocks it had.
    blocks: Option<(usize, Arc<BlockRegistry>)>,
    loaded_now: Vec<FarChunks>,
    unloaded_now: Vec<FarColumn>,
    pub stats: FarStats,
    pub error: Option<ApiError>,
}

impl Default for Far {
    fn default() -> Self {
        let (outbox, done) = channel();
        Self {
            // None until a client says how far it sees.
            reach: Reach { far: 0.0, ..Reach::default() },
            loaded: HashSet::new(),
            in_flight: HashMap::new(),
            stale: HashMap::new(),
            serial: 0,
            epoch: 0,
            wanted: Vec::new(),
            wanted_for: None,
            stored: None,
            cannot: HashSet::new(),
            outbox,
            done,
            blocks: None,
            loaded_now: Vec::new(),
            unloaded_now: Vec::new(),
            stats: FarStats::default(),
            error: None,
        }
    }
}

/// What the simulation hands far terrain each step.
pub(crate) struct Around<'a> {
    pub pos: WorldPos,
    pub terrains: &'a [TerrainDef],
    pub generators: &'a [SharedGenerator],
    pub blocks: &'a BlockRegistry,
    /// Loaded chunks edited since they were generated.
    pub edited: &'a HashSet<ChunkPos>,
    pub grid: &'a crate::Grid,
}

impl Far {
    /// Forgets every far column, as when the terrain is made again or another world loads.
    pub fn reset(&mut self) {
        self.epoch += 1;
        let mut gone: Vec<FarColumn> = self.loaded.drain().collect();
        gone.sort_unstable();
        self.unloaded_now.extend(gone);
        self.loaded_now.clear();
        self.in_flight.clear();
        self.stale.clear();
        self.wanted_for = None;
        self.stored = None;
        self.cannot.clear();
        self.stats = FarStats::default();
        self.error = None;
    }

    /// Notes chunks just put in the store, so far columns over them are built again with them.
    pub fn edited(&mut self, chunks: &[ChunkPos]) {
        if let Some(stored) = &mut self.stored {
            stored.extend(chunks.iter().copied());
        }
        let terrains: HashSet<usize> = self.loaded.iter().chain(self.in_flight.keys()).map(|c| c.terrain).collect();
        for pos in chunks {
            for &terrain in &terrains {
                for level in 1..=MAX_LEVEL {
                    let c = FarColumn::holding(terrain, level, pos.0.x, pos.0.y);
                    if self.loaded.contains(&c) || self.in_flight.contains_key(&c) {
                        self.stale.insert(c, self.serial);
                    }
                }
            }
        }
    }

    /// Whether `column` is loaded.
    pub fn is_loaded(&self, column: FarColumn) -> bool {
        self.loaded.contains(&column)
    }

    /// Whether every wanted column is loaded and up to date.
    pub fn settled(&self) -> bool {
        self.wanted_for.is_some() && self.wanted.iter().all(|c| self.loaded.contains(c) && !self.stale.contains_key(c))
    }

    /// Whether any column is on its way.
    pub fn busy(&self) -> bool {
        !self.in_flight.is_empty()
    }

    /// One step: drops columns well past the reach, asks for wanted ones missing or stale,
    /// coarse and near first, and takes in what's been built. `cleared` says which ordinary
    /// columns terrain is kept away from.
    pub(crate) fn step(&mut self, stream: &mut Stream, around: &Around, cleared: &dyn Fn(i64, i64) -> bool) {
        let pos = around.pos;
        // Drop.
        let keep = around.terrains.len();
        let widened = self.reach.widened();
        let mut far: Vec<FarColumn> = self
            .loaded
            .iter()
            .copied()
            .filter(|c| c.terrain >= keep || !widened.reaches(*c, pos) || !Self::kept(&widened, *c, pos))
            .collect();
        far.sort_unstable();
        for c in far {
            self.loaded.remove(&c);
            self.stale.remove(&c);
            self.unloaded_now.push(c);
        }
        // Want.
        let chunk = pos.boxel().chunk().0;
        let key = ((chunk.x, chunk.y), self.reach.detail.to_bits(), self.reach.far.to_bits() ^ self.reach.level_reach.to_bits().rotate_left(16), self.epoch);
        if self.wanted_for != Some(key) {
            self.wanted_for = Some(key);
            let mut wanted: Vec<(f32, FarColumn)> = Vec::new();
            for terrain in 0..around.terrains.len() {
                for c in self.reach.wanted(terrain, pos) {
                    // As big on screen comes first: coarse far ones as soon as fine near ones.
                    wanted.push((c.distance(pos) / c.step() as f32, c));
                }
            }
            wanted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            self.wanted = wanted.into_iter().map(|(_, c)| c).collect();
        }
        self.take_built();
        self.request(stream, around, cleared);
    }

    /// Whether `column`'s still within what `reach` wants from `pos`: its parent splits there,
    /// or it's among the coarsest.
    fn kept(reach: &Reach, column: FarColumn, pos: WorldPos) -> bool {
        column.level >= reach.top() || reach.splits(column.parent(), pos)
    }

    fn take_built(&mut self) {
        let wanted: HashSet<FarColumn> = self.wanted.iter().copied().collect();
        let came: Vec<Built> = self.done.try_iter().collect();
        for built in came {
            if built.epoch != self.epoch {
                continue;
            }
            if self.in_flight.get(&built.column) == Some(&built.serial) {
                self.in_flight.remove(&built.column);
            }
            self.stats.seconds += built.seconds;
            let chunks = match built.chunks {
                Ok(Some(chunks)) => chunks,
                Ok(None) => {
                    if self.cannot.insert(built.column.terrain) {
                        eprintln!("terrain {}: its generator can't fill coarsely, so it has no far terrain", built.column.terrain);
                    }
                    continue;
                }
                Err(e) => {
                    self.error = Some(e);
                    continue;
                }
            };
            if !wanted.contains(&built.column) {
                continue;
            }
            if let Some(&after) = self.stale.get(&built.column) {
                if built.serial <= after {
                    // Built before an edit under it: it's asked for again.
                    continue;
                }
                self.stale.remove(&built.column);
            }
            self.loaded.insert(built.column);
            self.loaded_now.push(FarChunks { column: built.column, chunks });
        }
        self.stats.columns = self.loaded.len();
    }

    fn request(&mut self, stream: &mut Stream, around: &Around, cleared: &dyn Fn(i64, i64) -> bool) {
        let missing: Vec<FarColumn> = self
            .wanted
            .iter()
            .copied()
            .filter(|c| !self.cannot.contains(&c.terrain) && (!self.loaded.contains(c) || self.stale.contains_key(c)))
            .collect();
        self.stats.waiting = missing.len();
        if missing.is_empty() {
            return;
        }
        let ordinary = stream.in_flight();
        let count = stream.workers().count;
        // Ordinary columns come first, but far ones always have one on its way.
        let room = (count * 2).saturating_sub(ordinary + self.in_flight.len()).max(1usize.saturating_sub(self.in_flight.len()));
        if room == 0 {
            return;
        }
        let store = Arc::clone(stream.store());
        if self.stored.is_none() {
            match store.positions() {
                Ok(p) => self.stored = Some(p.into_iter().filter(|&p| !crate::tops::is_store_key(p)).collect()),
                Err(e) => {
                    self.error = Some(ApiError::Storage(e));
                    self.stored = Some(HashSet::new());
                }
            }
        }
        let blocks = match &self.blocks {
            Some((n, b)) if *n == around.blocks.len() => Arc::clone(b),
            _ => {
                let b = Arc::new(around.blocks.clone());
                self.blocks = Some((around.blocks.len(), Arc::clone(&b)));
                b
            }
        };
        let stored_all = self.stored.as_ref().expect("just read");
        let mut sent = 0;
        for column in missing {
            if sent == room {
                break;
            }
            if let Some(&serial) = self.in_flight.get(&column) {
                let fresh = self.stale.get(&column).is_none_or(|&after| serial > after);
                if fresh {
                    continue;
                }
            }
            let (xs, ys) = column.under();
            let in_column = |p: &ChunkPos| xs.contains(&p.0.x) && ys.contains(&p.0.y);
            let loaded: Vec<ChunkData> = around.edited.iter().filter(|p| in_column(p)).map(|&p| around.grid.export_chunk(p)).collect();
            let stored: Vec<ChunkPos> =
                stored_all.iter().filter(|p| in_column(p) && !around.edited.contains(p)).copied().collect();
            let mut mask: Vec<bool> = Vec::new();
            if (xs.end - xs.start) <= 64 {
                let any = ys.clone().any(|y| xs.clone().any(|x| cleared(x, y)));
                if any {
                    mask = ys.clone().flat_map(|y| xs.clone().map(move |x| (x, y))).map(|(x, y)| cleared(x, y)).collect();
                }
            }
            self.serial += 1;
            let serial = self.serial;
            let job = FarJob {
                column,
                generator: Arc::clone(&around.generators[column.terrain]),
                center: around.terrains[column.terrain].center.0,
                blocks: Arc::clone(&blocks),
                store: Arc::clone(&store),
                loaded,
                stored,
                cleared: mask,
            };
            let (outbox, epoch) = (self.outbox.clone(), self.epoch);
            let task = Box::new(move || {
                let start = Instant::now();
                let chunks = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(&job)))
                    .unwrap_or_else(|_| Err(ApiError::InvalidDef(format!("building far column {:?} panicked", job.column))));
                let seconds = start.elapsed().as_secs_f32();
                let _ = outbox.send(Built { epoch, serial, column: job.column, chunks, seconds });
            });
            if !stream.workers().send(task) {
                break;
            }
            self.in_flight.insert(column, serial);
            sent += 1;
        }
    }

    /// Far columns built since the last call, oldest first. One built again after an edit
    /// comes again, to replace the one before.
    pub fn take_loaded(&mut self) -> Vec<FarChunks> {
        std::mem::take(&mut self.loaded_now)
    }

    /// Far columns dropped since the last call.
    pub fn take_unloaded(&mut self) -> Vec<FarColumn> {
        std::mem::take(&mut self.unloaded_now)
    }

    /// The loaded far columns, in no particular order.
    pub fn loaded(&self) -> impl Iterator<Item = FarColumn> + '_ {
        self.loaded.iter().copied()
    }

    /// Waits until every wanted column is in, for tests and tools. Gives up after a minute.
    pub(crate) fn wait(&mut self, stream: &mut Stream, around: &Around, cleared: &dyn Fn(i64, i64) -> bool) {
        let start = Instant::now();
        loop {
            self.step(stream, around, cleared);
            let pending = self
                .wanted
                .iter()
                .any(|c| !self.cannot.contains(&c.terrain) && (!self.loaded.contains(c) || self.stale.contains_key(c)));
            if !pending || self.error.is_some() || start.elapsed().as_secs() >= 60 {
                return;
            }
            match self.done.recv_timeout(std::time::Duration::from_secs(60)) {
                // Put back, to be taken in with the rest.
                Ok(built) => {
                    let _ = self.outbox.send(built);
                }
                Err(_) => return,
            }
        }
    }
}

/// Builds a far column: the generator's coarse fill, cleared where terrain is kept away, with
/// the edited chunks under it shrunk in. `None` if the generator can't fill coarsely.
fn build(job: &FarJob) -> Result<Option<Vec<ChunkData>>, ApiError> {
    let generator = job.generator.read().map_err(|_| ApiError::InvalidDef("a terrain generator panicked".into()))?;
    let column = job.column;
    let step = column.step();
    let mut edits: Vec<ChunkData> = job.loaded.clone();
    for &pos in &job.stored {
        if let Some(bytes) = job.store.read(pos).map_err(ApiError::Storage)? {
            let saved = SavedChunk::from_bytes(&bytes).map_err(|e| ApiError::Storage(format!("chunk {:?}: {e}", pos.0.to_array())))?;
            let mut report = crate::LoadReport::default();
            edits.push(crate::simulation::save::unname_chunk(&job.blocks, &saved, &mut report));
        }
    }
    // The ordinary chunk rows it spans: from under the lowest of the generator's tops to over
    // the highest, a cell to spare either way, and wherever an edit is.
    let mut tops = [api::NO_TOP; api::TOPS_LEN];
    generator.tops(column.x * column.span() - job.center.x, column.y * column.span() - job.center.y, step as u32, &mut tops);
    let known = || tops.iter().filter(|&&t| t != api::NO_TOP).map(|&t| t + job.center.z);
    let row = |z: i64| z.div_euclid(CHUNK_SIZE);
    let edited = || edits.iter().map(|e| e.pos.0.z);
    let low = known().min().map(|z| row(z - step)).into_iter().chain(edited()).min();
    let high = known().max().map(|z| row(z + step)).into_iter().chain(edited()).max();
    let (Some(low), Some(high)) = (low, high) else { return Ok(Some(Vec::new())) };
    let rows = low..high + 1;
    let mut fill = ChunkFill::default();
    let mut out = Vec::new();
    // Whether the chunk above covers this one: solid all through, or its lowest layer is. A
    // chunk solid all through under one is buried, never seen, and left out.
    let mut covered = false;
    for pos in column.chunks(&rows) {
        fill.clear();
        let origin = pos.0 * column.span();
        if !generator.fill_coarse(origin - job.center, step as u32, &mut fill) {
            return Ok(None);
        }
        let mut data = crate::terrain::to_chunk(pos, &fill, &job.blocks)?;
        if !job.cleared.is_empty() {
            clear(&mut data, column, &job.cleared);
        }
        for edit in &edits {
            shrink_into(&mut data, column, edit, &job.blocks);
        }
        data.cells.compact();
        let uniform = data.cells.uniform();
        if uniform == Some(0) {
            covered = false;
            continue;
        }
        if uniform.is_some() && covered {
            continue;
        }
        let n = CHUNK_SIZE as usize;
        covered = uniform.is_some() || (0..n * n).all(|i| data.cells.at(i) != 0);
        out.push(data);
    }
    Ok(Some(out))
}

/// Empties the cells of `data` over ordinary columns `cleared` marks.
fn clear(data: &mut ChunkData, column: FarColumn, cleared: &[bool]) {
    let n = 1i64 << column.level;
    let size = CHUNK_SIZE;
    for i in 0..api::CHUNK_VOLUME {
        let p = LocalPos::from_index(i);
        // The ordinary column the cell's middle is over.
        let bx = (p.x as i64 * column.step() + column.step() / 2).div_euclid(size);
        let by = (p.y as i64 * column.step() + column.step() / 2).div_euclid(size);
        if bx < n && by < n && cleared[(bx + by * n) as usize] {
            data.cells.set_at(i, 0);
        }
    }
}

/// Writes the cells of `data` (a far chunk of `column`'s level) that `edit` (an ordinary chunk)
/// lies under, shrunk: each cell takes the material that fills most of its boxels, if the
/// boxels are more than half filled, or is emptied. Only levels whose cells are no bigger than
/// a chunk shrink edits in.
fn shrink_into(data: &mut ChunkData, column: FarColumn, edit: &ChunkData, blocks: &BlockRegistry) {
    let s = column.step();
    if s > CHUNK_SIZE {
        return;
    }
    let n = 1i64 << column.level;
    let e = edit.pos.0;
    if I64Vec3::new(e.x.div_euclid(n), e.y.div_euclid(n), e.z.div_euclid(n)) != data.pos.0 {
        return;
    }
    let per = CHUNK_SIZE / s;
    // Where the edited chunk's cells start in the far chunk.
    let base = I64Vec3::new(e.x.rem_euclid(n), e.y.rem_euclid(n), e.z.rem_euclid(n)) * per;
    let full = crate::FULL_VOLUME as f32;
    // What each occupant list fills, by display block.
    let weights: Vec<Vec<(crate::BlockId, f32)>> = edit
        .lists
        .iter()
        .map(|list| {
            list.iter()
                .filter(|o| !blocks.is_hidden(o.block))
                .map(|o| {
                    let shown = blocks.made_of(o.block).map_or(o.block, |m| blocks.material_block(m));
                    (shown, blocks.shape(o.block).volume() as f32 / full)
                })
                .collect()
        })
        .collect();
    let half = (s * s * s) as f32 / 2.0;
    let mut tally: Vec<(crate::BlockId, f32)> = Vec::new();
    for cz in 0..per {
        for cy in 0..per {
            for cx in 0..per {
                tally.clear();
                let mut total = 0.0;
                for z in 0..s {
                    for y in 0..s {
                        for x in 0..s {
                            let local = LocalPos::new((cx * s + x) as u8, (cy * s + y) as u8, (cz * s + z) as u8);
                            for &(block, w) in &weights[edit.cells.get(local) as usize] {
                                total += w;
                                match tally.iter_mut().find(|(b, _)| *b == block) {
                                    Some(t) => t.1 += w,
                                    None => tally.push((block, w)),
                                }
                            }
                        }
                    }
                }
                let cell = LocalPos::new((base.x + cx) as u8, (base.y + cy) as u8, (base.z + cz) as u8);
                let entry = if total > half {
                    let block = tally.iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|t| t.0).expect("something filled");
                    list_for(data, block)
                } else {
                    0
                };
                data.cells.set(cell, entry);
            }
        }
    }
}

/// The entry in `data`'s occupant lists holding just `block`, added if it's new.
fn list_for(data: &mut ChunkData, block: crate::BlockId) -> u16 {
    let want = [Occupant::new(block)];
    match data.lists.iter().position(|l| l.as_slice() == want) {
        Some(i) => i as u16,
        None => {
            data.lists.push(want.to_vec());
            (data.lists.len() - 1) as u16
        }
    }
}

/// Each cell's entry, for tests.
#[cfg(test)]
fn layer_entries(layer: &crate::Layer<u16>) -> Vec<u16> {
    (0..api::CHUNK_VOLUME).map(|i| layer.at(i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::Vec3;

    fn at(x: f32, y: f32) -> WorldPos {
        WorldPos::new(I64Vec3::new(x.floor() as i64, y.floor() as i64, 0), Vec3::new(x.fract(), y.fract(), 0.0))
    }

    #[test]
    fn columns_nest_by_halves() {
        let c = FarColumn { terrain: 0, level: 2, x: -1, y: 3 };
        assert_eq!(c.span(), 128);
        for child in c.children() {
            assert_eq!(child.level, 1);
            assert_eq!(child.parent(), c);
        }
        assert_eq!(FarColumn::holding(0, 2, -1, 13), FarColumn { terrain: 0, level: 2, x: -1, y: 3 });
        assert_eq!(c.under(), (-4..0, 12..16));
        // Rows -8..8 at level 2 are -2..2; uneven ones round outward.
        assert_eq!(c.rows(&(-8..8)), -2..2);
        assert_eq!(c.rows(&(-3..2)), -1..1);
        let zs: Vec<i64> = c.chunks(&(-8..8)).map(|p| p.0.z).collect();
        assert_eq!(zs, [1, 0, -1, -2], "top first");
    }

    #[test]
    fn nearest_is_zero_inside_and_grows_outside() {
        let c = FarColumn { terrain: 0, level: 1, x: 0, y: 0 };
        assert_eq!(c.nearest(at(10.0, 60.0)), 0.0);
        assert!((c.nearest(at(74.0, 10.0)) - 10.0).abs() < 1e-4);
        assert!((c.nearest(at(-3.0, -4.0)) - 5.0).abs() < 1e-4);
    }

    #[test]
    fn the_top_level_reaches_the_far_distance() {
        let reach = Reach { detail: 256.0, far: 2048.0, level_reach: 2.0 };
        assert_eq!(reach.top(), 4);
        assert!(reach.split_distance(4) >= reach.far);
        assert_eq!(Reach { detail: 256.0, far: 200.0, level_reach: 2.0 }.top(), 1);
        assert_eq!(Reach { detail: 1.0, far: 1.0e9, level_reach: 2.0 }.top(), MAX_LEVEL);
    }

    #[test]
    fn the_column_the_player_is_in_splits_however_short_full_detail_is() {
        let reach = Reach { detail: 8.0, far: 2048.0, level_reach: 2.0 };
        // At a corner of a level-4 column, 362 m from its middle.
        let pos = at(0.5, 0.5);
        let mut c = FarColumn { terrain: 0, level: reach.top(), x: 0, y: 0 };
        while c.level > 0 {
            assert!(reach.splits(c, pos), "{c:?}");
            c = c.children()[0];
        }
    }

    /// The level-0 columns the picked columns cover, each once at most.
    fn coverage(picked: &[FarColumn]) -> HashMap<(i64, i64), usize> {
        let mut seen = HashMap::new();
        for c in picked {
            let (xs, ys) = c.under();
            for y in ys {
                for x in xs.clone() {
                    *seen.entry((x, y)).or_insert(0) += 1;
                }
            }
        }
        seen
    }

    #[test]
    fn with_everything_ready_each_spot_is_drawn_once_finer_nearer() {
        let reach = Reach { detail: 256.0, far: 2048.0, level_reach: 2.0 };
        for pos in [at(16.0, 16.0), at(-700.5, 333.25), at(5000.0, -12000.0)] {
            let picked = reach.pick(0, pos, &|_| true);
            let seen = coverage(&picked);
            assert!(seen.values().all(|&n| n == 1), "nothing drawn twice at {pos:?}");
            // Everything within the far distance is covered.
            let c = pos.boxel().chunk().0;
            for dy in -60..=60 {
                for dx in -60..=60 {
                    let col = FarColumn { terrain: 0, level: 0, x: c.x + dx, y: c.y + dy };
                    if col.nearest(pos) < reach.far - 1024.0 {
                        assert!(seen.contains_key(&(col.x, col.y)), "{col:?} covered from {pos:?}");
                    }
                }
            }
            // Full detail underfoot, coarser out at the edge.
            assert!(picked.iter().any(|p| p.level == 0 && p.distance(pos) < 40.0));
            for p in &picked {
                if p.level == 0 {
                    assert!(p.distance(pos) < reach.detail + 50.0, "{p:?}");
                }
            }
            // Far columns the server builds are all those drawn, at level 1 and up.
            let wanted: HashSet<FarColumn> = reach.wanted(0, pos).into_iter().collect();
            assert!(picked.iter().filter(|p| p.level > 0).all(|p| wanted.contains(p)));
        }
    }

    #[test]
    fn a_column_splits_only_once_its_children_are_ready() {
        let reach = Reach { detail: 256.0, far: 2048.0, level_reach: 2.0 };
        let pos = at(16.0, 16.0);
        // Everything but one ordinary column near the player.
        let missing = FarColumn { terrain: 0, level: 0, x: 1, y: 0 };
        let picked = reach.pick(0, pos, &|c| c != missing);
        let parent = missing.parent();
        assert!(picked.contains(&parent), "its level-1 column drawn instead");
        assert!(!picked.iter().any(|p| p.level == 0 && p.parent() == parent));
        assert!(coverage(&picked).values().all(|&n| n == 1));
        // Nothing far ready: the ordinary columns that are, with holes elsewhere.
        let picked = reach.pick(0, pos, &|c| c.level == 0);
        assert!(!picked.is_empty() && picked.iter().all(|p| p.level == 0));
        assert!(coverage(&picked).values().all(|&n| n == 1));
    }

    #[test]
    fn widened_keeps_what_was_wanted() {
        let reach = Reach { detail: 256.0, far: 2048.0, level_reach: 2.0 };
        let wide = reach.widened();
        for pos in [at(16.0, 16.0), at(400.0, -90.0)] {
            for c in reach.wanted(0, pos) {
                assert!(wide.reaches(c, pos) && Far::kept(&wide, c, pos), "{c:?}");
            }
        }
    }

    #[test]
    fn shrinking_an_edit_fills_cells_mostly_filled() {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(api::BlockDef::new("t:stone", true, [0; 3])).unwrap();
        // A far chunk at level 1 (cells 2 boxels a side), empty.
        let column = FarColumn { terrain: 0, level: 1, x: 0, y: 0 };
        let mut data = ChunkData {
            pos: ChunkPos(I64Vec3::ZERO),
            lists: vec![Vec::new()],
            cells: crate::Layer::Uniform(0),
            light: crate::Layer::Uniform(crate::Light::SKY),
            records: Vec::new(),
        };
        // An edited chunk at ordinary chunk (1, 0, 0): stone in its lowest 5 boxels.
        let mut edit = ChunkData {
            pos: ChunkPos(I64Vec3::new(1, 0, 0)),
            lists: vec![Vec::new(), vec![Occupant::new(stone)]],
            cells: crate::Layer::Uniform(0),
            light: crate::Layer::Uniform(crate::Light::SKY),
            records: Vec::new(),
        };
        for i in 0..api::CHUNK_VOLUME {
            if LocalPos::from_index(i).z < 5 {
                edit.cells.set_at(i, 1);
            }
        }
        shrink_into(&mut data, column, &edit, &blocks);
        let cells = layer_entries(&data.cells);
        let filled = |x: u8, y: u8, z: u8| cells[LocalPos::new(x, y, z).index()] != 0;
        // It lands in cells 16..32 in X; boxels 0..4 fill cells 0 and 1 whole, cell 2 half
        // (boxel 4 of 4..6): not more than half, so empty.
        assert!(filled(16, 0, 0) && filled(31, 15, 1));
        assert!(!filled(16, 0, 2));
        assert!(!filled(15, 0, 0), "west of it is untouched");
        let entry = cells[LocalPos::new(16, 0, 0).index()];
        assert_eq!(data.lists[entry as usize], [Occupant::new(stone)]);
    }
}
