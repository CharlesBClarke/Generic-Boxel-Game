//! Terrain streaming: generated terrain goes on forever sideways, so only the columns of its
//! chunks near the player are kept. Columns are generated and lit on worker threads, nearest
//! first, and brought into the world between ticks (see [`crate::Simulation`]); columns that fall
//! far behind are dropped, but chunks edited in them go to the chunk store (see [`crate::store`])
//! and come back from it in place of the generated ones.
//!
//! A column is every chunk of one terrain at one chunk X and Y, the terrain's full height, so
//! the sky above a column is always known. No terrain is made within [`CLEAR_DISTANCE`] of a
//! chunk the mods built, so the test areas stand apart from it.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use api::{ApiError, CHUNK_SIZE, ChunkFill, ChunkPos, Generator, I64Vec3, TerrainDef, WorldPos};

use crate::save::SavedChunk;
use crate::store::{ChunkStore, MemoryStore};
use crate::{BlockRegistry, ChunkData, Grid};

/// A generator the worker threads share with the simulation, which tunes it.
pub type SharedGenerator = Arc<RwLock<Box<dyn Generator>>>;

/// Horizontal meters around every chunk the mods built where no terrain is made.
pub const CLEAR_DISTANCE: f32 = 1024.0;

/// How long waiting for a column may take before it's given up on.
const STALLED: std::time::Duration = std::time::Duration::from_secs(60);

/// Meters past the load distance a column may drift before it's dropped, so walking back and
/// forth at the edge doesn't load and drop the same columns over and over.
pub const UNLOAD_MARGIN: f32 = CHUNK_SIZE as f32;

/// One terrain's chunks at one chunk X and Y.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Column {
    /// Which terrain, by its place among the mods' placements.
    pub terrain: usize,
    pub x: i64,
    pub y: i64,
}

impl Column {
    /// Every chunk in the column, top first.
    pub fn chunks(self, rows: Range<i64>) -> impl Iterator<Item = ChunkPos> {
        rows.rev().map(move |z| ChunkPos(I64Vec3::new(self.x, self.y, z)))
    }

    /// Horizontal meters from `pos` to the column's middle.
    pub fn distance(self, pos: WorldPos) -> f32 {
        let middle = I64Vec3::new(self.x * CHUNK_SIZE + CHUNK_SIZE / 2, self.y * CHUNK_SIZE + CHUNK_SIZE / 2, 0);
        let d = (middle - pos.cell()).as_vec3() - pos.offset();
        d.truncate().length()
    }
}

/// The chunk rows (Z) a terrain spans when it's `height` chunks tall: centered on its center.
pub fn rows(def: &TerrainDef, height: u32) -> Range<i64> {
    let center = def.center.chunk().0.z;
    let h = height as i64;
    center - h / 2..center + h - h / 2
}

/// How streaming has gone, for the client's readout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StreamStats {
    /// Columns loaded now.
    pub columns: usize,
    /// Columns within the load distance still waiting to be generated or brought in.
    pub waiting: usize,
    /// Columns generated since the terrain was last made from scratch.
    pub generated: usize,
    /// Seconds the worker threads spent generating and lighting them, added up.
    pub seconds: f32,
    /// Seconds the simulation spent bringing them in: mostly light across their seams.
    pub join_seconds: f32,
}

/// A column to generate.
struct Job {
    epoch: u64,
    column: Column,
    generator: SharedGenerator,
    center: I64Vec3,
    rows: Range<i64>,
    blocks: Arc<BlockRegistry>,
    store: Arc<dyn ChunkStore>,
}

/// A generated column, lit on its own, and its stored chunks to bring in instead.
pub(crate) struct Done {
    pub epoch: u64,
    pub column: Column,
    pub chunks: Result<Vec<ChunkData>, ApiError>,
    /// The column's chunks the store has, by name: the simulation names their blocks, since
    /// blocks made while the game runs are newer than the workers' registry.
    pub stored: Vec<SavedChunk>,
    pub seconds: f32,
}

/// Threads that generate columns. They stop once the simulation is dropped.
struct Workers {
    jobs: Sender<Job>,
    done: Receiver<Done>,
    count: usize,
}

impl Workers {
    fn start() -> Self {
        // Half the cores, less one: the game, its renderer and the client's background meshing
        // want the rest.
        let count = (std::thread::available_parallelism().map_or(2, |n| n.get()) / 2).saturating_sub(1).max(1);
        let (jobs, inbox) = channel::<Job>();
        let (outbox, done) = channel::<Done>();
        let inbox = Arc::new(Mutex::new(inbox));
        for i in 0..count {
            let inbox = Arc::clone(&inbox);
            let outbox = outbox.clone();
            std::thread::Builder::new()
                .name(format!("terrain {i}"))
                .spawn(move || {
                    loop {
                        // The lock is held only while waiting for the next job.
                        let job = match inbox.lock() {
                            Ok(inbox) => inbox.recv(),
                            Err(_) => return,
                        };
                        let Ok(job) = job else { return };
                        let start = Instant::now();
                        // A generator that panics fails its column, not the thread.
                        let chunks = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| generate_column(&job)))
                            .unwrap_or_else(|_| Err(ApiError::InvalidDef(format!("generating column {:?} panicked", job.column))));
                        let (chunks, stored) = match (chunks, read_stored(&job)) {
                            (Ok(chunks), Ok(stored)) => (Ok(chunks), stored),
                            (Err(e), _) | (_, Err(e)) => (Err(e), Vec::new()),
                        };
                        let seconds = start.elapsed().as_secs_f32();
                        let done = Done { epoch: job.epoch, column: job.column, chunks, stored, seconds };
                        if outbox.send(done).is_err() {
                            return;
                        }
                    }
                })
                .expect("starting a terrain thread");
        }
        Self { jobs, done, count }
    }
}

/// The column's chunks the store has.
fn read_stored(job: &Job) -> Result<Vec<SavedChunk>, ApiError> {
    let mut out = Vec::new();
    for pos in job.column.chunks(job.rows.clone()) {
        if let Some(bytes) = job.store.read(pos).map_err(ApiError::Storage)? {
            let chunk = SavedChunk::from_bytes(&bytes).map_err(|e| ApiError::Storage(format!("chunk {:?}: {e}", pos.0.to_array())))?;
            out.push(chunk);
        }
    }
    Ok(out)
}

/// Generates a column and lights it as if nothing stood beside it.
fn generate_column(job: &Job) -> Result<Vec<ChunkData>, ApiError> {
    let generator = job.generator.read().map_err(|_| ApiError::InvalidDef("a terrain generator panicked".into()))?;
    let mut fill = ChunkFill::default();
    let mut grid = Grid::default();
    let positions: Vec<ChunkPos> = job.column.chunks(job.rows.clone()).collect();
    for &pos in &positions {
        fill.clear();
        generator.fill(pos.origin().0 - job.center, &mut fill);
        grid.import_chunk(&crate::terrain::to_chunk(pos, &fill, &job.blocks)?);
    }
    grid.relight_all(&job.blocks);
    Ok(positions.into_iter().map(|pos| grid.export_chunk(pos)).collect())
}

/// Which columns are loaded, which are wanted and which are on their way; the store edited
/// chunks of dropped columns go to; and the worker threads.
pub struct Stream {
    /// Meters around the player to keep loaded.
    pub radius: f32,
    /// How many chunks tall each column is: the world settings' terrain height when the terrain
    /// was last made from scratch.
    height: u32,
    loaded: HashSet<Column>,
    /// Asked of the workers and not back yet.
    in_flight: HashSet<Column>,
    /// Back from the workers and not brought in yet, oldest first.
    ready: std::collections::VecDeque<Done>,
    /// Columns within the radius, nearest first, as last worked out.
    wanted: Vec<Column>,
    /// What `wanted` was worked out for: the player's column, the radius and the epoch.
    wanted_for: Option<((i64, i64), u32, u64)>,
    /// Where edited chunks of dropped columns go, to come back when they load again.
    store: Arc<dyn ChunkStore>,
    /// Columns of chunks the mods built, which keep terrain away.
    built: Vec<(i64, i64)>,
    /// Goes up whenever the terrain is made from scratch, so columns generated before are
    /// thrown away when they come back.
    epoch: u64,
    workers: Option<Workers>,
    /// The registry the workers name blocks from: a copy taken when they start. Terrain only
    /// uses blocks the mods registered, which never change.
    blocks: Option<Arc<BlockRegistry>>,
    /// Chunks loaded and dropped since the server last asked.
    loaded_now: Vec<ChunkPos>,
    unloaded_now: Vec<ChunkPos>,
    pub stats: StreamStats,
    /// Why generating last failed, if it did.
    pub error: Option<ApiError>,
}

impl Default for Stream {
    fn default() -> Self {
        Self::new(DEFAULT_RADIUS)
    }
}

/// Meters kept loaded until a client says how far it draws: the client's default render
/// distance and a chunk.
pub const DEFAULT_RADIUS: f32 = 512.0 + CHUNK_SIZE as f32;

impl Stream {
    /// A stream keeping `radius` meters around the player.
    pub fn new(radius: f32) -> Self {
        Self {
            radius,
            height: crate::WorldSettings::default().terrain_height,
            loaded: HashSet::new(),
            in_flight: HashSet::new(),
            ready: std::collections::VecDeque::new(),
            wanted: Vec::new(),
            wanted_for: None,
            store: Arc::new(MemoryStore::default()),
            built: Vec::new(),
            epoch: 0,
            workers: None,
            blocks: None,
            loaded_now: Vec::new(),
            unloaded_now: Vec::new(),
            stats: StreamStats::default(),
            error: None,
        }
    }

    /// How many chunks tall each column is.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Takes `height` as the columns' height if nothing is loaded or on its way, so the world
    /// settings apply before the first column; after that only making the terrain again does.
    pub fn follow_height(&mut self, height: u32) {
        if self.loaded.is_empty() && self.in_flight.is_empty() {
            self.height = height;
        }
    }

    /// Notes which columns the mods built on, to keep terrain away from them.
    pub fn set_built(&mut self, grid: &Grid) {
        let mut columns: Vec<(i64, i64)> = grid.chunks().map(|(pos, _)| (pos.0.x, pos.0.y)).collect();
        columns.sort_unstable();
        columns.dedup();
        self.built = columns;
    }

    /// Whether the column at chunk `(x, y)` is too near something the mods built for terrain.
    pub fn cleared(&self, x: i64, y: i64) -> bool {
        let reach = CLEAR_DISTANCE / CHUNK_SIZE as f32;
        self.built.iter().any(|&(bx, by)| {
            let (dx, dy) = ((bx - x) as f32, (by - y) as f32);
            dx * dx + dy * dy <= reach * reach
        })
    }

    pub fn is_loaded(&self, column: Column) -> bool {
        self.loaded.contains(&column)
    }

    /// Whether the chunk at `pos` belongs to a loaded column of a terrain spanning `rows`.
    pub fn covers(&self, terrain: usize, rows: &Range<i64>, pos: ChunkPos) -> bool {
        rows.contains(&pos.0.z) && self.loaded.contains(&Column { terrain, x: pos.0.x, y: pos.0.y })
    }

    /// The loaded columns, in no particular order.
    pub fn loaded(&self) -> impl Iterator<Item = Column> + '_ {
        self.loaded.iter().copied()
    }

    /// Where edited chunks of dropped columns go.
    pub fn store(&self) -> &Arc<dyn ChunkStore> {
        &self.store
    }

    /// Keeps edited chunks in `store` from now on. Columns asked for already read the old one.
    pub fn set_store(&mut self, store: Arc<dyn ChunkStore>) {
        self.store = store;
    }

    /// Forgets every column, loaded or on its way, so the terrain is made again from scratch,
    /// `height` chunks tall from now on. Returns the columns that were loaded.
    pub fn reset(&mut self, height: u32) -> Vec<Column> {
        self.epoch += 1;
        self.height = height;
        self.in_flight.clear();
        self.ready.clear();
        self.wanted_for = None;
        self.stats = StreamStats { columns: self.loaded.len(), ..StreamStats::default() };
        self.error = None;
        let mut columns: Vec<Column> = self.loaded.iter().copied().collect();
        columns.sort_unstable();
        columns
    }

    /// Columns within the radius of `pos`, nearest first, over `terrains` (each with its
    /// rows), leaving out cleared ones.
    pub fn wanted(&mut self, pos: WorldPos, terrains: usize) -> &[Column] {
        let chunk = pos.boxel().chunk().0;
        let key = ((chunk.x, chunk.y), self.radius.to_bits(), self.epoch);
        if self.wanted_for != Some(key) {
            self.wanted_for = Some(key);
            let reach = (self.radius / CHUNK_SIZE as f32).ceil() as i64 + 1;
            let mut wanted: Vec<(f32, Column)> = Vec::new();
            for y in chunk.y - reach..=chunk.y + reach {
                for x in chunk.x - reach..=chunk.x + reach {
                    if self.cleared(x, y) {
                        continue;
                    }
                    for terrain in 0..terrains {
                        let column = Column { terrain, x, y };
                        let d = column.distance(pos);
                        if d <= self.radius {
                            wanted.push((d, column));
                        }
                    }
                }
            }
            wanted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            self.wanted = wanted.into_iter().map(|(_, c)| c).collect();
        }
        &self.wanted
    }

    /// Loaded columns that have drifted past the radius and its margin from `pos`.
    pub fn too_far(&self, pos: WorldPos) -> Vec<Column> {
        let mut far: Vec<Column> = self.loaded.iter().copied().filter(|c| c.distance(pos) > self.radius + UNLOAD_MARGIN).collect();
        far.sort_unstable();
        far
    }

    /// Notes a column as dropped, with its chunks.
    pub fn drop_column(&mut self, column: Column, chunks: impl Iterator<Item = ChunkPos>) {
        self.loaded.remove(&column);
        self.unloaded_now.extend(chunks);
        self.stats.columns = self.loaded.len();
    }

    /// Notes a column as brought in, with its chunks and the seconds generating it took.
    pub fn add_column(&mut self, column: Column, chunks: impl Iterator<Item = ChunkPos>, seconds: f32) {
        self.loaded.insert(column);
        self.loaded_now.extend(chunks);
        self.stats.generated += 1;
        self.stats.seconds += seconds;
        self.stats.columns = self.loaded.len();
        self.stats.waiting = self.stats.waiting.saturating_sub(1);
    }

    /// Whether a wanted column within `meters` of `pos` isn't loaded yet.
    pub fn missing_within(&self, pos: WorldPos, meters: f32) -> bool {
        self.wanted.iter().take_while(|c| c.distance(pos) <= meters).any(|c| !self.loaded.contains(c))
    }

    /// Whether any column is on its way from the workers, or back and not brought in.
    pub fn busy(&self) -> bool {
        !self.in_flight.is_empty() || !self.ready.is_empty()
    }

    /// Asks the workers for wanted columns that aren't loaded or on their way, nearest first,
    /// keeping a few per worker queued so a move toward new ground is answered soon.
    pub fn request(&mut self, generators: &[SharedGenerator], terrains: &[TerrainDef], blocks: &BlockRegistry) {
        let wanted: Vec<Column> = self.wanted.iter().copied().filter(|c| !self.loaded.contains(c)).collect();
        self.stats.waiting = wanted.len();
        if wanted.is_empty() {
            return;
        }
        let workers = self.workers.get_or_insert_with(Workers::start);
        let room = (workers.count * 2).saturating_sub(self.in_flight.len());
        let blocks = self.blocks.get_or_insert_with(|| Arc::new(blocks.clone()));
        let asked: Vec<Column> = wanted.into_iter().filter(|c| !self.in_flight.contains(c)).take(room).collect();
        for column in asked {
            let def = &terrains[column.terrain];
            let job = Job {
                epoch: self.epoch,
                column,
                generator: Arc::clone(&generators[column.terrain]),
                center: def.center.0,
                rows: rows(def, self.height),
                blocks: Arc::clone(blocks),
                store: Arc::clone(&self.store),
            };
            if workers.jobs.send(job).is_ok() {
                self.in_flight.insert(column);
            }
        }
    }

    /// The next generated column to bring in, with ones from an older epoch or no longer
    /// wanted thrown away. Waits for one if `wait` and any are on their way.
    pub(crate) fn next_finished(&mut self, wait: bool) -> Option<Done> {
        if self.ready.is_empty() {
            let came = self.finished(wait && !self.busy_ready());
            self.ready.extend(came);
        }
        // The player may have moved on since it came back.
        while let Some(done) = self.ready.pop_front() {
            if self.wanted.contains(&done.column) && !self.loaded.contains(&done.column) {
                return Some(done);
            }
        }
        None
    }

    fn busy_ready(&self) -> bool {
        !self.ready.is_empty()
    }

    /// Generated columns that came back, with ones from an older epoch or no longer wanted
    /// thrown away. Waits for at least one if `wait` and any are on their way.
    fn finished(&mut self, wait: bool) -> Vec<Done> {
        let Some(workers) = &self.workers else { return Vec::new() };
        let mut out: Vec<Done> = Vec::new();
        if wait && !self.in_flight.is_empty() {
            match workers.done.recv_timeout(STALLED) {
                Ok(done) => out.push(done),
                Err(_) => {
                    // Nothing came back: ask again rather than wait forever.
                    self.in_flight.clear();
                    self.error = Some(ApiError::InvalidDef("generating terrain stalled".into()));
                }
            }
        }
        out.extend(workers.done.try_iter());
        let wanted: HashSet<Column> = self.wanted.iter().copied().collect();
        out.retain(|done| {
            if done.epoch != self.epoch {
                return false;
            }
            self.in_flight.remove(&done.column);
            wanted.contains(&done.column) && !self.loaded.contains(&done.column)
        });
        out
    }

    /// Whether every wanted column is loaded.
    pub fn settled(&self) -> bool {
        self.wanted_for.is_some() && self.wanted.iter().all(|c| self.loaded.contains(c))
    }

    /// Chunks brought in since the last call, in the order they came.
    pub fn take_loaded(&mut self) -> Vec<ChunkPos> {
        std::mem::take(&mut self.loaded_now)
    }

    /// Chunks dropped since the last call.
    pub fn take_unloaded(&mut self) -> Vec<ChunkPos> {
        std::mem::take(&mut self.unloaded_now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{BoxelPos, Vec3};

    fn at(x: f32, y: f32) -> WorldPos {
        WorldPos::new(I64Vec3::new(x.floor() as i64, y.floor() as i64, 0), Vec3::new(x.fract(), y.fract(), 0.0))
    }

    #[test]
    fn columns_within_the_radius_come_nearest_first() {
        let mut stream = Stream::new(48.0);
        let wanted = stream.wanted(at(16.0, 16.0), 1).to_vec();
        assert_eq!(wanted[0], Column { terrain: 0, x: 0, y: 0 }, "the one underfoot first");
        // Middles within 48 m of (16, 16): the 3×3 around, plus those 2 away straight out
        // (64 m) are past it, and the corners (45 m) are in.
        assert_eq!(wanted.len(), 9);
        let d: Vec<f32> = wanted.iter().map(|c| c.distance(at(16.0, 16.0))).collect();
        assert!(d.windows(2).all(|w| w[0] <= w[1]), "{d:?}");
    }

    #[test]
    fn columns_are_dropped_only_past_the_margin() {
        let mut stream = Stream::new(48.0);
        let column = Column { terrain: 0, x: 0, y: 0 };
        stream.add_column(column, std::iter::empty(), 0.0);
        // Its middle is at (16, 16): 64 m away is past the radius but within the margin.
        assert!(stream.too_far(at(80.0, 16.0)).is_empty());
        assert_eq!(stream.too_far(at(16.0 + 48.0 + UNLOAD_MARGIN + 1.0, 16.0)), [column]);
    }

    #[test]
    fn built_chunks_keep_terrain_away() {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(api::BlockDef::new("t:stone", true, [0; 3])).unwrap();
        let mut grid = Grid::default();
        grid.set_block(BoxelPos::new(5, 5, 5), stone);
        let mut stream = Stream::new(64.0);
        stream.set_built(&grid);
        assert!(stream.cleared(0, 0));
        assert!(stream.cleared(31, 0), "992 m away");
        assert!(!stream.cleared(33, 0), "1056 m away");
        assert!(stream.wanted(at(16.0, 16.0), 1).is_empty());
        assert!(!stream.wanted(at(16.0, 40_000.0), 1).is_empty());
    }

    #[test]
    fn rows_center_on_the_terrain() {
        let def = TerrainDef { generator: "t".into(), center: BoxelPos::new(0, 4096, 0) };
        assert_eq!(rows(&def, 6), -3..3);
        assert_eq!(rows(&def, 5), -2..3);
        let column = Column { terrain: 0, x: 1, y: 2 };
        let zs: Vec<i64> = column.chunks(rows(&def, 3)).map(|c| c.0.z).collect();
        assert_eq!(zs, [1, 0, -1], "top first");
    }
}
