//! Terrain streaming: generated terrain goes on forever every way, so only the chunks near a
//! player are kept: those within a sphere of the load distance, nearest first. Each chunk is
//! generated and lit on a worker thread as if nothing stood beside it (see
//! [`crate::light::light_alone`]) and brought into the world between ticks (see
//! [`crate::Simulation`]); chunks that fall far behind are dropped, but ones edited go to the
//! chunk store (see [`crate::store`]) and come back from it in place of the generated ones.
//!
//! Lighting a chunk alone takes knowing where the sky starts over it and beside it, so every
//! chunk comes with its column's tops and the four beside it (see [`crate::tops`]): the
//! simulation's where it has them, or else the stored ones, or else the generator's.
//!
//! No terrain is made within [`CLEAR_DISTANCE`] of a chunk the mods built, so the test areas
//! stand apart from it.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use api::{ApiError, CHUNK_SIZE, ChunkFill, ChunkPos, Generator, I64Vec3, NO_TOP, TOPS_LEN, TerrainDef, WorldPos};

use crate::save::SavedChunk;
use crate::store::{ChunkStore, MemoryStore};
use crate::tops::{AROUND, Tops, TopsArray};
use crate::{BlockRegistry, ChunkData, Layer, Light, Occupant};

/// A generator the worker threads share with the simulation, which tunes it.
pub type SharedGenerator = Arc<RwLock<Box<dyn Generator>>>;

/// Horizontal meters around every chunk the mods built where no terrain is made.
pub const CLEAR_DISTANCE: f32 = 1024.0;

/// How long waiting for a chunk may take before it's given up on.
const STALLED: std::time::Duration = std::time::Duration::from_secs(60);

/// Meters past the load distance a chunk may drift before it's dropped, so walking back and
/// forth at the edge doesn't load and drop the same chunks over and over.
pub const UNLOAD_MARGIN: f32 = CHUNK_SIZE as f32;

/// Meters from `from` to the middle of the chunk at `pos`.
pub fn chunk_distance(pos: ChunkPos, from: WorldPos) -> f32 {
    let middle = pos.0 * CHUNK_SIZE + I64Vec3::splat(CHUNK_SIZE / 2);
    ((middle - from.cell()).as_vec3() - from.offset()).length()
}

/// Meters from the nearest of `centers` to the middle of the chunk at `pos`.
fn nearest(pos: ChunkPos, centers: &[WorldPos]) -> f32 {
    centers.iter().map(|&c| chunk_distance(pos, c)).fold(f32::INFINITY, f32::min)
}

/// Whether the column at chunk `(x, y)` is within [`CLEAR_DISTANCE`] of one of the `built`
/// columns.
pub fn cleared_by(built: &[(i64, i64)], x: i64, y: i64) -> bool {
    let reach = CLEAR_DISTANCE / CHUNK_SIZE as f32;
    built.iter().any(|&(bx, by)| {
        let (dx, dy) = ((bx - x) as f32, (by - y) as f32);
        dx * dx + dy * dy <= reach * reach
    })
}

/// How streaming has gone, for the client's readout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StreamStats {
    /// Chunks loaded now.
    pub chunks: usize,
    /// Chunks within the load distance still waiting to be generated or brought in.
    pub waiting: usize,
    /// Chunks generated since the terrain was last made from scratch.
    pub generated: usize,
    /// Seconds the worker threads spent generating and lighting them, added up.
    pub seconds: f32,
    /// Seconds the simulation spent bringing them in: mostly light across their seams.
    pub join_seconds: f32,
}

/// The generator's tops the worker threads have worked out, by epoch and chunk X and Y, so the
/// chunks of a column don't each ask the generator again.
type TopsCache = Arc<Mutex<HashMap<(u64, i64, i64), Arc<TopsArray>>>>;

/// A chunk to generate.
struct Job {
    epoch: u64,
    pos: ChunkPos,
    generator: SharedGenerator,
    center: I64Vec3,
    blocks: Arc<BlockRegistry>,
    store: Arc<dyn ChunkStore>,
    /// The simulation's tops for the chunk's column and the four beside it, in [`AROUND`]'s
    /// order, where it has them.
    tops: [Option<Arc<TopsArray>>; 5],
    cache: TopsCache,
}

/// What a worker made of a chunk.
pub(crate) struct Made {
    /// The generated chunk, lit alone; `None` if the store has the chunk.
    pub chunk: Option<ChunkData>,
    /// The chunk the store has, by name: the simulation names its blocks, since blocks made
    /// while the game runs are newer than the workers' registry, and lights it.
    pub stored: Option<SavedChunk>,
    /// The tops the chunk was lit by, for its column and the four beside it, in [`AROUND`]'s
    /// order.
    pub tops: [Arc<TopsArray>; 5],
}

/// A chunk back from a worker.
pub(crate) struct Done {
    pub epoch: u64,
    pub pos: ChunkPos,
    pub made: Result<Made, ApiError>,
    pub seconds: f32,
}

/// Work for the terrain threads: generating a chunk, or a far chunk (see [`crate::far`]).
/// Each job sends its own result back.
pub(crate) type Task = Box<dyn FnOnce() + Send>;

/// Threads that generate terrain. They stop once the simulation is dropped.
pub(crate) struct Workers {
    jobs: Sender<Task>,
    pub count: usize,
}

impl Workers {
    fn start() -> Self {
        // Half the cores, less one: the game, its renderer and the client's background meshing
        // want the rest.
        let count = (std::thread::available_parallelism().map_or(2, |n| n.get()) / 2).saturating_sub(1).max(1);
        let (jobs, inbox) = channel::<Task>();
        let inbox = Arc::new(Mutex::new(inbox));
        for i in 0..count {
            let inbox = Arc::clone(&inbox);
            std::thread::Builder::new()
                .name(format!("terrain {i}"))
                .spawn(move || {
                    loop {
                        // The lock is held only while waiting for the next job.
                        let task = match inbox.lock() {
                            Ok(inbox) => inbox.recv(),
                            Err(_) => return,
                        };
                        let Ok(task) = task else { return };
                        task();
                    }
                })
                .expect("starting a terrain thread");
        }
        Self { jobs, count }
    }

    /// Hands a job to the threads; `false` if they've stopped.
    pub fn send(&self, task: Task) -> bool {
        self.jobs.send(task).is_ok()
    }
}

/// Makes the chunk for `job`, and sends it to `outbox`.
fn run_job(job: Job, outbox: &Sender<Done>) {
    let start = Instant::now();
    // A generator that panics fails its chunk, not the thread.
    let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| make(&job)))
        .unwrap_or_else(|_| Err(ApiError::InvalidDef(format!("generating chunk {:?} panicked", job.pos.0.to_array()))));
    let seconds = start.elapsed().as_secs_f32();
    let _ = outbox.send(Done { epoch: job.epoch, pos: job.pos, made, seconds });
}

/// The generator's own tops at chunk `x, y`, in world boxels: from the cache, or worked out
/// and kept there.
fn generated_tops(job: &Job, generator: &dyn Generator, x: i64, y: i64) -> Arc<TopsArray> {
    let key = (job.epoch, x, y);
    if let Some(tops) = job.cache.lock().ok().and_then(|cache| cache.get(&key).cloned()) {
        return tops;
    }
    let tops = Arc::new(world_tops(generator, job.center, x, y));
    if let Ok(mut cache) = job.cache.lock() {
        cache.insert(key, Arc::clone(&tops));
    }
    tops
}

/// The tops `generator` gives at chunk `x, y` for a terrain centered at `center`, in world
/// boxels.
pub(crate) fn world_tops(generator: &dyn Generator, center: I64Vec3, x: i64, y: i64) -> TopsArray {
    let mut tops = [NO_TOP; TOPS_LEN];
    generator.tops(x * CHUNK_SIZE - center.x, y * CHUNK_SIZE - center.y, 1, &mut tops);
    for top in tops.iter_mut().filter(|t| **t != NO_TOP) {
        *top += center.z;
    }
    tops
}

/// The chunk the store has for `job`, or the generated one lit alone.
fn make(job: &Job) -> Result<Made, ApiError> {
    let generator = job.generator.read().map_err(|_| ApiError::InvalidDef("a terrain generator panicked".into()))?;
    let pos = job.pos;
    let (x, y) = (pos.0.x, pos.0.y);
    let stored = match job.store.read(pos).map_err(ApiError::Storage)? {
        Some(bytes) => Some(SavedChunk::from_bytes(&bytes).map_err(|e| ApiError::Storage(format!("chunk {:?}: {e}", pos.0.to_array())))?),
        None => None,
    };
    // The tops to light by: the simulation's, or as stored, or else the generator's.
    let mut tops: Vec<Arc<TopsArray>> = Vec::with_capacity(AROUND.len());
    for ((dx, dy), given) in AROUND.iter().zip(&job.tops) {
        let (x, y) = (x + dx, y + dy);
        tops.push(match given {
            Some(tops) => Arc::clone(tops),
            None => match job.store.read(crate::tops::store_key(x, y)).map_err(ApiError::Storage)? {
                Some(bytes) => Arc::new(crate::tops::from_bytes(&bytes).map_err(ApiError::Storage)?),
                None => generated_tops(job, &**generator, x, y),
            },
        });
    }
    let tops: [Arc<TopsArray>; 5] = tops.try_into().expect("five columns");
    if stored.is_some() {
        return Ok(Made { chunk: None, stored, tops });
    }
    // Over everything the generator fills in this column there's nothing to ask it for.
    let bottom = pos.0.z * CHUNK_SIZE;
    let mut chunk = if generated_tops(job, &**generator, x, y).iter().all(|&top| top < bottom) {
        ChunkData { pos, lists: vec![Vec::<Occupant>::new()], cells: Layer::Uniform(0), light: Layer::Uniform(Light::SKY), records: Vec::new() }
    } else {
        let mut fill = ChunkFill::default();
        generator.fill(pos.origin().0 - job.center, &mut fill);
        crate::terrain::to_chunk(pos, &fill, &job.blocks)?
    };
    let mut around = Tops::default();
    for ((dx, dy), tops) in AROUND.iter().zip(&tops) {
        around.set_column(x + dx, y + dy, Arc::clone(tops));
    }
    crate::light::light_alone(&mut chunk, &around, &job.blocks);
    Ok(Made { chunk: Some(chunk), stored: None, tops })
}

/// Which chunks are loaded, which are wanted and which are on their way; the store edited
/// chunks that are dropped go to; and the worker threads.
pub struct Stream {
    /// Meters around each player to keep loaded.
    pub radius: f32,
    loaded: HashSet<ChunkPos>,
    /// Asked of the workers and not back yet.
    in_flight: HashSet<ChunkPos>,
    /// Back from the workers and not brought in yet, oldest first.
    ready: std::collections::VecDeque<Done>,
    /// Chunks within the radius of a center, nearest first, as last worked out, with how far
    /// each is from the nearest center.
    wanted: Vec<ChunkPos>,
    wanted_distance: Vec<f32>,
    wanted_set: HashSet<ChunkPos>,
    /// How many of `wanted` from the start are all loaded: where looking for missing ones
    /// starts, so it isn't the whole list for every chunk brought in.
    loaded_up_to: usize,
    /// What `wanted` was worked out for: each center's chunk, the radius and the epoch.
    wanted_for: Option<(Vec<[i64; 3]>, u32, u64)>,
    /// Where edited chunks that are dropped go, to come back when they load again.
    store: Arc<dyn ChunkStore>,
    /// Columns of chunks the mods built, which keep terrain away.
    built: Vec<(i64, i64)>,
    /// Goes up whenever the terrain is made from scratch, so chunks generated before are
    /// thrown away when they come back.
    epoch: u64,
    workers: Option<Workers>,
    /// Where the workers send generated chunks, and where they come back.
    outbox: Sender<Done>,
    done: Receiver<Done>,
    /// The registry the workers name blocks from: a copy taken when they start. Terrain only
    /// uses blocks the mods registered, which never change.
    blocks: Option<Arc<BlockRegistry>>,
    cache: TopsCache,
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
pub const DEFAULT_RADIUS: f32 = 256.0 + CHUNK_SIZE as f32;

/// How many chunks each worker may have waiting: enough that the threads never run dry
/// between ticks, few enough that a move toward new ground is answered soon.
const QUEUED_PER_WORKER: usize = 16;

impl Stream {
    /// A stream keeping `radius` meters around each player.
    pub fn new(radius: f32) -> Self {
        let (outbox, done) = channel();
        Self {
            outbox,
            done,
            radius,
            loaded: HashSet::new(),
            in_flight: HashSet::new(),
            ready: std::collections::VecDeque::new(),
            wanted: Vec::new(),
            wanted_distance: Vec::new(),
            wanted_set: HashSet::new(),
            loaded_up_to: 0,
            wanted_for: None,
            store: Arc::new(MemoryStore::default()),
            built: Vec::new(),
            epoch: 0,
            workers: None,
            blocks: None,
            cache: TopsCache::default(),
            loaded_now: Vec::new(),
            unloaded_now: Vec::new(),
            stats: StreamStats::default(),
            error: None,
        }
    }

    /// Notes which columns the mods built on, to keep terrain away from them.
    pub fn set_built(&mut self, grid: &crate::Grid) {
        let mut columns: Vec<(i64, i64)> = grid.chunks().map(|(pos, _)| (pos.0.x, pos.0.y)).collect();
        columns.sort_unstable();
        columns.dedup();
        self.built = columns;
    }

    /// Whether the column at chunk `(x, y)` is too near something the mods built for terrain.
    pub fn cleared(&self, x: i64, y: i64) -> bool {
        cleared_by(&self.built, x, y)
    }

    /// The columns of chunks the mods built on, which keep terrain away.
    pub fn built(&self) -> &[(i64, i64)] {
        &self.built
    }

    /// Whether the chunk at `pos` is loaded terrain.
    pub fn is_loaded(&self, pos: ChunkPos) -> bool {
        self.loaded.contains(&pos)
    }

    /// The loaded chunks, in no particular order.
    pub fn loaded(&self) -> impl Iterator<Item = ChunkPos> + '_ {
        self.loaded.iter().copied()
    }

    /// Where edited chunks that are dropped go.
    pub fn store(&self) -> &Arc<dyn ChunkStore> {
        &self.store
    }

    /// Keeps edited chunks in `store` from now on. Chunks asked for already read the old one.
    pub fn set_store(&mut self, store: Arc<dyn ChunkStore>) {
        self.store = store;
    }

    /// Forgets every chunk, loaded or on its way, so the terrain is made again from scratch.
    /// Returns the chunks that were loaded.
    pub fn reset(&mut self) -> Vec<ChunkPos> {
        self.epoch += 1;
        self.in_flight.clear();
        self.ready.clear();
        self.wanted_for = None;
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
        }
        self.stats = StreamStats { chunks: self.loaded.len(), ..StreamStats::default() };
        self.error = None;
        let mut chunks: Vec<ChunkPos> = self.loaded.iter().copied().collect();
        chunks.sort_unstable_by_key(|p| p.0.to_array());
        chunks
    }

    /// Chunks within the radius of any of `centers`, nearest first, leaving out cleared ones.
    pub fn wanted(&mut self, centers: &[WorldPos]) -> &[ChunkPos] {
        let key = (centers.iter().map(|c| c.boxel().chunk().0.to_array()).collect(), self.radius.to_bits(), self.epoch);
        if self.wanted_for.as_ref() != Some(&key) {
            let reach = (self.radius / CHUNK_SIZE as f32).ceil() as i64 + 1;
            let mut wanted: HashMap<ChunkPos, f32> = HashMap::new();
            for &center in centers {
                let chunk = center.boxel().chunk().0;
                for y in chunk.y - reach..=chunk.y + reach {
                    for x in chunk.x - reach..=chunk.x + reach {
                        if self.cleared(x, y) {
                            continue;
                        }
                        for z in chunk.z - reach..=chunk.z + reach {
                            let pos = ChunkPos(I64Vec3::new(x, y, z));
                            let d = chunk_distance(pos, center);
                            if d <= self.radius {
                                let nearest = wanted.entry(pos).or_insert(d);
                                *nearest = nearest.min(d);
                            }
                        }
                    }
                }
            }
            let mut wanted: Vec<(f32, ChunkPos)> = wanted.into_iter().map(|(pos, d)| (d, pos)).collect();
            wanted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.0.to_array().cmp(&b.1.0.to_array())));
            self.wanted_distance = wanted.iter().map(|(d, _)| *d).collect();
            self.wanted = wanted.into_iter().map(|(_, pos)| pos).collect();
            self.wanted_set = self.wanted.iter().copied().collect();
            self.wanted_for = Some(key);
            self.loaded_up_to = 0;
            self.stats.waiting = self.wanted.iter().filter(|p| !self.loaded.contains(p)).count();
        }
        &self.wanted
    }

    /// Loaded chunks that have drifted past the radius and its margin from every one of
    /// `centers`.
    pub fn too_far(&self, centers: &[WorldPos]) -> Vec<ChunkPos> {
        let mut far: Vec<ChunkPos> = self.loaded.iter().copied().filter(|&pos| nearest(pos, centers) > self.radius + UNLOAD_MARGIN).collect();
        far.sort_unstable_by_key(|p| p.0.to_array());
        far
    }

    /// Notes a chunk as dropped.
    pub fn drop_chunk(&mut self, pos: ChunkPos) {
        self.loaded.remove(&pos);
        self.unloaded_now.push(pos);
        self.stats.chunks = self.loaded.len();
    }

    /// Notes a chunk as brought in, with the seconds making it took.
    pub fn add_chunk(&mut self, pos: ChunkPos, seconds: f32) {
        if self.loaded.insert(pos) && self.wanted_set.contains(&pos) {
            self.stats.waiting = self.stats.waiting.saturating_sub(1);
        }
        self.loaded_now.push(pos);
        self.stats.generated += 1;
        self.stats.seconds += seconds;
        self.stats.chunks = self.loaded.len();
    }

    /// Moves [`Stream::loaded_up_to`] past the wanted chunks loaded since.
    fn skip_loaded(&mut self) {
        while self.wanted.get(self.loaded_up_to).is_some_and(|p| self.loaded.contains(p)) {
            self.loaded_up_to += 1;
        }
    }

    /// Whether a wanted chunk within `meters` of the centers it was last worked out for isn't
    /// loaded yet.
    pub fn missing_within(&mut self, meters: f32) -> bool {
        self.skip_loaded();
        let (wanted, distance) = (&self.wanted[self.loaded_up_to..], &self.wanted_distance[self.loaded_up_to..]);
        wanted.iter().zip(distance).take_while(|(_, d)| **d <= meters).any(|(p, _)| !self.loaded.contains(p))
    }

    /// Whether any chunk is on its way from the workers, or back and not brought in.
    pub fn busy(&self) -> bool {
        !self.in_flight.is_empty() || !self.ready.is_empty()
    }

    /// Asks the workers for wanted chunks that aren't loaded or on their way, nearest first,
    /// keeping a few per worker queued. `tops` are the simulation's, to light them by.
    pub fn request(&mut self, generator: &SharedGenerator, terrain: &TerrainDef, blocks: &BlockRegistry, tops: &Tops) {
        self.skip_loaded();
        if self.loaded_up_to == self.wanted.len() {
            return;
        }
        let workers = self.workers.get_or_insert_with(Workers::start);
        let room = (workers.count * QUEUED_PER_WORKER).saturating_sub(self.in_flight.len());
        if room == 0 {
            return;
        }
        let blocks = self.blocks.get_or_insert_with(|| Arc::new(blocks.clone()));
        let asked: Vec<ChunkPos> = self.wanted[self.loaded_up_to..]
            .iter()
            .copied()
            .filter(|c| !self.loaded.contains(c) && !self.in_flight.contains(c))
            .take(room)
            .collect();
        for pos in asked {
            let job = Job {
                epoch: self.epoch,
                pos,
                generator: Arc::clone(generator),
                center: terrain.center.0,
                blocks: Arc::clone(blocks),
                store: Arc::clone(&self.store),
                tops: AROUND.map(|(dx, dy)| tops.column(pos.0.x + dx, pos.0.y + dy).cloned()),
                cache: Arc::clone(&self.cache),
            };
            let outbox = self.outbox.clone();
            if workers.send(Box::new(move || run_job(job, &outbox))) {
                self.in_flight.insert(pos);
            }
        }
    }

    /// The next generated chunk to bring in, with ones from an older epoch or no longer
    /// wanted thrown away. Waits for one if `wait` and any are on their way.
    pub(crate) fn next_finished(&mut self, wait: bool) -> Option<Done> {
        if self.ready.is_empty() {
            let came = self.finished(wait);
            self.ready.extend(came);
        }
        // The player may have moved on since it came back.
        while let Some(done) = self.ready.pop_front() {
            if self.wanted_set.contains(&done.pos) && !self.loaded.contains(&done.pos) {
                return Some(done);
            }
        }
        None
    }

    /// Generated chunks that came back, with ones from an older epoch or no longer wanted
    /// thrown away. Waits for at least one if `wait` and any are on their way.
    fn finished(&mut self, wait: bool) -> Vec<Done> {
        let mut out: Vec<Done> = Vec::new();
        if wait && !self.in_flight.is_empty() {
            match self.done.recv_timeout(STALLED) {
                Ok(done) => out.push(done),
                Err(_) => {
                    // Nothing came back: ask again rather than wait forever.
                    self.in_flight.clear();
                    self.error = Some(ApiError::InvalidDef("generating terrain stalled".into()));
                }
            }
        }
        out.extend(self.done.try_iter());
        out.retain(|done| {
            if done.epoch != self.epoch {
                return false;
            }
            self.in_flight.remove(&done.pos);
            self.wanted_set.contains(&done.pos) && !self.loaded.contains(&done.pos)
        });
        out
    }

    /// Forgets the generator's tops the workers worked out at chunk `x, y`.
    pub(crate) fn forget_tops(&mut self, x: i64, y: i64) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(&(self.epoch, x, y));
        }
    }

    /// The terrain threads, started if they aren't, for far chunks to share.
    pub(crate) fn workers(&mut self) -> &Workers {
        self.workers.get_or_insert_with(Workers::start)
    }

    /// How many chunks are on their way from the workers.
    pub(crate) fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// Whether every wanted chunk is loaded.
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

    fn at(x: f32, y: f32, z: f32) -> WorldPos {
        let cell = I64Vec3::new(x.floor() as i64, y.floor() as i64, z.floor() as i64);
        WorldPos::new(cell, Vec3::new(x - x.floor(), y - y.floor(), z - z.floor()))
    }

    #[test]
    fn chunks_within_the_radius_come_nearest_first_in_every_direction() {
        let mut stream = Stream::new(48.0);
        let center = at(16.0, 16.0, 16.0);
        let wanted = stream.wanted(&[center]).to_vec();
        assert_eq!(wanted[0], ChunkPos::new(0, 0, 0), "the one the player is in first");
        // Middles within 48 m of (16, 16, 16): the chunk itself, the 6 a chunk away (32 m) and
        // the 12 across an edge (45 m); the 8 across a corner (55 m) are past it.
        assert_eq!(wanted.len(), 19);
        assert!(wanted.contains(&ChunkPos::new(0, 0, -1)) && wanted.contains(&ChunkPos::new(0, 0, 1)), "below and above");
        assert!(wanted.contains(&ChunkPos::new(1, 0, -1)) && !wanted.contains(&ChunkPos::new(1, 1, 1)));
        let d: Vec<f32> = wanted.iter().map(|&c| chunk_distance(c, center)).collect();
        assert!(d.windows(2).all(|w| w[0] <= w[1]), "{d:?}");
    }

    #[test]
    fn the_sphere_follows_the_player_down() {
        let mut stream = Stream::new(48.0);
        let wanted = stream.wanted(&[at(16.0, 16.0, -1024.0 + 16.0)]).to_vec();
        assert_eq!(wanted[0], ChunkPos::new(0, 0, -32));
        assert_eq!(wanted.len(), 19);
        assert!(wanted.iter().all(|c| (-33..=-31).contains(&c.0.z)));
    }

    #[test]
    fn several_centers_each_get_their_sphere() {
        let mut stream = Stream::new(48.0);
        let centers = [at(16.0, 16.0, 16.0), at(16.0 + 32.0 * 20.0, 16.0, 16.0)];
        let wanted = stream.wanted(&centers).to_vec();
        assert_eq!(wanted.len(), 38, "two spheres apart");
        assert!(wanted[..2].contains(&ChunkPos::new(0, 0, 0)) && wanted[..2].contains(&ChunkPos::new(20, 0, 0)), "nearest to either first");
        // Close together they share chunks, each wanted once.
        let wanted = stream.wanted(&[centers[0], at(48.0, 16.0, 16.0)]).to_vec();
        let distinct: HashSet<ChunkPos> = wanted.iter().copied().collect();
        assert_eq!(distinct.len(), wanted.len());
        assert!(wanted.len() > 19 && wanted.len() < 38);
        // A chunk is dropped only once it's far from both.
        stream.add_chunk(ChunkPos::new(20, 0, 0), 0.0);
        assert!(stream.too_far(&centers).is_empty());
        assert_eq!(stream.too_far(&centers[..1]), [ChunkPos::new(20, 0, 0)]);
    }

    #[test]
    fn chunks_are_dropped_only_past_the_margin() {
        let mut stream = Stream::new(48.0);
        let pos = ChunkPos::new(0, 0, 0);
        stream.add_chunk(pos, 0.0);
        // Its middle is at (16, 16, 16): 64 m away is past the radius but within the margin.
        assert!(stream.too_far(&[at(80.0, 16.0, 16.0)]).is_empty());
        assert_eq!(stream.too_far(&[at(16.0 + 48.0 + UNLOAD_MARGIN + 1.0, 16.0, 16.0)]), [pos]);
        assert_eq!(stream.too_far(&[at(16.0, 16.0, 16.0 - 48.0 - UNLOAD_MARGIN - 1.0)]), [pos], "straight down counts too");
    }

    #[test]
    fn built_chunks_keep_terrain_away() {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(api::BlockDef::new("t:stone", true, [0; 3])).unwrap();
        let mut grid = crate::Grid::default();
        grid.set_block(BoxelPos::new(5, 5, 5), stone);
        let mut stream = Stream::new(64.0);
        stream.set_built(&grid);
        assert!(stream.cleared(0, 0));
        assert!(stream.cleared(31, 0), "992 m away");
        assert!(!stream.cleared(33, 0), "1056 m away");
        assert!(stream.wanted(&[at(16.0, 16.0, 0.0)]).is_empty());
        assert!(stream.wanted(&[at(16.0, 16.0, 5000.0)]).is_empty(), "however high or deep");
        assert!(!stream.wanted(&[at(16.0, 40_000.0, 0.0)]).is_empty());
    }
}
