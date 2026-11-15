//! Far terrain: generated terrain drawn coarsely past full detail, so the view reaches far
//! without loading the world there.
//!
//! A far chunk ([`FarChunk`]) is a level-*k* chunk: a chunk's 32³ cells, each standing for
//! 2^*k* boxels a side, so the same [`ChunkData`] carries it; positions are in level-*k*
//! chunks (a level-*k* chunk at `p` spans boxels `p * 32 * 2^k` on). Level 0 is the ordinary
//! chunks [`crate::stream`] loads.
//!
//! Which level is drawn where is an octree over far chunks ([`Reach::pick`]): a far chunk
//! splits into its eight children of the level below when its middle is near enough in 3D,
//! and only once all eight can be drawn, so no far chunk lies over another and nothing goes
//! missing while levels load; ordinary chunks that are in are drawn all the same, the far
//! chunk still waiting left out where they are. Levels split by size on screen: where a level's cells would look wider
//! than a pixel target (see [`Reach`]). The server builds every far chunk the octree may want
//! around the player ([`Reach::wanted`]) on the terrain threads, from the generator's coarse
//! fill ([`api::Generator::fill_coarse`]), with the edited chunks in it shrunk in, and sends
//! them; the client picks what to draw. One over all the generator's tops with no edits in it
//! is known empty without filling.
//!
//! A far cell is solid or empty, of one main material, with a second one and its share
//! beside it (a [`Mix`]), found by sampling the generator a level finer, so a hillside of
//! grass and rock is drawn as their mix. Builds shrink into every level, and by default a
//! cell holds what was built in it however little that is ([`Reach::hold`]), so builds never
//! vanish far away: a one-boxel wall gets as thick as the cell instead.
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

/// The coarsest level: cells 64 boxels a side, far chunks 2 km across.
pub const MAX_LEVEL: u8 = 6;

/// One terrain's level-`level` chunk at `pos`, in level-`level` chunks. Level 0 is an
/// ordinary chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FarChunk {
    pub terrain: usize,
    pub level: u8,
    pub pos: I64Vec3,
}

impl Ord for FarChunk {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.terrain, self.level, self.pos.to_array()).cmp(&(other.terrain, other.level, other.pos.to_array()))
    }
}

impl PartialOrd for FarChunk {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl FarChunk {
    /// Boxels each cell stands for, a side.
    pub fn step(self) -> i64 {
        1 << self.level
    }

    /// Boxels across the far chunk.
    pub fn span(self) -> i64 {
        CHUNK_SIZE << self.level
    }

    /// The level-`level` far chunk holding the ordinary chunk at `chunk`.
    pub fn holding(terrain: usize, level: u8, chunk: ChunkPos) -> Self {
        let n = 1i64 << level;
        Self { terrain, level, pos: chunk.0.div_euclid(I64Vec3::splat(n)) }
    }

    /// The far chunk of the level above holding this one.
    pub fn parent(self) -> Self {
        Self { terrain: self.terrain, level: self.level + 1, pos: self.pos.div_euclid(I64Vec3::splat(2)) }
    }

    /// The eight far chunks of the level below that make this one up.
    pub fn children(self) -> [Self; 8] {
        let (base, level) = (self.pos * 2, self.level.saturating_sub(1));
        std::array::from_fn(|i| {
            let i = i as i64;
            Self { terrain: self.terrain, level, pos: base + I64Vec3::new(i & 1, (i >> 1) & 1, (i >> 2) & 1) }
        })
    }

    /// The six far chunks of its level that share a face with it.
    pub fn beside(self) -> [Self; 6] {
        let o = [I64Vec3::X, I64Vec3::NEG_X, I64Vec3::Y, I64Vec3::NEG_Y, I64Vec3::Z, I64Vec3::NEG_Z];
        o.map(|o| Self { pos: self.pos + o, ..self })
    }

    /// The ordinary chunks in it, as chunk X, Y and Z ranges.
    pub fn under(self) -> [Range<i64>; 3] {
        let n = 1i64 << self.level;
        [0, 1, 2].map(|k| self.pos[k] * n..(self.pos[k] + 1) * n)
    }

    /// Whether the ordinary chunk at `chunk` is in it.
    pub fn holds(self, chunk: ChunkPos) -> bool {
        Self::holding(self.terrain, self.level, chunk) == self
    }

    /// Meters from `pos` to the far chunk's middle.
    pub fn distance(self, pos: WorldPos) -> f32 {
        let span = self.span();
        let middle = self.pos * span + I64Vec3::splat(span / 2);
        ((middle - pos.cell()).as_vec3() - pos.offset()).length()
    }

    /// Meters from `pos` to the nearest point of the far chunk; 0 inside it.
    pub fn nearest(self, pos: WorldPos) -> f32 {
        let span = self.span() as f32;
        let rel = (pos.cell() - self.pos * self.span()).as_vec3() + pos.offset();
        let out = |v: f32| if v < 0.0 { -v } else if v > span { v - span } else { 0.0 };
        (out(rel.x).powi(2) + out(rel.y).powi(2) + out(rel.z).powi(2)).sqrt()
    }
}

/// How far the view reaches and how far terrain is built: full detail out to `detail` meters,
/// coarser and coarser out to `far`. A level's cells are kept no wider on screen than
/// `pixels`: a level-*k* far chunk splits once its cells, 2^*k* meters a side, would look
/// wider than that from the player, seen at `screen` pixels for a meter one meter away (the
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
    /// How a cell under edits is kept solid by what was built in it (boxels that differ from
    /// what the generator makes there), however empty it is otherwise: [`HOLD_OFF`],
    /// [`HOLD_PLANE`] or [`HOLD_ANY`].
    pub hold: u8,
    /// How many levels finer the generator is sampled to find what a cell is made of besides
    /// its main material (see [`Mix`]): 0 none, up to [`MAX_MIX`].
    pub mix: u8,
}

impl Default for Reach {
    fn default() -> Self {
        Self { detail: 256.0, far: 2048.0, pixels: DEFAULT_PIXELS, screen: Reach::screen_scale(1080.0, 80.0), hold: HOLD_ANY, mix: 1 }
    }
}

/// [`Reach::pixels`] when nothing says otherwise. At 8, far terrain draws about as many
/// triangles as it did when levels reached 1.6 times as far as the one before (2026-11-14).
pub const DEFAULT_PIXELS: f32 = 8.0;

/// The range [`Reach::pixels`] is kept in.
pub const PIXELS: std::ops::RangeInclusive<f32> = 1.0..=64.0;

/// [`Reach::hold`]: builds keep a cell only by filling more than half of it, as every other
/// cell is kept.
pub const HOLD_OFF: u8 = 0;

/// [`Reach::hold`]: a build keeps a cell if it adds up to a boxel-thick plane right across it,
/// so a one-boxel wall needs to be as tall and as wide as the cell.
pub const HOLD_PLANE: u8 = 1;

/// [`Reach::hold`]: any built boxel keeps a cell, as Voxy's "any solid child wins" does (see
/// `docs/notes/voxy-questions.md`, answers 3, 4 and 26). Nothing built vanishes far away; it
/// gets as thick as the cell.
pub const HOLD_ANY: u8 = 2;

/// The most [`Reach::hold`] goes to.
pub const MAX_HOLD: u8 = HOLD_ANY;

/// The most [`Reach::mix`] goes to. A level finer makes far terrain about four times as slow
/// to build; two levels finer, sixteen (2026-11-14), for little to see.
pub const MAX_MIX: u8 = 1;

impl Reach {
    /// Pixels a meter-wide thing one meter away covers, for a view `height` pixels tall with a
    /// vertical field of view of `fov` degrees.
    pub fn screen_scale(height: f32, fov: f32) -> f32 {
        height.max(1.0) / (2.0 * (fov.clamp(1.0, 179.0).to_radians() * 0.5).tan())
    }

    /// The coarsest level used: the first whose far chunks never split at the far distance.
    pub fn top(&self) -> u8 {
        let mut level = 1u8;
        while level < MAX_LEVEL && self.split_distance(level) < self.far {
            level += 1;
        }
        level
    }

    /// Meters within which a level-`level` far chunk's middle must be for it to split: level
    /// 1 at the full-detail distance (so the ordinary chunks it splits into are all within the
    /// load distance, a chunk past it), coarser ones where their cells would look `pixels`
    /// wide, never nearer than that; and never less than the far chunk's width, so the one the
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

    /// Whether `chunk` splits into its children, seen from `pos`.
    pub fn splits(&self, chunk: FarChunk, pos: WorldPos) -> bool {
        chunk.level > 0 && chunk.distance(pos) < self.split_distance(chunk.level)
    }

    /// Whether any of `chunk` is within the far distance of `pos`.
    pub fn reaches(&self, chunk: FarChunk, pos: WorldPos) -> bool {
        chunk.nearest(pos) < self.far
    }

    /// The coarsest far chunks of `terrain` within reach of `pos`, every way.
    fn tops(&self, terrain: usize, pos: WorldPos) -> Vec<FarChunk> {
        let level = self.top();
        let span = (CHUNK_SIZE << level) as f32;
        let c = pos.cell().as_vec3() / span;
        let c = I64Vec3::new(c.x.floor() as i64, c.y.floor() as i64, c.z.floor() as i64);
        let r = (self.far / span).ceil() as i64 + 1;
        let mut out = Vec::new();
        for z in c.z - r..=c.z + r {
            for y in c.y - r..=c.y + r {
                for x in c.x - r..=c.x + r {
                    let f = FarChunk { terrain, level, pos: I64Vec3::new(x, y, z) };
                    if self.reaches(f, pos) {
                        out.push(f);
                    }
                }
            }
        }
        out
    }

    /// Every far chunk (level 1 and up) of `terrain` the octree may draw from `pos`: those
    /// within reach that are the coarsest, or the child of one that splits. Coarse ones near
    /// the player stay wanted under their children, to fall back on as the player moves.
    pub fn wanted(&self, terrain: usize, pos: WorldPos) -> Vec<FarChunk> {
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

    /// The far chunks of `terrain` to draw from `pos`, any level, as fine as what's `ready` to
    /// draw allows. A far chunk splits only once all its children can be drawn; until then
    /// it's drawn whole if it's ready, or else its ready children are (leaving holes where
    /// nothing is ready). So no far chunk (level 1 and up) lies over another. Ordinary chunks
    /// (level 0) that are ready are always drawn, under a far chunk still waiting too: full
    /// detail that's in is never hidden, and whoever draws leaves the far chunk out where they
    /// are (see `client::far`'s mask).
    pub fn pick(&self, terrain: usize, pos: WorldPos, ready: &dyn Fn(FarChunk) -> bool) -> Vec<FarChunk> {
        let mut out = Vec::new();
        for top in self.tops(terrain, pos) {
            out.extend(self.pick_in(top, pos, ready).0);
        }
        out
    }

    /// What to draw of `chunk`, and whether that covers all of it within reach.
    fn pick_in(&self, chunk: FarChunk, pos: WorldPos, ready: &dyn Fn(FarChunk) -> bool) -> (Vec<FarChunk>, bool) {
        if !self.reaches(chunk, pos) {
            return (Vec::new(), true);
        }
        if self.splits(chunk, pos) {
            let mut all = Vec::new();
            let mut whole = true;
            for child in chunk.children() {
                let (list, covered) = self.pick_in(child, pos, ready);
                all.extend(list);
                whole &= covered;
            }
            if whole {
                return (all, true);
            }
            if ready(chunk) {
                // It stands in for its far chunks; the ordinary chunks under it stay.
                all.retain(|c| c.level == 0);
                all.push(chunk);
                return (all, true);
            }
            return (all, false);
        }
        if ready(chunk) { (vec![chunk], true) } else { (Vec::new(), false) }
    }

    /// The reach a little wider, for dropping far chunks only once they're well past wanted:
    /// full detail, the far distance and where levels split by size on screen. (What's wanted
    /// is worked out as the player comes into a chunk, and a player moving within it mustn't
    /// drop what was just built.)
    pub fn widened(&self) -> Reach {
        Reach { detail: self.detail * 1.25 + CHUNK_SIZE as f32, far: self.far + 2.0 * CHUNK_SIZE as f32, screen: self.screen * 1.25, ..*self }
    }
}

/// A far chunk's cells and their [`Mix`], with something in them.
#[derive(Clone, Debug, PartialEq)]
pub struct FarCells {
    pub data: ChunkData,
    pub mix: Mix,
}

/// A far chunk as sent: its cells, or `None` if nothing in it is solid.
#[derive(Clone, Debug, PartialEq)]
pub struct FarBuilt {
    pub chunk: FarChunk,
    pub cells: Option<FarCells>,
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
    /// Far chunks loaded now.
    pub chunks: usize,
    /// Wanted and not loaded yet.
    pub waiting: usize,
    /// Seconds the worker threads spent building them, added up.
    pub seconds: f32,
}

/// What building a far chunk made.
enum Made {
    /// The generator can't fill coarsely.
    Cannot,
    Cells(Option<FarCells>),
}

/// A far chunk built on a terrain thread.
struct Built {
    epoch: u64,
    serial: u64,
    chunk: FarChunk,
    made: Result<Made, ApiError>,
    seconds: f32,
}

/// What a terrain thread needs to build a far chunk.
struct FarJob {
    chunk: FarChunk,
    generator: SharedGenerator,
    center: I64Vec3,
    blocks: Arc<BlockRegistry>,
    store: Arc<dyn ChunkStore>,
    /// Edited chunks in it that are loaded, as they stand.
    loaded: Vec<ChunkData>,
    /// Edited chunks in it to read from the store.
    stored: Vec<ChunkPos>,
    /// Which ordinary chunk X and Y under it terrain is kept away from (see
    /// [`crate::stream::CLEAR_DISTANCE`]), x fastest; empty if none is.
    cleared: Vec<bool>,
    /// [`Reach::hold`] and [`Reach::mix`].
    hold: u8,
    mix: u8,
}

/// The far chunks around the player: which are loaded, wanted and on their way.
pub struct Far {
    pub reach: Reach,
    loaded: HashSet<FarChunk>,
    /// Asked of the workers, by the serial they were asked with.
    in_flight: HashMap<FarChunk, u64>,
    /// Loaded or asked-for far chunks an edit has changed since, by the serial a build must be
    /// newer than to count.
    stale: HashMap<FarChunk, u64>,
    serial: u64,
    epoch: u64,
    wanted: Vec<FarChunk>,
    wanted_for: Option<(I64Vec3, Reach, u64)>,
    /// How the loaded far chunks were built: their [`Reach::hold`] and [`Reach::mix`].
    built_like: Option<Reach>,
    /// Edited chunks in the store, once read from it.
    stored: Option<HashSet<ChunkPos>>,
    /// Terrains whose generator can't fill coarsely.
    cannot: HashSet<usize>,
    outbox: Sender<Built>,
    done: Receiver<Built>,
    /// The block registry the workers name blocks from, and how many blocks it had.
    blocks: Option<(usize, Arc<BlockRegistry>)>,
    loaded_now: Vec<FarBuilt>,
    unloaded_now: Vec<FarChunk>,
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
    /// Forgets every far chunk, as when the terrain is made again or another world loads.
    pub fn reset(&mut self) {
        self.epoch += 1;
        let mut gone: Vec<FarChunk> = self.loaded.drain().collect();
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

    /// Notes chunks just put in the store, so the far chunks holding them are built again
    /// with them.
    pub fn edited(&mut self, chunks: &[ChunkPos]) {
        if let Some(stored) = &mut self.stored {
            stored.extend(chunks.iter().copied());
        }
        let terrains: HashSet<usize> = self.loaded.iter().chain(self.in_flight.keys()).map(|c| c.terrain).collect();
        for &pos in chunks {
            for &terrain in &terrains {
                for level in 1..=MAX_LEVEL {
                    let c = FarChunk::holding(terrain, level, pos);
                    if self.loaded.contains(&c) || self.in_flight.contains_key(&c) {
                        self.stale.insert(c, self.serial);
                    }
                }
            }
        }
    }

    /// Whether `chunk` is loaded.
    pub fn is_loaded(&self, chunk: FarChunk) -> bool {
        self.loaded.contains(&chunk)
    }

    /// Whether every wanted far chunk is loaded and up to date.
    pub fn settled(&self) -> bool {
        self.wanted_for.is_some() && self.wanted.iter().all(|c| self.loaded.contains(c) && !self.stale.contains_key(c))
    }

    /// Whether any far chunk is on its way.
    pub fn busy(&self) -> bool {
        !self.in_flight.is_empty()
    }

    /// One step: drops far chunks well past the reach, asks for wanted ones missing or stale,
    /// coarse and near first, and takes in what's been built. `cleared` says which ordinary
    /// chunk X and Y terrain is kept away from.
    pub(crate) fn step(&mut self, stream: &mut Stream, around: &Around, cleared: &dyn Fn(i64, i64) -> bool) {
        let pos = around.pos;
        // Drop.
        let keep = around.terrains.len();
        let widened = self.reach.widened();
        let mut far: Vec<FarChunk> = self
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
        // Built another way than asked now: every far chunk is built again, the old ones drawn
        // until then.
        if self.built_like.is_some_and(|r| !r.builds_like(&self.reach)) {
            for &c in self.loaded.iter().chain(self.in_flight.keys()) {
                self.stale.insert(c, self.serial);
            }
        }
        self.built_like = Some(self.reach);
        let key = (chunk, self.reach, self.epoch);
        if self.wanted_for != Some(key) {
            self.wanted_for = Some(key);
            let mut wanted: Vec<(f32, FarChunk)> = Vec::new();
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

    /// Whether `chunk`'s still within what `reach` wants from `pos`: its parent splits there,
    /// or it's among the coarsest.
    fn kept(reach: &Reach, chunk: FarChunk, pos: WorldPos) -> bool {
        chunk.level >= reach.top() || reach.splits(chunk.parent(), pos)
    }

    fn take_built(&mut self) {
        let wanted: HashSet<FarChunk> = self.wanted.iter().copied().collect();
        let came: Vec<Built> = self.done.try_iter().collect();
        for built in came {
            if built.epoch != self.epoch {
                continue;
            }
            if self.in_flight.get(&built.chunk) == Some(&built.serial) {
                self.in_flight.remove(&built.chunk);
            }
            self.stats.seconds += built.seconds;
            let cells = match built.made {
                Ok(Made::Cells(cells)) => cells,
                Ok(Made::Cannot) => {
                    if self.cannot.insert(built.chunk.terrain) {
                        eprintln!("terrain {}: its generator can't fill coarsely, so it has no far terrain", built.chunk.terrain);
                    }
                    continue;
                }
                Err(e) => {
                    self.error = Some(e);
                    continue;
                }
            };
            if !wanted.contains(&built.chunk) {
                continue;
            }
            if let Some(&after) = self.stale.get(&built.chunk) {
                if built.serial <= after {
                    // Built before an edit in it: it's asked for again.
                    continue;
                }
                self.stale.remove(&built.chunk);
            }
            self.loaded.insert(built.chunk);
            self.loaded_now.push(FarBuilt { chunk: built.chunk, cells });
        }
        self.stats.chunks = self.loaded.len();
    }

    fn request(&mut self, stream: &mut Stream, around: &Around, cleared: &dyn Fn(i64, i64) -> bool) {
        let missing: Vec<FarChunk> = self
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
        // Ordinary chunks come first, but far ones always have one on its way.
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
        for chunk in missing {
            if sent == room {
                break;
            }
            if let Some(&serial) = self.in_flight.get(&chunk) {
                let fresh = self.stale.get(&chunk).is_none_or(|&after| serial > after);
                if fresh {
                    continue;
                }
            }
            let loaded: Vec<ChunkData> = around.edited.iter().filter(|&&p| chunk.holds(p)).map(|&p| around.grid.export_chunk(p)).collect();
            let stored: Vec<ChunkPos> = stored_all.iter().filter(|&&p| chunk.holds(p) && !around.edited.contains(&p)).copied().collect();
            let [xs, ys, _] = chunk.under();
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
                chunk,
                generator: Arc::clone(&around.generators[chunk.terrain]),
                center: around.terrains[chunk.terrain].center.0,
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
                let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(&job)))
                    .unwrap_or_else(|_| Err(ApiError::InvalidDef(format!("building far chunk {:?} panicked", job.chunk))));
                let seconds = start.elapsed().as_secs_f32();
                let _ = outbox.send(Built { epoch, serial, chunk: job.chunk, made, seconds });
            });
            if !stream.workers().send(task) {
                break;
            }
            self.in_flight.insert(chunk, serial);
            sent += 1;
        }
    }

    /// Far chunks built since the last call, oldest first. One built again after an edit
    /// comes again, to replace the one before.
    pub fn take_loaded(&mut self) -> Vec<FarBuilt> {
        std::mem::take(&mut self.loaded_now)
    }

    /// Far chunks dropped since the last call.
    pub fn take_unloaded(&mut self) -> Vec<FarChunk> {
        std::mem::take(&mut self.unloaded_now)
    }

    /// The loaded far chunks, in no particular order.
    pub fn loaded(&self) -> impl Iterator<Item = FarChunk> + '_ {
        self.loaded.iter().copied()
    }

    /// Waits until every wanted far chunk is in, for tests and tools. Gives up after a minute.
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

/// An edited chunk in a far chunk, and what the generator makes there, to tell what was
/// built (`None` where nothing is told apart: all of it counts as built).
struct Edit {
    data: ChunkData,
    generated: Option<ChunkData>,
}

/// Builds a far chunk: the generator's coarse fill, cleared where terrain is kept away, with
/// the edited chunks in it shrunk in, and its [`Mix`]. Known empty without filling if it's
/// over all the generator's tops under it (a cell to spare) and nothing's edited in it.
fn build(job: &FarJob) -> Result<Made, ApiError> {
    let generator = job.generator.read().map_err(|_| ApiError::InvalidDef("a terrain generator panicked".into()))?;
    let chunk = job.chunk;
    let step = chunk.step();
    let mut edited: Vec<ChunkData> = job.loaded.clone();
    for &pos in &job.stored {
        if let Some(bytes) = job.store.read(pos).map_err(ApiError::Storage)? {
            let saved = SavedChunk::from_bytes(&bytes).map_err(|e| ApiError::Storage(format!("chunk {:?}: {e}", pos.0.to_array())))?;
            let mut report = crate::LoadReport::default();
            edited.push(crate::simulation::save::unname_chunk(&job.blocks, &saved, &mut report));
        }
    }
    let origin = chunk.pos * chunk.span();
    if edited.is_empty() {
        let mut tops = [api::NO_TOP; api::TOPS_LEN];
        generator.tops(origin.x - job.center.x, origin.y - job.center.y, step as u32, &mut tops);
        let highest = tops.iter().filter(|&&t| t != api::NO_TOP).map(|&t| t + job.center.z).max();
        if highest.is_none_or(|h| origin.z > h + step) {
            return Ok(Made::Cells(None));
        }
    }
    let mut fill = ChunkFill::default();
    let [xs, ys, _] = chunk.under();
    // What the generator makes under each edit, only needed to hold built cells.
    let mut edits = Vec::with_capacity(edited.len());
    for data in edited {
        let generated = if job.hold != HOLD_OFF {
            let p = data.pos.0;
            let n = 1i64 << chunk.level;
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
    fill.clear();
    if !generator.fill_coarse(origin - job.center, step as u32, &mut fill) {
        return Ok(Made::Cannot);
    }
    let mut data = crate::terrain::to_chunk(ChunkPos(chunk.pos), &fill, &job.blocks)?;
    if !job.cleared.is_empty() {
        clear(&mut data, chunk, &job.cleared);
    }
    let mut mix = Mix::default();
    let touched = shrink_edits(&mut data, &mut mix, chunk, &edits, &job.blocks, job.hold);
    data.cells.compact();
    let uniform = data.cells.uniform();
    if uniform == Some(0) {
        return Ok(Made::Cells(None));
    }
    if job.mix > 0 && uniform.is_none() && !mix_finer(&**generator, &mut data, &mut mix, chunk, (job.center, job.mix, &job.blocks), &touched, &mut fill)? {
        return Ok(Made::Cannot);
    }
    mix.cells.compact();
    Ok(Made::Cells(Some(FarCells { data, mix })))
}

/// Finds what each solid cell of `data` shows by filling the generator `depth` levels finer
/// over it (see [`Reach::mix`]), the terrain's center at `center`: of the finer cells that
/// face air, as full detail would show them, the material most are becomes the cell's own,
/// the next its second (see [`Mix`]). Buried finer cells don't count: the dirt under a
/// grass top isn't seen from above. Cells `touched` by edits are left as the edits made them,
/// as are cleared ones. `false` if the generator can't fill coarsely.
fn mix_finer(
    generator: &dyn api::Generator,
    data: &mut ChunkData,
    mix: &mut Mix,
    far: FarChunk,
    (center, depth, blocks): (I64Vec3, u8, &BlockRegistry),
    touched: &[bool],
    fill: &mut ChunkFill,
) -> Result<bool, ApiError> {
    let depth = depth.min(far.level) as i64;
    let per = 1i64 << depth;
    let fine = far.step() >> depth;
    let size = CHUNK_SIZE;
    // Up to four materials and how many finer cells each fills, per cell.
    let mut tally: Vec<[(crate::BlockId, u16); 4]> = vec![[(crate::BlockId::default(), 0); 4]; api::CHUNK_VOLUME];
    // Only cells that can show need a mix: solid ones with air beside them, or under the
    // chunk's top (what's over it isn't known here). A cell on the far chunk's sides counts by
    // its neighbors inside: the far chunks beside it hide what it shows there.
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
                let origin = data.pos.0 * far.span() + sub * size * fine;
                if !generator.fill_coarse(origin - center, fine as u32, fill) {
                    return Ok(false);
                }
                let ids: Vec<Option<crate::BlockId>> = std::iter::once(None)
                    .chain(fill.palette.iter().map(|name| blocks.id(name)))
                    .collect();
                // Whether the finer cell at `p` (in this finer chunk) faces air: past the finer
                // chunk, as the cell there says, open over the top, closed on the sides and under.
                let empty = |x: i64, y: i64, z: i64| -> bool {
                    if (0..size).contains(&x) && (0..size).contains(&y) && (0..size).contains(&z) {
                        let e = fill.cells[LocalPos::new(x as u8, y as u8, z as u8).index()] as usize;
                        return ids.get(e).copied().flatten().is_none();
                    }
                    let (px, py, pz) = (base.x + x.div_euclid(per), base.y + y.div_euclid(per), base.z + z.div_euclid(per));
                    if pz >= size {
                        return true;
                    }
                    if pz < 0 || !(0..size).contains(&px) || !(0..size).contains(&py) {
                        return false;
                    }
                    !filled(px, py, pz)
                };
                for i in 0..api::CHUNK_VOLUME {
                    let Some(block) = ids.get(fill.cells[i] as usize).copied().flatten() else { continue };
                    let p = LocalPos::from_index(i);
                    let (x, y, z) = (p.x as i64, p.y as i64, p.z as i64);
                    let faces_air = empty(x, y, z + 1)
                        || empty(x, y, z - 1)
                        || empty(x + 1, y, z)
                        || empty(x - 1, y, z)
                        || empty(x, y + 1, z)
                        || empty(x, y - 1, z);
                    if !faces_air {
                        continue;
                    }
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

/// Empties the cells of `data` over the ordinary chunk X and Y `cleared` marks.
fn clear(data: &mut ChunkData, far: FarChunk, cleared: &[bool]) {
    let n = 1i64 << far.level;
    let size = CHUNK_SIZE;
    for i in 0..api::CHUNK_VOLUME {
        let p = LocalPos::from_index(i);
        // The ordinary chunk X and Y the cell's middle is over.
        let bx = (p.x as i64 * far.step() + far.step() / 2).div_euclid(size);
        let by = (p.y as i64 * far.step() + far.step() / 2).div_euclid(size);
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

/// Writes the cells of `data` (a far chunk of `far`'s level) that `edits` lie under,
/// shrunk, and their mix: a cell is solid if more than half filled, or if what was built in it
/// is enough to hold it (see [`Reach::hold`]), and takes the material filling most
/// of it, or of what was built; else it's emptied. A cell wider than a chunk counts what's
/// outside the edited chunks as it was generated. Returns which cells the edits touched.
fn shrink_edits(data: &mut ChunkData, mix: &mut Mix, far: FarChunk, edits: &[Edit], blocks: &BlockRegistry, hold: u8) -> Vec<bool> {
    let mut touched = vec![false; api::CHUNK_VOLUME];
    let n = 1i64 << far.level;
    let s = far.step();
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
    // How much must be built to hold a cell: none past "something" for any, a plane's worth.
    let holds = |built: f32| match hold {
        HOLD_OFF => false,
        HOLD_PLANE => built >= (s * s) as f32,
        _ => built > 0.0,
    };
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
        } else if holds(t.built) {
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
        at3(x, y, 0.0)
    }

    fn at3(x: f32, y: f32, z: f32) -> WorldPos {
        let cell = I64Vec3::new(x.floor() as i64, y.floor() as i64, z.floor() as i64);
        WorldPos::new(cell, Vec3::new(x, y, z) - cell.as_vec3())
    }

    fn far(level: u8, x: i64, y: i64, z: i64) -> FarChunk {
        FarChunk { terrain: 0, level, pos: I64Vec3::new(x, y, z) }
    }

    #[test]
    fn far_chunks_nest_by_halves() {
        let c = far(2, -1, 3, 0);
        assert_eq!(c.span(), 128);
        let children = c.children();
        assert_eq!(children.iter().collect::<HashSet<_>>().len(), 8);
        for child in children {
            assert_eq!(child.level, 1);
            assert_eq!(child.parent(), c);
        }
        assert_eq!(FarChunk::holding(0, 2, ChunkPos(I64Vec3::new(-1, 13, -1))), far(2, -1, 3, -1));
        assert_eq!(c.under(), [-4..0, 12..16, 0..4]);
        assert!(c.holds(ChunkPos(I64Vec3::new(-4, 15, 3))) && !c.holds(ChunkPos(I64Vec3::new(-4, 15, 4))));
        for b in c.beside() {
            assert_eq!((b.pos - c.pos).abs().element_sum(), 1);
        }
    }

    #[test]
    fn levels_split_where_their_cells_would_look_wider_than_the_pixel_target() {
        // 1080 pixels tall, 90° up and down: a meter one meter away is 540 pixels.
        let screen = Reach::screen_scale(1080.0, 90.0);
        assert!((screen - 540.0).abs() < 0.01);
        let reach = Reach { detail: 300.0, far: 4096.0, pixels: 6.0, screen, hold: HOLD_PLANE, mix: 1 };
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
    fn nearest_is_zero_inside_and_grows_outside_every_way() {
        let c = far(1, 0, 0, 0);
        assert_eq!(c.nearest(at3(10.0, 60.0, 30.0)), 0.0);
        assert!((c.nearest(at(74.0, 10.0)) - 10.0).abs() < 1e-4);
        assert!((c.nearest(at(-3.0, -4.0)) - 5.0).abs() < 1e-4);
        assert!((c.nearest(at3(10.0, 10.0, 70.0)) - 6.0).abs() < 1e-4, "over it");
        assert!((c.nearest(at3(10.0, 10.0, -8.0)) - 8.0).abs() < 1e-4, "under it");
        assert!((c.distance(at3(32.0, 32.0, 132.0)) - 100.0).abs() < 1e-4);
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
    fn the_far_chunk_the_player_is_in_splits_however_short_full_detail_is() {
        let reach = reach(8.0, 2048.0);
        // At a corner of a level-5 far chunk, 1.7 km from its middle.
        let pos = at3(0.5, 0.5, 0.5);
        let mut c = far(reach.top(), 0, 0, 0);
        while c.level > 0 {
            assert!(reach.splits(c, pos), "{c:?}");
            c = c.children()[0];
        }
    }

    /// The ordinary chunks the picked far chunks cover, each once at most, within `around`
    /// chunks of `center`.
    fn coverage(picked: &[FarChunk], center: I64Vec3, around: i64) -> HashMap<I64Vec3, usize> {
        let mut seen = HashMap::new();
        for c in picked {
            let [xs, ys, zs] = c.under();
            let clip = |r: Range<i64>, m: i64| r.start.max(m - around)..r.end.min(m + around + 1);
            for z in clip(zs, center.z) {
                for y in clip(ys.clone(), center.y) {
                    for x in clip(xs.clone(), center.x) {
                        *seen.entry(I64Vec3::new(x, y, z)).or_insert(0) += 1;
                    }
                }
            }
        }
        seen
    }

    #[test]
    fn with_everything_ready_each_spot_is_drawn_once_finer_nearer() {
        let reach = reach(256.0, 2048.0);
        for pos in [at3(16.0, 16.0, 16.0), at3(-700.5, 333.25, 90.0), at3(5000.0, -12000.0, -400.0)] {
            let picked = reach.pick(0, pos, &|_| true);
            let c = pos.boxel().chunk().0;
            let seen = coverage(&picked, c, 24);
            assert!(seen.values().all(|&n| n == 1), "nothing drawn twice at {pos:?}");
            // Everything near is covered, every way.
            for dz in -24..=24 {
                for dy in -24..=24 {
                    for dx in -24..=24 {
                        let o = I64Vec3::new(dx, dy, dz);
                        if (o.as_vec3() * CHUNK_SIZE as f32).length() < 700.0 {
                            assert!(seen.contains_key(&(c + o)), "{o:?} covered from {pos:?}");
                        }
                    }
                }
            }
            // Full detail around, above and below too, and coarser out at the edge.
            assert!(picked.iter().any(|p| p.level == 0 && p.distance(pos) < 40.0));
            for p in &picked {
                if p.level == 0 {
                    assert!(p.distance(pos) < reach.detail + 50.0, "{p:?}");
                }
            }
            // Far chunks the server builds are all those drawn, at level 1 and up.
            let wanted: HashSet<FarChunk> = reach.wanted(0, pos).into_iter().collect();
            assert!(picked.iter().filter(|p| p.level > 0).all(|p| wanted.contains(p)));
        }
    }

    #[test]
    fn levels_follow_distance_up_and_down() {
        let reach = reach(256.0, 2048.0);
        let level_at = |pos: WorldPos, chunk: I64Vec3| {
            let picked = reach.pick(0, pos, &|_| true);
            picked.into_iter().find(|p| p.holds(ChunkPos(chunk))).map(|p| p.level)
        };
        let ground = I64Vec3::new(0, 0, 0);
        // On the ground: full detail. 600 m up: the ground under is coarse.
        assert_eq!(level_at(at3(16.0, 16.0, 16.0), ground), Some(0));
        let high = level_at(at3(16.0, 16.0, 600.0), ground).unwrap();
        assert!(high >= 2, "level {high} under a player 600 m up");
        // A pit 200 m straight down stays full detail; ground 200 m away sideways too.
        assert_eq!(level_at(at3(16.0, 16.0, 16.0), I64Vec3::new(0, 0, -6)), Some(0));
        assert_eq!(level_at(at3(16.0, 16.0, 16.0), I64Vec3::new(6, 0, 0)), Some(0));
    }

    #[test]
    fn a_far_chunk_splits_only_once_its_children_are_ready() {
        let reach = reach(256.0, 2048.0);
        let pos = at3(16.0, 16.0, 16.0);
        // Everything but one ordinary chunk near the player.
        let missing = far(0, 1, 0, 0);
        let picked = reach.pick(0, pos, &|c| c != missing);
        let parent = missing.parent();
        assert!(picked.contains(&parent), "its level-1 far chunk drawn instead");
        // The seven that are ready stay drawn under it, and no far chunk lies over another.
        assert_eq!(picked.iter().filter(|p| p.level == 0 && p.parent() == parent).count(), 7);
        let center = pos.boxel().chunk().0;
        let far_only: Vec<FarChunk> = picked.iter().copied().filter(|p| p.level > 0).collect();
        assert!(coverage(&far_only, center, 16).values().all(|&n| n == 1));
        let ordinary: Vec<FarChunk> = picked.iter().copied().filter(|p| p.level == 0).collect();
        assert!(coverage(&ordinary, center, 16).values().all(|&n| n == 1));
        // Nothing far ready: the ordinary chunks that are, with holes elsewhere.
        let picked = reach.pick(0, pos, &|c| c.level == 0);
        assert!(!picked.is_empty() && picked.iter().all(|p| p.level == 0));
        assert!(coverage(&picked, center, 16).values().all(|&n| n == 1));
    }

    #[test]
    fn ordinary_chunks_stay_drawn_under_a_far_chunk_still_waiting() {
        // As after spawning somewhere new: the ordinary chunks around the player are in, and
        // of far terrain only the coarsest far chunks, whose finer levels are still coming.
        let reach = reach(256.0, 2048.0);
        let pos = at3(16.0, 16.0, 16.0);
        let center = pos.boxel().chunk().0;
        let near = |c: FarChunk| (c.pos - center).abs().max_element() <= 2;
        let picked = reach.pick(0, pos, &|c| if c.level == 0 { near(c) } else { c.level == reach.top() });
        // Every ordinary chunk that's in is drawn, once.
        for z in -2..=2 {
            for y in -2..=2 {
                for x in -2..=2 {
                    let c = far(0, center.x + x, center.y + y, center.z + z);
                    assert_eq!(picked.iter().filter(|&&p| p == c).count(), 1, "{c:?} drawn once");
                }
            }
        }
        // The coarsest far chunk over them is drawn too (the client leaves it out where they
        // are), and no far chunk lies over another.
        let far_only: Vec<FarChunk> = picked.iter().copied().filter(|p| p.level > 0).collect();
        assert!(far_only.iter().any(|p| p.level == reach.top() && p.holds(ChunkPos(center))));
        assert!(coverage(&far_only, center, 16).values().all(|&n| n == 1));
    }

    #[test]
    fn widened_keeps_what_was_wanted_from_anywhere_in_the_same_chunk() {
        // What's wanted is worked out as the player comes into a chunk; moving about in it
        // (falling, say) mustn't drop any of it, or it's built and dropped over and over.
        for reach in [reach(256.0, 2048.0), reach(32.0, 512.0), Reach { pixels: 1.0, ..reach(64.0, 2048.0) }] {
            let wide = reach.widened();
            for (from, to) in [((0.5, 0.5, 0.5), (31.5, 31.5, 31.5)), ((31.5, 0.5, 31.5), (0.5, 31.5, 0.5)), ((400.0, -90.0, 300.0), (415.0, -65.0, 319.0))] {
                let (from, to) = (at3(from.0, from.1, from.2), at3(to.0, to.1, to.2));
                assert_eq!(from.boxel().chunk(), to.boxel().chunk());
                for c in reach.wanted(0, from) {
                    assert!(wide.reaches(c, to) && Far::kept(&wide, c, to), "{c:?} wanted from {from:?}, dropped at {to:?}");
                }
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
        let cube = far(1, 0, 0, 0);
        let mut data = chunk(I64Vec3::ZERO, vec![Vec::new()], |_| 0);
        // An edited chunk at ordinary chunk (1, 0, 0): stone in its lowest 5 boxels.
        let edit = chunk(I64Vec3::new(1, 0, 0), vec![Vec::new(), vec![Occupant::new(stone)]], |p| (p.z < 5) as u16);
        let mut mix = Mix::default();
        shrink_edits(&mut data, &mut mix, cube, &[Edit { data: edit, generated: None }], &blocks, HOLD_OFF);
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
    fn shrunk(level: u8, edits: &[Edit], blocks: &BlockRegistry, hold: u8) -> ChunkData {
        let cube = far(level, 0, 0, 0);
        let mut data = chunk(I64Vec3::ZERO, vec![Vec::new()], |_| 0);
        let mut mix = Mix::default();
        shrink_edits(&mut data, &mut mix, cube, edits, blocks, hold);
        data
    }

    /// An edit at `pos` of stone where `solid` says, on empty generated ground.
    fn built(pos: I64Vec3, stone: crate::BlockId, solid: impl Fn(LocalPos) -> bool) -> Edit {
        let data = chunk(pos, vec![Vec::new(), vec![Occupant::new(stone)]], |p| solid(p) as u16);
        Edit { data, generated: Some(chunk(pos, vec![Vec::new()], |_| 0)) }
    }

    #[test]
    fn any_built_boxel_keeps_its_cell_at_every_level() {
        let (blocks, stone, _) = stone_and_grass();
        type Build = (&'static str, Box<dyn Fn(LocalPos) -> bool>);
        let builds: [Build; 4] = [
            // A wall 3 boxels tall and 6 long: far shorter than most cells.
            ("short wall", Box::new(|p: LocalPos| p.x == 5 && p.y < 6 && p.z < 3)),
            // The same wall standing across the line between cells of every level, at 15 and 16.
            ("wall across cells", Box::new(|p: LocalPos| p.x == 5 && p.y < 6 && (14..18).contains(&p.z))),
            ("pillar", Box::new(|p: LocalPos| p.x == 5 && p.y == 5)),
            ("single block", Box::new(|p: LocalPos| p.x == 5 && p.y == 0 && p.z == 16)),
        ];
        for (name, solid) in &builds {
            for level in 1..=MAX_LEVEL {
                let data = shrunk(level, &[built(I64Vec3::ZERO, stone, solid)], &blocks, HOLD_ANY);
                // The cells the build's boxels fall in, and only those, are solid.
                let mut want = HashSet::new();
                for i in 0..api::CHUNK_VOLUME {
                    let b = LocalPos::from_index(i);
                    if solid(b) {
                        want.insert(LocalPos::new(b.x >> level, b.y >> level, b.z >> level).index());
                    }
                }
                assert!(!want.is_empty());
                for i in 0..api::CHUNK_VOLUME {
                    assert_eq!(data.cells.at(i) != 0, want.contains(&i), "{name}, level {level}, cell {:?}", LocalPos::from_index(i));
                }
                // None of them is a plane right across a cell 4 boxels or more a side.
                if level >= 2 {
                    let plane = shrunk(level, &[built(I64Vec3::ZERO, stone, solid)], &blocks, HOLD_PLANE);
                    assert_eq!(plane.cells.uniform(), Some(0), "{name}, level {level}: not held as a plane");
                }
            }
        }
    }

    #[test]
    fn a_one_boxel_wall_held_as_a_plane_needs_the_whole_cell() {
        let (blocks, stone, _) = stone_and_grass();
        for level in 1..=5 {
            let data = shrunk(level, &[wall(I64Vec3::ZERO, stone, 32)], &blocks, HOLD_PLANE);
            let x = (5 >> level) as u8;
            assert_eq!(cell(&data, x, 0, 0), Some(stone), "level {level}");
            assert_eq!(cell(&data, x + 1, 0, 0), None, "level {level}: only the wall's cells");
            // Held off, a thin wall goes.
            assert_eq!(cell(&shrunk(level, &[wall(I64Vec3::ZERO, stone, 32)], &blocks, HOLD_OFF), x, 0, 0), None, "level {level}");
        }
        // At level 6 a cell is 2 chunks a side: a wall through all of it, made of four
        // edited chunks, holds it; a wall through a quarter of it doesn't, but any build does.
        let four: Vec<Edit> = [(0, 0), (1, 0), (0, 1), (1, 1)].iter().map(|&(y, z)| wall(I64Vec3::new(0, y, z), stone, 32)).collect();
        assert_eq!(cell(&shrunk(6, &four, &blocks, HOLD_PLANE), 0, 0, 0), Some(stone));
        assert_eq!(cell(&shrunk(6, &four[..1], &blocks, HOLD_PLANE), 0, 0, 0), None);
        assert_eq!(cell(&shrunk(6, &four[..1], &blocks, HOLD_ANY), 0, 0, 0), Some(stone));
    }

    #[test]
    fn generated_ground_and_dug_out_space_are_not_held() {
        let (blocks, stone, _) = stone_and_grass();
        let pos = I64Vec3::ZERO;
        let lists = vec![Vec::new(), vec![Occupant::new(stone)]];
        // Ground 3 boxels deep that the generator made itself: not built, so cells 8 a side
        // stay empty under 3/8 of it, as they do where nothing was edited.
        let ground = |p: LocalPos| (p.z < 3) as u16;
        let edit = Edit { data: chunk(pos, lists.clone(), ground), generated: Some(chunk(pos, lists.clone(), ground)) };
        assert_eq!(cell(&shrunk(3, &[edit], &blocks, HOLD_ANY), 0, 0, 0), None);
        // Ground 8 deep dug down to 3: what's left was generated, what's gone is nothing built.
        let dug = Edit { data: chunk(pos, lists.clone(), ground), generated: Some(chunk(pos, lists, |p| (p.z < 8) as u16)) };
        assert_eq!(cell(&shrunk(3, &[dug], &blocks, HOLD_ANY), 0, 0, 0), None);
    }

    #[test]
    fn an_edited_cell_of_two_materials_keeps_both() {
        let (blocks, stone, grass) = stone_and_grass();
        // Stone in the lowest 5 boxels, grass the 3 over it: cells 8 a side are 5/8 stone,
        // 3/8 grass.
        let lists = vec![Vec::new(), vec![Occupant::new(stone)], vec![Occupant::new(grass)]];
        let edit = chunk(I64Vec3::ZERO, lists, |p| if p.z < 5 { 1 } else if p.z < 8 { 2 } else { 0 });
        let cube = far(3, 0, 0, 0);
        let mut data = chunk(I64Vec3::ZERO, vec![Vec::new()], |_| 0);
        let mut mix = Mix::default();
        let touched = shrink_edits(&mut data, &mut mix, cube, &[Edit { data: edit, generated: None }], &blocks, HOLD_PLANE);
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

    /// Stone below z 0, with grass making up the top `grass` boxels of it west of `bare`;
    /// bare stone from `bare` east.
    struct Layers {
        grass: i64,
        bare: i64,
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
                let x = origin.x + p.x as i64 * s + s / 2;
                let z = origin.z + p.z as i64 * s + s / 2;
                out.cells[i] = if z >= 0 { 0 } else if z >= -self.grass && x < self.bare { grass } else { stone };
            }
            true
        }
        fn tops(&self, _: i64, _: i64, _: u32, out: &mut [i64]) {
            out.fill(-1);
        }
    }

    /// The level-2 chunk under the ground of `generator` at the origin, its cells given their
    /// mix from level 1, and the main and second materials of the top cell at `x`.
    fn mixed_top(generator: &Layers, blocks: &BlockRegistry, x: u8) -> (crate::BlockId, Option<(crate::BlockId, f32)>) {
        use api::Generator;
        let cube = far(2, 0, 0, -1);
        let pos = ChunkPos(I64Vec3::new(0, 0, -1));
        let mut fill = ChunkFill::default();
        assert!(generator.fill_coarse(pos.0 * cube.span(), 4, &mut fill));
        let mut data = crate::terrain::to_chunk(pos, &fill, blocks).unwrap();
        let mut mix = Mix::default();
        let untouched = vec![false; api::CHUNK_VOLUME];
        assert!(mix_finer(generator, &mut data, &mut mix, cube, (I64Vec3::ZERO, 1, blocks), &untouched, &mut fill).unwrap());
        let top = LocalPos::new(x, 0, 31).index();
        (data.lists[data.cells.at(top) as usize][0].block, mix.at(top))
    }

    #[test]
    fn a_cell_shows_only_what_faces_the_air() {
        let (blocks, stone, grass) = stone_and_grass();
        // Grass 2 deep over stone: the top cell (4 boxels, from -4 to 0) samples stone at its
        // middle, but only its grass top shows from above.
        let (main, second) = mixed_top(&Layers { grass: 2, bare: i64::MAX }, &blocks, 0);
        assert_eq!(main, grass);
        assert_eq!(second, None, "the stone under the grass isn't seen");
        // Bare rock from x 2: the cell from x 0 to 4 shows half grass, half rock.
        let (main, second) = mixed_top(&Layers { grass: 2, bare: 2 }, &blocks, 0);
        let (other, share) = second.expect("mixed");
        assert!([main, other] == [grass, stone] || [main, other] == [stone, grass]);
        assert!((share - 0.5).abs() < 0.01, "{share}");
        // Past it, all rock.
        assert_eq!(mixed_top(&Layers { grass: 2, bare: 2 }, &blocks, 3), (stone, None));
    }
}
