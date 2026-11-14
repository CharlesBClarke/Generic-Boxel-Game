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
//! Levels split by size on screen: where a level's cells would look wider than a pixel
//! target (see [`Reach`]).
//!
//! A far cell is solid or empty, of one main material, with a second one and its share
//! beside it (a [`Mix`]), found by sampling the generator a level finer, so a hillside of
//! grass and rock is drawn as their mix. Builds shrink into every level, and a cell holds
//! what was built in it if that fills a boxel-thick plane across it ([`Reach::hold`]).
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

/// How far the view reaches and how far terrain is built: full detail out to `detail` meters,
/// coarser and coarser out to `far`. A level's cells are kept no wider on screen than
/// `pixels`: a level-*k* column splits once its cells, 2^*k* meters a side, would look wider
/// than that from the player, seen at `screen` pixels for a meter one meter away (the
/// client's window height and field of view, see [`Reach::screen_scale`]). So switching
/// levels changes about the same few pixels everywhere, and `pixels` is the quality slider:
/// lower is finer, for more triangles.
///
/// `hold` and `mix` say how cells are built: see [`Reach::hold`] and [`Reach::mix`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reach {
    pub detail: f32,
    pub far: f32,
    pub pixels: f32,
    pub screen: f32,
    /// A cell under edits stays solid, however empty, if what was built in it (boxels that
    /// differ from what the generator makes there) fills `hold` boxel-thick planes across it:
    /// at 1 a one-boxel wall or floor right through the cell keeps it, a lone pillar doesn't.
    /// 0 keeps only cells more than half filled, as every other cell is.
    pub hold: f32,
    /// How many levels finer the generator is sampled to find what a cell is made of besides
    /// its main material (see [`Mix`]): 0 none, up to [`MAX_MIX`].
    pub mix: u8,
}

impl Default for Reach {
    fn default() -> Self {
        Self { detail: 256.0, far: 2048.0, pixels: DEFAULT_PIXELS, screen: Reach::screen_scale(1080.0, 80.0), hold: 1.0, mix: 1 }
    }
}

/// [`Reach::pixels`] when nothing says otherwise. At 8, far terrain draws about as many
/// triangles as it did when levels reached 1.6 times as far as the one before (2026-11-14).
pub const DEFAULT_PIXELS: f32 = 8.0;

/// The range [`Reach::pixels`] is kept in.
pub const PIXELS: std::ops::RangeInclusive<f32> = 1.0..=64.0;

/// The most [`Reach::mix`] goes to. A level finer makes far terrain about four times as slow
/// to build; two levels finer, sixteen (2026-11-14), for little to see.
pub const MAX_MIX: u8 = 1;

impl Reach {
    /// Pixels a meter-wide thing one meter away covers, for a view `height` pixels tall with a
    /// vertical field of view of `fov` degrees.
    pub fn screen_scale(height: f32, fov: f32) -> f32 {
        height.max(1.0) / (2.0 * (fov.clamp(1.0, 179.0).to_radians() * 0.5).tan())
    }

    /// The coarsest level used: the first whose columns never split at the far distance.
    pub fn top(&self) -> u8 {
        let mut level = 1u8;
        while level < MAX_LEVEL && self.split_distance(level) < self.far {
            level += 1;
        }
        level
    }

    /// Meters within which a level-`level` column's middle must be for it to split: level 1
    /// at the full-detail distance, coarser ones where their cells would look `pixels` wide,
    /// never nearer than that; and never less than the column's width, so the column the
    /// player is in always splits however short full detail is.
    pub fn split_distance(&self, level: u8) -> f32 {
        if level == 0 {
            return 0.0;
        }
        let detail = self.detail.max(1.0);
        let pixels = self.pixels.clamp(*PIXELS.start(), *PIXELS.end());
        let by_size = if level == 1 { 0.0 } else { (1u64 << level) as f32 * self.screen.max(1.0) / pixels };
        by_size.max(detail).max((CHUNK_SIZE << level) as f32)
    }

    /// Whether `other` builds cells the same way: the same hold and mix.
    pub fn builds_like(&self, other: &Reach) -> bool {
        self.hold == other.hold && self.mix == other.mix
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
        Reach { detail: self.detail * 1.25 + CHUNK_SIZE as f32, far: self.far + 2.0 * CHUNK_SIZE as f32, ..*self }
    }
}

/// A far column's chunks, as sent: only those with something in them, and not solid all
/// through under one whose lowest layer is solid; and each one's [`Mix`], in the same order.
#[derive(Clone, Debug, PartialEq)]
pub struct FarChunks {
    pub column: FarColumn,
    pub chunks: Vec<ChunkData>,
    pub mixes: Vec<Mix>,
}

/// What a far chunk's cells are made of besides their main material (the one in
/// [`ChunkData::cells`]): for each cell, its second material and how much of the cell's solid
/// part that makes up, so a cell of grass over stone is drawn as their mix rather than as
/// grass. Cells with no second material have entry 0.
#[derive(Clone, Debug, PartialEq)]
pub struct Mix {
    /// Each second material with its share, in sixteenths (1 to 16); entry 0 is none.
    pub kinds: Vec<(crate::BlockId, u8)>,
    pub cells: crate::Layer<u16>,
}

impl Default for Mix {
    fn default() -> Self {
        Self { kinds: vec![(crate::BlockId::default(), 0)], cells: crate::Layer::Uniform(0) }
    }
}

impl Mix {
    /// The second material at cell `index` and its share, from 0 to 1, if it has one.
    pub fn at(&self, index: usize) -> Option<(crate::BlockId, f32)> {
        let entry = self.cells.at(index) as usize;
        (entry != 0).then(|| self.kinds.get(entry)).flatten().map(|&(b, w)| (b, w as f32 / 16.0))
    }

    /// Sets cell `index`'s second material to `block` making up `share` of it (0 to 1), or none.
    pub fn set(&mut self, index: usize, second: Option<(crate::BlockId, f32)>) {
        let entry = match second {
            Some((block, share)) => {
                let w = (share.clamp(0.0, 1.0) * 16.0).round() as u8;
                if w == 0 {
                    0
                } else {
                    match self.kinds.iter().position(|&k| k == (block, w)) {
                        Some(i) => i as u16,
                        None => {
                            self.kinds.push((block, w));
                            (self.kinds.len() - 1) as u16
                        }
                    }
                }
            }
            None => 0,
        };
        if entry != 0 || self.cells.at(index) != 0 {
            self.cells.set_at(index, entry);
        }
    }
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

/// A far column's chunks and their mixes, as built.
type Column = (Vec<ChunkData>, Vec<Mix>);

/// A far column built on a terrain thread.
struct Built {
    epoch: u64,
    serial: u64,
    column: FarColumn,
    /// `None` if the generator can't fill coarsely.
    chunks: Result<Option<Column>, ApiError>,
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
    /// [`Reach::hold`] and [`Reach::mix`].
    hold: f32,
    mix: u8,
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
    wanted_for: Option<((i64, i64), Reach, u64)>,
    /// How the loaded columns were built: their [`Reach::hold`] and [`Reach::mix`].
    built_like: Option<Reach>,
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
            built_like: None,
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
        // Built another way than asked now: every column is built again, the old ones drawn
        // until then.
        if self.built_like.is_some_and(|r| !r.builds_like(&self.reach)) {
            for &c in self.loaded.iter().chain(self.in_flight.keys()) {
                self.stale.insert(c, self.serial);
            }
        }
        self.built_like = Some(self.reach);
        let key = ((chunk.x, chunk.y), self.reach, self.epoch);
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
            let (chunks, mixes) = match built.chunks {
                Ok(Some(built)) => built,
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
            self.loaded_now.push(FarChunks { column: built.column, chunks, mixes });
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
                hold: self.reach.hold,
                mix: self.reach.mix,
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

/// An edited chunk under a far column, and what the generator makes there, to tell what was
/// built (`None` where nothing is told apart: all of it counts as built).
struct Edit {
    data: ChunkData,
    generated: Option<ChunkData>,
}

/// Builds a far column: the generator's coarse fill, cleared where terrain is kept away, with
/// the edited chunks under it shrunk in, and each kept chunk's [`Mix`]. `None` if the
/// generator can't fill coarsely.
fn build(job: &FarJob) -> Result<Option<Column>, ApiError> {
    let generator = job.generator.read().map_err(|_| ApiError::InvalidDef("a terrain generator panicked".into()))?;
    let column = job.column;
    let step = column.step();
    let mut edited: Vec<ChunkData> = job.loaded.clone();
    for &pos in &job.stored {
        if let Some(bytes) = job.store.read(pos).map_err(ApiError::Storage)? {
            let saved = SavedChunk::from_bytes(&bytes).map_err(|e| ApiError::Storage(format!("chunk {:?}: {e}", pos.0.to_array())))?;
            let mut report = crate::LoadReport::default();
            edited.push(crate::simulation::save::unname_chunk(&job.blocks, &saved, &mut report));
        }
    }
    let mut fill = ChunkFill::default();
    // What the generator makes under each edit, only needed to hold built cells.
    let mut edits = Vec::with_capacity(edited.len());
    for data in edited {
        let generated = if job.hold > 0.0 {
            let p = data.pos.0;
            let (n, (xs, ys)) = (1i64 << column.level, column.under());
            let (bx, by) = (p.x - xs.start, p.y - ys.start);
            let cleared = !job.cleared.is_empty() && xs.contains(&p.x) && ys.contains(&p.y) && job.cleared[(bx + by * n) as usize];
            fill.clear();
            if !cleared {
                generator.fill(p * CHUNK_SIZE - job.center, &mut fill);
            }
            Some(crate::terrain::to_chunk(data.pos, &fill, &job.blocks)?)
        } else {
            None
        };
        edits.push(Edit { data, generated });
    }
    // The ordinary chunk rows it spans: from under the lowest of the generator's tops to over
    // the highest, a cell to spare either way, and wherever an edit is.
    let mut tops = [api::NO_TOP; api::TOPS_LEN];
    generator.tops(column.x * column.span() - job.center.x, column.y * column.span() - job.center.y, step as u32, &mut tops);
    let known = || tops.iter().filter(|&&t| t != api::NO_TOP).map(|&t| t + job.center.z);
    let row = |z: i64| z.div_euclid(CHUNK_SIZE);
    let edited = || edits.iter().map(|e| e.data.pos.0.z);
    let low = known().min().map(|z| row(z - step)).into_iter().chain(edited()).min();
    let high = known().max().map(|z| row(z + step)).into_iter().chain(edited()).max();
    let (Some(low), Some(high)) = (low, high) else { return Ok(Some((Vec::new(), Vec::new()))) };
    let rows = low..high + 1;
    let mut out = Vec::new();
    let mut mixes = Vec::new();
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
        let mut mix = Mix::default();
        let touched = shrink_edits(&mut data, &mut mix, column, &edits, &job.blocks, job.hold);
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
        if job.mix > 0 && uniform.is_none() && !mix_finer(&**generator, &mut data, &mut mix, column, (job.center, job.mix, &job.blocks), &touched, &mut fill)? {
            return Ok(None);
        }
        mix.cells.compact();
        out.push(data);
        mixes.push(mix);
    }
    Ok(Some((out, mixes)))
}

/// Finds what each solid cell of `data` is made of by filling the generator `depth` levels
/// finer over it (see [`Reach::mix`]), the terrain's center at `center`: the material most of the finer cells are becomes the cell's own, the
/// next its second (see [`Mix`]). Cells `touched` by edits are left as the edits made them,
/// as are cleared ones. `false` if the generator can't fill coarsely.
fn mix_finer(
    generator: &dyn api::Generator,
    data: &mut ChunkData,
    mix: &mut Mix,
    column: FarColumn,
    (center, depth, blocks): (I64Vec3, u8, &BlockRegistry),
    touched: &[bool],
    fill: &mut ChunkFill,
) -> Result<bool, ApiError> {
    let depth = depth.min(column.level) as i64;
    let per = 1i64 << depth;
    let fine = column.step() >> depth;
    let size = CHUNK_SIZE;
    // Up to four materials and how many finer cells each fills, per cell.
    let mut tally: Vec<[(crate::BlockId, u16); 4]> = vec![[(crate::BlockId::default(), 0); 4]; api::CHUNK_VOLUME];
    // Only cells that can show need a mix: solid ones with air beside them, or under the
    // chunk's top (what's over it isn't known here). A cell on the column's sides counts by
    // its neighbors inside; the sides are drawn, but neighboring columns hide them.
    let filled = |x: i64, y: i64, z: i64| data.cells.get(LocalPos::new(x as u8, y as u8, z as u8)) != 0;
    let shows = |p: LocalPos| {
        let (x, y, z) = (p.x as i64, p.y as i64, p.z as i64);
        z == size - 1
            || !filled(x, y, z + 1)
            || (z > 0 && !filled(x, y, z - 1))
            || (x > 0 && !filled(x - 1, y, z))
            || (x < size - 1 && !filled(x + 1, y, z))
            || (y > 0 && !filled(x, y - 1, z))
            || (y < size - 1 && !filled(x, y + 1, z))
    };
    let solid: Vec<bool> =
        (0..api::CHUNK_VOLUME).map(|i| data.cells.at(i) != 0 && !touched[i] && shows(LocalPos::from_index(i))).collect();
    for cz in 0..per {
        for cy in 0..per {
            for cx in 0..per {
                let sub = I64Vec3::new(cx, cy, cz);
                // Skip finer chunks under no solid cell.
                let base = sub * (size / per);
                let any = (0..size / per).any(|z| {
                    (0..size / per).any(|y| {
                        (0..size / per).any(|x| solid[LocalPos::new((base.x + x) as u8, (base.y + y) as u8, (base.z + z) as u8).index()])
                    })
                });
                if !any {
                    continue;
                }
                fill.clear();
                let origin = data.pos.0 * column.span() + sub * size * fine;
                if !generator.fill_coarse(origin - center, fine as u32, fill) {
                    return Ok(false);
                }
                let ids: Vec<Option<crate::BlockId>> = std::iter::once(None)
                    .chain(fill.palette.iter().map(|name| blocks.id(name)))
                    .collect();
                for i in 0..api::CHUNK_VOLUME {
                    let Some(block) = ids.get(fill.cells[i] as usize).copied().flatten() else { continue };
                    let p = LocalPos::from_index(i);
                    let cell = LocalPos::new(
                        (base.x + p.x as i64 / per) as u8,
                        (base.y + p.y as i64 / per) as u8,
                        (base.z + p.z as i64 / per) as u8,
                    )
                    .index();
                    if !solid[cell] {
                        continue;
                    }
                    let slots = &mut tally[cell];
                    if let Some(slot) = slots.iter_mut().find(|s| s.1 > 0 && s.0 == block) {
                        slot.1 += 1;
                    } else if let Some(slot) = slots.iter_mut().find(|s| s.1 == 0) {
                        *slot = (block, 1);
                    }
                }
            }
        }
    }
    for (i, slots) in tally.iter_mut().enumerate() {
        if !solid[i] {
            continue;
        }
        slots.sort_by_key(|s| std::cmp::Reverse(s.1));
        let [(first, a), (second, b), ..] = *slots;
        if a == 0 {
            continue;
        }
        if data.lists[data.cells.at(i) as usize] != [Occupant::new(first)] {
            let entry = list_for(data, first);
            data.cells.set_at(i, entry);
        }
        mix.set(i, (b > 0).then(|| (second, b as f32 / (a + b) as f32)));
    }
    Ok(true)
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

/// What the edits put in one far cell, in boxels.
#[derive(Default)]
struct Tally {
    /// Boxels of the cell inside an edited chunk.
    covered: u32,
    total: f32,
    /// What differs from what the generator makes there.
    built: f32,
    kinds: Vec<(crate::BlockId, f32)>,
    built_kinds: Vec<(crate::BlockId, f32)>,
}

fn add(list: &mut Vec<(crate::BlockId, f32)>, block: crate::BlockId, w: f32) {
    match list.iter_mut().find(|(b, _)| *b == block) {
        Some(t) => t.1 += w,
        None => list.push((block, w)),
    }
}

/// The material filling most, if any.
fn top(list: &[(crate::BlockId, f32)]) -> Option<(crate::BlockId, f32)> {
    list.iter().copied().max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
}

/// Writes the cells of `data` (a far chunk of `column`'s level) that `edits` lie under,
/// shrunk, and their mix: a cell is solid if more than half filled, or if what was built in it
/// fills `hold` planes across it (see [`Reach::hold`]), and takes the material filling most
/// of it, or of what was built; else it's emptied. A cell wider than a chunk counts what's
/// outside the edited chunks as it was generated. Returns which cells the edits touched.
fn shrink_edits(data: &mut ChunkData, mix: &mut Mix, column: FarColumn, edits: &[Edit], blocks: &BlockRegistry, hold: f32) -> Vec<bool> {
    let mut touched = vec![false; api::CHUNK_VOLUME];
    let n = 1i64 << column.level;
    let s = column.step();
    let mut tallies: HashMap<usize, Tally> = HashMap::new();
    let full = crate::FULL_VOLUME as f32;
    for edit in edits {
        let e = edit.data.pos.0;
        if I64Vec3::new(e.x.div_euclid(n), e.y.div_euclid(n), e.z.div_euclid(n)) != data.pos.0 {
            continue;
        }
        // Where the edited chunk starts in the far chunk, in boxels.
        let base = I64Vec3::new(e.x.rem_euclid(n), e.y.rem_euclid(n), e.z.rem_euclid(n)) * CHUNK_SIZE;
        // What each occupant list fills, by display block.
        let weights: Vec<Vec<(crate::BlockId, f32)>> = edit
            .data
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
        for i in 0..api::CHUNK_VOLUME {
            let p = LocalPos::from_index(i);
            let cell = LocalPos::new(
                ((base.x + p.x as i64) / s) as u8,
                ((base.y + p.y as i64) / s) as u8,
                ((base.z + p.z as i64) / s) as u8,
            )
            .index();
            let t = tallies.entry(cell).or_default();
            t.covered += 1;
            let entry = edit.data.cells.at(i) as usize;
            let built = edit.generated.as_ref().is_none_or(|g| g.lists[g.cells.at(i) as usize] != edit.data.lists[entry]);
            for &(block, w) in &weights[entry] {
                t.total += w;
                add(&mut t.kinds, block, w);
                if built {
                    t.built += w;
                    add(&mut t.built_kinds, block, w);
                }
            }
        }
    }
    let volume = (s * s * s) as f32;
    let planes = hold * (s * s) as f32;
    let mut cells: Vec<(usize, Tally)> = tallies.into_iter().collect();
    cells.sort_by_key(|c| c.0);
    for (cell, mut t) in cells {
        touched[cell] = true;
        let outside = volume - t.covered as f32;
        if outside > 0.0
            && let Some(o) = data.lists[data.cells.at(cell) as usize].first()
        {
            t.total += outside;
            add(&mut t.kinds, o.block, outside);
        }
        let main = if t.total > volume / 2.0 {
            top(&t.kinds)
        } else if hold > 0.0 && t.built > 0.0 && t.built >= planes {
            top(&t.built_kinds)
        } else {
            None
        };
        match main {
            Some((block, _)) => {
                let entry = list_for(data, block);
                data.cells.set_at(cell, entry);
                let w = t.kinds.iter().find(|k| k.0 == block).map_or(0.0, |k| k.1);
                let rest: Vec<(crate::BlockId, f32)> = t.kinds.iter().copied().filter(|k| k.0 != block).collect();
                mix.set(cell, top(&rest).map(|(b, w2)| (b, w2 / (w + w2))));
            }
            None => {
                data.cells.set_at(cell, 0);
                mix.set(cell, None);
            }
        }
    }
    touched
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
    fn levels_split_where_their_cells_would_look_wider_than_the_pixel_target() {
        // 1080 pixels tall, 90° up and down: a meter one meter away is 540 pixels.
        let screen = Reach::screen_scale(1080.0, 90.0);
        assert!((screen - 540.0).abs() < 0.01);
        let reach = Reach { detail: 300.0, far: 4096.0, pixels: 6.0, screen, hold: 1.0, mix: 1 };
        // Level 1 meets full detail; level 3's 8 m cells are 6 pixels at 720 m.
        assert_eq!(reach.split_distance(1), 300.0);
        assert!((reach.split_distance(3) - 720.0).abs() < 0.01);
        // Each level twice as far as the one before: a cell's always about as wide where its
        // level starts.
        for level in 3..MAX_LEVEL {
            assert!((reach.split_distance(level + 1) / reach.split_distance(level) - 2.0).abs() < 1e-4);
        }
        // Never nearer than full detail: a coarse target skips fine levels.
        let coarse = Reach { pixels: 40.0, ..reach };
        assert_eq!(coarse.split_distance(2), 300.0);
        // A smaller pixel target, or a taller window, splits farther.
        assert!(Reach { pixels: 3.0, ..reach }.split_distance(4) > reach.split_distance(4));
        assert!(Reach { screen: screen * 2.0, ..reach }.split_distance(4) > reach.split_distance(4));
    }

    #[test]
    fn nearest_is_zero_inside_and_grows_outside() {
        let c = FarColumn { terrain: 0, level: 1, x: 0, y: 0 };
        assert_eq!(c.nearest(at(10.0, 60.0)), 0.0);
        assert!((c.nearest(at(74.0, 10.0)) - 10.0).abs() < 1e-4);
        assert!((c.nearest(at(-3.0, -4.0)) - 5.0).abs() < 1e-4);
    }

    /// Full detail to `detail` meters and far terrain to `far`, otherwise as by default.
    fn reach(detail: f32, far: f32) -> Reach {
        Reach { detail, far, ..Reach::default() }
    }

    #[test]
    fn the_top_level_reaches_the_far_distance() {
        let r = reach(256.0, 2048.0);
        let top = r.top();
        assert!(r.split_distance(top) >= r.far);
        assert!(r.split_distance(top - 1) < r.far);
        assert_eq!(reach(256.0, 200.0).top(), 1);
        assert_eq!(reach(1.0, 1.0e9).top(), MAX_LEVEL);
    }

    #[test]
    fn the_column_the_player_is_in_splits_however_short_full_detail_is() {
        let reach = reach(8.0, 2048.0);
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
        let reach = reach(256.0, 2048.0);
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
        let reach = reach(256.0, 2048.0);
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
        let reach = reach(256.0, 2048.0);
        let wide = reach.widened();
        for pos in [at(16.0, 16.0), at(400.0, -90.0)] {
            for c in reach.wanted(0, pos) {
                assert!(wide.reaches(c, pos) && Far::kept(&wide, c, pos), "{c:?}");
            }
        }
    }

    /// A chunk at `pos` of `lists`, each cell's entry from `entry`.
    fn chunk(pos: I64Vec3, lists: Vec<Vec<Occupant>>, entry: impl Fn(LocalPos) -> u16) -> ChunkData {
        let mut data = ChunkData {
            pos: ChunkPos(pos),
            lists,
            cells: crate::Layer::Uniform(0),
            light: crate::Layer::Uniform(crate::Light::SKY),
            records: Vec::new(),
        };
        for i in 0..api::CHUNK_VOLUME {
            let e = entry(LocalPos::from_index(i));
            if e != 0 {
                data.cells.set_at(i, e);
            }
        }
        data
    }

    fn stone_and_grass() -> (BlockRegistry, crate::BlockId, crate::BlockId) {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(api::BlockDef::new("t:stone", true, [0; 3])).unwrap();
        let grass = blocks.register(api::BlockDef::new("t:grass", true, [0; 3])).unwrap();
        (blocks, stone, grass)
    }

    /// Whether cell `(x, y, z)` of `data` is filled, and with what.
    fn cell(data: &ChunkData, x: u8, y: u8, z: u8) -> Option<crate::BlockId> {
        data.lists[data.cells.get(LocalPos::new(x, y, z)) as usize].first().map(|o| o.block)
    }

    #[test]
    fn shrinking_an_edit_fills_cells_mostly_filled() {
        let (blocks, stone, _) = stone_and_grass();
        // A far chunk at level 1 (cells 2 boxels a side), empty.
        let column = FarColumn { terrain: 0, level: 1, x: 0, y: 0 };
        let mut data = chunk(I64Vec3::ZERO, vec![Vec::new()], |_| 0);
        // An edited chunk at ordinary chunk (1, 0, 0): stone in its lowest 5 boxels.
        let edit = chunk(I64Vec3::new(1, 0, 0), vec![Vec::new(), vec![Occupant::new(stone)]], |p| (p.z < 5) as u16);
        let mut mix = Mix::default();
        shrink_edits(&mut data, &mut mix, column, &[Edit { data: edit, generated: None }], &blocks, 0.0);
        // It lands in cells 16..32 in X; boxels 0..4 fill cells 0 and 1 whole, cell 2 half
        // (boxel 4 of 4..6): not more than half, so empty.
        assert_eq!(cell(&data, 16, 0, 0), Some(stone));
        assert_eq!(cell(&data, 31, 15, 1), Some(stone));
        assert_eq!(cell(&data, 16, 0, 2), None);
        assert_eq!(cell(&data, 15, 0, 0), None, "west of it is untouched");
    }

    /// An ordinary chunk at `pos` with a one-boxel wall of stone across it at `x` 5, `rows`
    /// high (from its floor), on empty generated ground.
    fn wall(pos: I64Vec3, stone: crate::BlockId, rows: u8) -> Edit {
        let data = chunk(pos, vec![Vec::new(), vec![Occupant::new(stone)]], |p| (p.x == 5 && p.z < rows) as u16);
        let generated = Some(chunk(pos, vec![Vec::new()], |_| 0));
        Edit { data, generated }
    }

    /// Shrinks `edits` into the far chunk at the origin of `level`, empty, holding `hold`.
    fn shrunk(level: u8, edits: &[Edit], blocks: &BlockRegistry, hold: f32) -> ChunkData {
        let column = FarColumn { terrain: 0, level, x: 0, y: 0 };
        let mut data = chunk(I64Vec3::ZERO, vec![Vec::new()], |_| 0);
        let mut mix = Mix::default();
        shrink_edits(&mut data, &mut mix, column, edits, blocks, hold);
        data
    }

    #[test]
    fn a_one_boxel_wall_stays_at_every_level() {
        let (blocks, stone, _) = stone_and_grass();
        for level in 1..=5 {
            let data = shrunk(level, &[wall(I64Vec3::ZERO, stone, 32)], &blocks, 1.0);
            let x = (5 >> level) as u8;
            assert_eq!(cell(&data, x, 0, 0), Some(stone), "level {level}");
            assert_eq!(cell(&data, x + 1, 0, 0), None, "level {level}: only the wall's cells");
            // Held off, a thin wall goes.
            assert_eq!(cell(&shrunk(level, &[wall(I64Vec3::ZERO, stone, 32)], &blocks, 0.0), x, 0, 0), None, "level {level}");
        }
        // At level 6 a cell is 2 chunks a side: a wall through all of it, made of four
        // edited chunks, holds it; a wall through a quarter of it doesn't.
        let four: Vec<Edit> = [(0, 0), (1, 0), (0, 1), (1, 1)].iter().map(|&(y, z)| wall(I64Vec3::new(0, y, z), stone, 32)).collect();
        assert_eq!(cell(&shrunk(6, &four, &blocks, 1.0), 0, 0, 0), Some(stone));
        assert_eq!(cell(&shrunk(6, &four[..1], &blocks, 1.0), 0, 0, 0), None);
    }

    #[test]
    fn a_lone_pillar_and_generated_ground_are_not_held() {
        let (blocks, stone, _) = stone_and_grass();
        let pos = I64Vec3::ZERO;
        // A pillar one boxel across, all the way up the chunk.
        let pillar = Edit {
            data: chunk(pos, vec![Vec::new(), vec![Occupant::new(stone)]], |p| (p.x == 5 && p.y == 5) as u16),
            generated: Some(chunk(pos, vec![Vec::new()], |_| 0)),
        };
        assert_eq!(cell(&shrunk(3, &[pillar], &blocks, 1.0), 0, 0, 0), None);
        // Ground 3 boxels deep that the generator made itself: not built, so cells 8 a side
        // stay empty under 3/8 of it, as they do where nothing was edited.
        let ground = |p: LocalPos| (p.z < 3) as u16;
        let lists = vec![Vec::new(), vec![Occupant::new(stone)]];
        let edit = Edit { data: chunk(pos, lists.clone(), ground), generated: Some(chunk(pos, lists, ground)) };
        assert_eq!(cell(&shrunk(3, &[edit], &blocks, 1.0), 0, 0, 0), None);
    }

    #[test]
    fn an_edited_cell_of_two_materials_keeps_both() {
        let (blocks, stone, grass) = stone_and_grass();
        // Stone in the lowest 5 boxels, grass the 3 over it: cells 8 a side are 5/8 stone,
        // 3/8 grass.
        let lists = vec![Vec::new(), vec![Occupant::new(stone)], vec![Occupant::new(grass)]];
        let edit = chunk(I64Vec3::ZERO, lists, |p| if p.z < 5 { 1 } else if p.z < 8 { 2 } else { 0 });
        let column = FarColumn { terrain: 0, level: 3, x: 0, y: 0 };
        let mut data = chunk(I64Vec3::ZERO, vec![Vec::new()], |_| 0);
        let mut mix = Mix::default();
        let touched = shrink_edits(&mut data, &mut mix, column, &[Edit { data: edit, generated: None }], &blocks, 1.0);
        assert_eq!(cell(&data, 0, 0, 0), Some(stone));
        let (second, share) = mix.at(LocalPos::new(0, 0, 0).index()).expect("a second material");
        assert_eq!(second, grass);
        assert!((share - 0.375).abs() < 1.0 / 32.0, "{share}");
        assert!(touched[LocalPos::new(3, 3, 0).index()] && !touched[LocalPos::new(4, 0, 0).index()]);
    }

    #[test]
    fn a_mix_keeps_shares_in_sixteenths_and_reuses_entries() {
        let (_, stone, grass) = stone_and_grass();
        let mut mix = Mix::default();
        mix.set(3, Some((grass, 0.5)));
        mix.set(9, Some((grass, 0.52)));
        mix.set(12, Some((stone, 0.01)));
        assert_eq!(mix.at(3), Some((grass, 0.5)));
        assert_eq!(mix.at(9), Some((grass, 0.5)), "rounded to the same sixteenth");
        assert_eq!(mix.at(12), None, "too little to show");
        assert_eq!(mix.kinds.len(), 2);
        mix.set(3, None);
        assert_eq!(mix.at(3), None);
    }

    /// Stone below z 0, with grass making up the top `grass` boxels of it.
    struct Layers {
        grass: i64,
    }

    impl api::Tunables for Layers {
        fn tunables(&self) -> Vec<api::Tunable> {
            Vec::new()
        }
        fn set_tunable(&mut self, _: &str, _: f32) -> bool {
            false
        }
    }

    impl api::Generator for Layers {
        fn fill(&self, origin: I64Vec3, out: &mut ChunkFill) {
            self.fill_coarse(origin, 1, out);
        }
        fn fill_coarse(&self, origin: I64Vec3, step: u32, out: &mut ChunkFill) -> bool {
            let s = step as i64;
            let (stone, grass) = (out.entry("t:stone"), out.entry("t:grass"));
            for i in 0..api::CHUNK_VOLUME {
                let p = LocalPos::from_index(i);
                // The cell's middle, sampled.
                let z = origin.z + p.z as i64 * s + s / 2;
                out.cells[i] = if z >= 0 { 0 } else if z >= -self.grass { grass } else { stone };
            }
            true
        }
        fn tops(&self, _: i64, _: i64, _: u32, out: &mut [i64]) {
            out.fill(-1);
        }
    }

    #[test]
    fn finer_cells_give_a_cell_its_mix() {
        use api::Generator;
        let (blocks, stone, grass) = stone_and_grass();
        let generator = Layers { grass: 2 };
        // Level 2, cells 4 a side: the cell from -4 to 0 samples stone at its middle (-2 is
        // grass's lowest, so grass by the sample), and at level 1 its halves are grass over
        // stone.
        let column = FarColumn { terrain: 0, level: 2, x: 0, y: 0 };
        let pos = ChunkPos(I64Vec3::new(0, 0, -1));
        let mut fill = ChunkFill::default();
        assert!(generator.fill_coarse(pos.0 * column.span(), 4, &mut fill));
        let mut data = crate::terrain::to_chunk(pos, &fill, &blocks).unwrap();
        let mut mix = Mix::default();
        let untouched = vec![false; api::CHUNK_VOLUME];
        assert!(mix_finer(&generator, &mut data, &mut mix, column, (I64Vec3::ZERO, 1, &blocks), &untouched, &mut fill).unwrap());
        let top = LocalPos::new(0, 0, 31).index();
        let main = data.lists[data.cells.at(top) as usize][0].block;
        let (second, share) = mix.at(top).expect("mixed");
        assert_eq!([main, second].iter().filter(|&&b| b == grass).count(), 1);
        assert_eq!([main, second].iter().filter(|&&b| b == stone).count(), 1);
        assert!((share - 0.5).abs() < 0.01, "{share}");
        // Deeper, all stone: no mix.
        let deep = LocalPos::new(0, 0, 20).index();
        assert_eq!(mix.at(deep), None);
        assert_eq!(data.lists[data.cells.at(deep) as usize][0].block, stone);
    }
}
