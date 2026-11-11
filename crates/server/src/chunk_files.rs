//! A world's chunk store on disk (see [`sim::store`]): region files of 16×16×16 chunks in the
//! world's folder, `chunks/` for the saved layer and `unsaved/` for chunks put since the last
//! save.
//!
//! A region file is `r.<x>.<y>.<z>.bin`: `BOXELRGN`, a version, how many chunks, then each
//! chunk's place in the region and length, then the chunks' bytes in that order. A region is
//! always written whole, to a `.part` file renamed over the old one, so a crash halfway leaves
//! the old region.
//!
//! Puts are kept in memory until a writer thread has written them to `unsaved/`, so a tick never
//! waits on the disk; reads see them at once. Committing, discarding and forgetting go through
//! the same thread, in order, and wait for it.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use api::{ChunkPos, I64Vec3};
use sim::ChunkStore;

/// Chunks along each side of a region.
const REGION: i64 = 16;
const MAGIC: &[u8; 8] = b"BOXELRGN";
const VERSION: u32 = 1;
/// The most chunks a region holds.
const REGION_CHUNKS: usize = (REGION * REGION * REGION) as usize;

/// Which layer a region file belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Layer {
    Saved,
    Unsaved,
}

impl Layer {
    fn folder(self) -> &'static str {
        match self {
            Layer::Saved => "chunks",
            Layer::Unsaved => "unsaved",
        }
    }
}

/// A region's place, in regions.
type Region = [i64; 3];

fn region_of(pos: ChunkPos) -> (Region, u16) {
    let p = pos.0;
    let region = [p.x.div_euclid(REGION), p.y.div_euclid(REGION), p.z.div_euclid(REGION)];
    let (x, y, z) = (p.x.rem_euclid(REGION), p.y.rem_euclid(REGION), p.z.rem_euclid(REGION));
    (region, (x + REGION * (y + REGION * z)) as u16)
}

fn chunk_at(region: Region, local: u16) -> ChunkPos {
    let l = local as i64;
    let local = I64Vec3::new(l % REGION, l / REGION % REGION, l / (REGION * REGION));
    ChunkPos(I64Vec3::from_array(region) * REGION + local)
}

/// Where each chunk of a region file sits in it: its place in the region, offset and length.
type Index = HashMap<u16, (u64, u32)>;

/// A world's chunks on disk. Dropping it finishes writing what was put first.
pub struct ChunkFiles {
    shared: Arc<Shared>,
    jobs: Mutex<Sender<Job>>,
    writer: Option<JoinHandle<()>>,
}

struct Shared {
    folder: PathBuf,
    state: Mutex<State>,
    /// Held to read a region file, and by the writer while it swaps one in or deletes one, so
    /// a read never sees half a change. The region indexes read so far, by layer.
    files: Mutex<HashMap<(Layer, Region), Arc<Index>>>,
}

#[derive(Default)]
struct State {
    /// Put and not written yet: each with the number of the put that made it, so the writer
    /// only lets go of what it wrote.
    pending: HashMap<ChunkPos, (u64, Arc<Vec<u8>>)>,
    puts: u64,
    /// The saved layer is hidden (see [`ChunkStore::forget`]).
    forgotten: bool,
    /// Why writing failed since the last commit, if it did.
    error: Option<String>,
}

enum Job {
    /// Write the pending chunks of these regions.
    Write(Vec<Region>),
    Forget(Sender<Result<(), String>>),
    Commit(Sender<Result<(), String>>),
    Discard(Sender<Result<(), String>>),
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Nothing holding these locks leaves its data half changed when it panics.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl ChunkFiles {
    /// The chunks kept in `folder`, a world's folder. Nothing is read until asked for.
    pub fn open(folder: &Path) -> ChunkFiles {
        let shared = Arc::new(Shared { folder: folder.to_owned(), state: Mutex::default(), files: Mutex::default() });
        let (jobs, inbox) = channel();
        let for_writer = Arc::clone(&shared);
        let writer = std::thread::Builder::new()
            .name("chunk files".into())
            .spawn(move || for_writer.run(inbox))
            .expect("starting the chunk files thread");
        ChunkFiles { shared, jobs: Mutex::new(jobs), writer: Some(writer) }
    }

    /// Asks the writer to do `job` and waits for its answer.
    fn ask(&self, job: impl FnOnce(Sender<Result<(), String>>) -> Job) -> Result<(), String> {
        let (reply, answer) = channel();
        lock(&self.jobs).send(job(reply)).map_err(|_| "the chunk files thread stopped".to_owned())?;
        answer.recv().map_err(|_| "the chunk files thread stopped".to_owned())?
    }
}

impl Drop for ChunkFiles {
    fn drop(&mut self) {
        // Closing the channel lets the writer finish what's queued and stop.
        drop(std::mem::replace(&mut *lock(&self.jobs), channel().0));
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

impl ChunkStore for ChunkFiles {
    fn read(&self, pos: ChunkPos) -> Result<Option<Vec<u8>>, String> {
        let forgotten = {
            let state = lock(&self.shared.state);
            if let Some((_, bytes)) = state.pending.get(&pos) {
                return Ok(Some(bytes.to_vec()));
            }
            state.forgotten
        };
        let mut files = lock(&self.shared.files);
        let (region, local) = region_of(pos);
        if let Some(bytes) = self.shared.read_chunk(&mut files, Layer::Unsaved, region, local)? {
            return Ok(Some(bytes));
        }
        if forgotten {
            return Ok(None);
        }
        self.shared.read_chunk(&mut files, Layer::Saved, region, local)
    }

    fn put(&self, chunks: Vec<(ChunkPos, Vec<u8>)>) {
        let mut regions: Vec<Region> = Vec::new();
        {
            let mut state = lock(&self.shared.state);
            for (pos, bytes) in chunks {
                state.puts += 1;
                let n = state.puts;
                state.pending.insert(pos, (n, Arc::new(bytes)));
                let region = region_of(pos).0;
                if !regions.contains(&region) {
                    regions.push(region);
                }
            }
        }
        if !regions.is_empty() && lock(&self.jobs).send(Job::Write(regions)).is_err() {
            lock(&self.shared.state).error.get_or_insert_with(|| "the chunk files thread stopped".into());
        }
    }

    fn forget(&self) -> Result<(), String> {
        self.ask(Job::Forget)
    }

    fn commit(&self) -> Result<(), String> {
        self.ask(Job::Commit)
    }

    fn discard(&self) -> Result<(), String> {
        self.ask(Job::Discard)
    }

    fn positions(&self) -> Result<Vec<ChunkPos>, String> {
        let (mut out, forgotten): (Vec<ChunkPos>, bool) = {
            let state = lock(&self.shared.state);
            (state.pending.keys().copied().collect(), state.forgotten)
        };
        let mut files = lock(&self.shared.files);
        let mut layers = vec![Layer::Unsaved];
        if !forgotten {
            layers.push(Layer::Saved);
        }
        for layer in layers {
            for region in self.shared.regions(layer)? {
                let index = self.shared.index(&mut files, layer, region)?;
                out.extend(index.keys().map(|&local| chunk_at(region, local)));
            }
        }
        out.sort_by_key(|p| p.0.to_array());
        out.dedup();
        Ok(out)
    }
}

impl Shared {
    fn region_path(&self, layer: Layer, region: Region) -> PathBuf {
        let [x, y, z] = region;
        self.folder.join(layer.folder()).join(format!("r.{x}.{y}.{z}.bin"))
    }

    /// Every region file of `layer`.
    fn regions(&self, layer: Layer) -> Result<Vec<Region>, String> {
        let dir = self.folder.join(layer.folder());
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("{}: {e}", dir.display())),
        };
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(parts) = name.strip_prefix("r.").and_then(|n| n.strip_suffix(".bin")) else { continue };
            let coords: Vec<i64> = parts.split('.').filter_map(|c| c.parse().ok()).collect();
            if let [x, y, z] = coords[..] {
                out.push([x, y, z]);
            }
        }
        out.sort_unstable();
        Ok(out)
    }

    /// The index of a region file, read once and kept; empty if there's no such file.
    fn index(&self, files: &mut HashMap<(Layer, Region), Arc<Index>>, layer: Layer, region: Region) -> Result<Arc<Index>, String> {
        if let Some(index) = files.get(&(layer, region)) {
            return Ok(Arc::clone(index));
        }
        let path = self.region_path(layer, region);
        let index = match std::fs::File::open(&path) {
            Ok(mut file) => {
                let len = file.metadata().map_err(|e| format!("{}: {e}", path.display()))?.len();
                read_index(&mut file, len).map_err(|e| format!("{}: {e}", path.display()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Index::new(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let index = Arc::new(index);
        files.insert((layer, region), Arc::clone(&index));
        Ok(index)
    }

    fn read_chunk(
        &self,
        files: &mut HashMap<(Layer, Region), Arc<Index>>,
        layer: Layer,
        region: Region,
        local: u16,
    ) -> Result<Option<Vec<u8>>, String> {
        let index = self.index(files, layer, region)?;
        let Some(&(offset, len)) = index.get(&local) else { return Ok(None) };
        let path = self.region_path(layer, region);
        let at = |e: std::io::Error| format!("{}: {e}", path.display());
        let mut file = std::fs::File::open(&path).map_err(at)?;
        file.seek(SeekFrom::Start(offset)).map_err(at)?;
        let mut bytes = vec![0; len as usize];
        file.read_exact(&mut bytes).map_err(at)?;
        Ok(Some(bytes))
    }

    /// Every chunk of a region file, by its place in the region.
    fn read_region(&self, layer: Layer, region: Region) -> Result<HashMap<u16, Vec<u8>>, String> {
        let path = self.region_path(layer, region);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let index = read_index(&mut std::io::Cursor::new(&bytes), bytes.len() as u64).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(index.into_iter().map(|(local, (offset, len))| (local, bytes[offset as usize..][..len as usize].to_vec())).collect())
    }

    /// Writes a region file whole and swaps it in, or deletes it if it holds nothing.
    fn write_region(&self, layer: Layer, region: Region, chunks: &HashMap<u16, Vec<u8>>) -> Result<(), String> {
        let path = self.region_path(layer, region);
        let at = |e: std::io::Error| format!("{}: {e}", path.display());
        if chunks.is_empty() {
            let mut files = lock(&self.files);
            files.remove(&(layer, region));
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(at(e)),
                _ => Ok(()),
            };
        }
        let (bytes, index) = region_bytes(chunks);
        std::fs::create_dir_all(path.parent().expect("in a folder")).map_err(at)?;
        let part = path.with_extension("bin.part");
        std::fs::write(&part, bytes).map_err(at)?;
        let mut files = lock(&self.files);
        std::fs::rename(&part, &path).map_err(at)?;
        files.insert((layer, region), Arc::new(index));
        Ok(())
    }

    /// The writer thread: does each job in turn, until the store is dropped.
    fn run(&self, inbox: Receiver<Job>) {
        while let Ok(job) = inbox.recv() {
            match job {
                Job::Write(regions) => {
                    // Writes queued behind this one go in the same pass.
                    let mut all = regions;
                    let mut later = Vec::new();
                    for next in inbox.try_iter() {
                        match next {
                            Job::Write(more) => all.extend(more),
                            other => {
                                later.push(other);
                                break;
                            }
                        }
                    }
                    all.sort_unstable();
                    all.dedup();
                    self.write_pending(&all);
                    for job in later {
                        self.answer(job);
                    }
                }
                other => self.answer(other),
            }
        }
        // Closed: what's still pending is written before the thread stops.
        let regions: Vec<Region> = {
            let state = lock(&self.state);
            let mut r: Vec<Region> = state.pending.keys().map(|&p| region_of(p).0).collect();
            r.sort_unstable();
            r.dedup();
            r
        };
        self.write_pending(&regions);
    }

    fn answer(&self, job: Job) {
        let (reply, result) = match job {
            Job::Write(regions) => {
                self.write_pending(&regions);
                return;
            }
            Job::Forget(reply) => (reply, self.forget()),
            Job::Commit(reply) => (reply, self.commit()),
            Job::Discard(reply) => (reply, self.discard()),
        };
        let _ = reply.send(result);
    }

    /// Writes the pending chunks of `regions` into their unsaved region files, then lets go of
    /// the ones no newer put replaced meanwhile.
    fn write_pending(&self, regions: &[Region]) {
        for &region in regions {
            let taken: Vec<(ChunkPos, u64, Arc<Vec<u8>>)> = {
                let state = lock(&self.state);
                state
                    .pending
                    .iter()
                    .filter(|(pos, _)| region_of(**pos).0 == region)
                    .map(|(&pos, (n, bytes))| (pos, *n, Arc::clone(bytes)))
                    .collect()
            };
            if taken.is_empty() {
                continue;
            }
            let written = self.read_region(Layer::Unsaved, region).and_then(|mut chunks| {
                for (pos, _, bytes) in &taken {
                    chunks.insert(region_of(*pos).1, bytes.to_vec());
                }
                self.write_region(Layer::Unsaved, region, &chunks)
            });
            let mut state = lock(&self.state);
            match written {
                Ok(()) => {
                    for (pos, n, _) in taken {
                        if state.pending.get(&pos).is_some_and(|(m, _)| *m == n) {
                            state.pending.remove(&pos);
                        }
                    }
                }
                // Kept pending, so reads still see them; the next commit reports it.
                Err(e) => {
                    state.error.get_or_insert(e);
                }
            }
        }
    }

    /// Deletes every region file of `layer`.
    fn clear(&self, layer: Layer) -> Result<(), String> {
        for region in self.regions(layer)? {
            self.write_region(layer, region, &HashMap::new())?;
        }
        Ok(())
    }

    fn forget(&self) -> Result<(), String> {
        self.clear(Layer::Unsaved)?;
        let mut state = lock(&self.state);
        state.pending.clear();
        state.forgotten = true;
        Ok(())
    }

    fn discard(&self) -> Result<(), String> {
        {
            let mut state = lock(&self.state);
            state.pending.clear();
            state.forgotten = false;
            state.error = None;
        }
        self.clear(Layer::Unsaved)
    }

    fn commit(&self) -> Result<(), String> {
        let regions: Vec<Region> = {
            let state = lock(&self.state);
            let mut r: Vec<Region> = state.pending.keys().map(|&p| region_of(p).0).collect();
            r.sort_unstable();
            r.dedup();
            r
        };
        self.write_pending(&regions);
        if let Some(e) = lock(&self.state).error.take() {
            return Err(e);
        }
        if lock(&self.state).forgotten {
            self.clear(Layer::Saved)?;
            lock(&self.state).forgotten = false;
        }
        for region in self.regions(Layer::Unsaved)? {
            let mut chunks = self.read_region(Layer::Saved, region)?;
            chunks.extend(self.read_region(Layer::Unsaved, region)?);
            self.write_region(Layer::Saved, region, &chunks)?;
            self.write_region(Layer::Unsaved, region, &HashMap::new())?;
        }
        Ok(())
    }
}

/// A region file's bytes, and its index.
fn region_bytes(chunks: &HashMap<u16, Vec<u8>>) -> (Vec<u8>, Index) {
    let mut locals: Vec<u16> = chunks.keys().copied().collect();
    locals.sort_unstable();
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(locals.len() as u32).to_le_bytes());
    for local in &locals {
        out.extend_from_slice(&local.to_le_bytes());
        out.extend_from_slice(&(chunks[local].len() as u32).to_le_bytes());
    }
    let mut index = Index::new();
    for local in locals {
        index.insert(local, (out.len() as u64, chunks[&local].len() as u32));
        out.extend_from_slice(&chunks[&local]);
    }
    (out, index)
}

/// Reads a region file's index from its start, checking it fits a file `len` bytes long.
fn read_index(file: &mut impl Read, len: u64) -> Result<Index, String> {
    let mut head = [0u8; 16];
    file.read_exact(&mut head).map_err(|_| "a region file that ends early")?;
    if &head[..8] != MAGIC {
        return Err("not a region file".into());
    }
    let version = u32::from_le_bytes(head[8..12].try_into().expect("4 bytes"));
    if version != VERSION {
        return Err(format!("a region file from version {version}; this game reads version {VERSION}"));
    }
    let count = u32::from_le_bytes(head[12..16].try_into().expect("4 bytes")) as usize;
    if count > REGION_CHUNKS {
        return Err(format!("a region file claiming {count} chunks"));
    }
    let mut entries = vec![0u8; count * 6];
    file.read_exact(&mut entries).map_err(|_| "a region file that ends early")?;
    let mut offset = 16 + entries.len() as u64;
    let mut index = Index::new();
    for entry in entries.as_chunks::<6>().0 {
        let local = u16::from_le_bytes([entry[0], entry[1]]);
        let n = u32::from_le_bytes([entry[2], entry[3], entry[4], entry[5]]);
        if local as usize >= REGION_CHUNKS || index.insert(local, (offset, n)).is_some() {
            return Err("a region file with a chunk out of place".into());
        }
        offset += n as u64;
    }
    if offset != len {
        return Err("a region file whose chunks don't fill it".into());
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("boxel-chunk-files-test-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn places_in_regions_go_both_ways() {
        for p in [ChunkPos::new(0, 0, 0), ChunkPos::new(-1, -16, 15), ChunkPos::new(17, -33, -300), ChunkPos::new(i32::MAX as i64, 5, -5)] {
            let (region, local) = region_of(p);
            assert_eq!(chunk_at(region, local), p);
            assert!((local as usize) < REGION_CHUNKS);
        }
    }

    #[test]
    fn chunk_files_keep_to_the_store_rules() {
        let dir = scratch("rules");
        sim::store::check_store(&ChunkFiles::open(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn saved_chunks_are_there_when_opened_again_and_unsaved_ones_wait_for_a_discard() {
        let dir = scratch("reopen");
        let (a, b) = (ChunkPos::new(3, -4, 1), ChunkPos::new(-20, 40, 0));
        {
            let files = ChunkFiles::open(&dir);
            files.put(vec![(a, vec![1, 2, 3])]);
            files.commit().unwrap();
            files.put(vec![(b, vec![4])]);
        }
        assert!(dir.join("chunks").read_dir().unwrap().count() == 1, "one saved region");
        let files = ChunkFiles::open(&dir);
        assert_eq!(files.read(a).unwrap(), Some(vec![1, 2, 3]));
        assert_eq!(files.read(b).unwrap(), Some(vec![4]), "written out when dropped");
        files.discard().unwrap();
        assert_eq!(files.read(b).unwrap(), None);
        assert!(!dir.join("unsaved").exists() || dir.join("unsaved").read_dir().unwrap().count() == 0);
        drop(files);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn many_puts_end_up_on_disk_and_out_of_memory() {
        let dir = scratch("many");
        let files = ChunkFiles::open(&dir);
        let chunks: Vec<(ChunkPos, Vec<u8>)> = (0..300).map(|i| (ChunkPos::new(i * 7, -i, i % 5), vec![i as u8; 100])).collect();
        for c in chunks.chunks(10) {
            files.put(c.to_vec());
        }
        files.commit().unwrap();
        assert!(lock(&files.shared.state).pending.is_empty(), "nothing left in memory");
        for (pos, bytes) in &chunks {
            assert_eq!(files.read(*pos).unwrap().as_ref(), Some(bytes));
        }
        assert_eq!(files.positions().unwrap().len(), 300);
        drop(files);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_damaged_region_is_an_error_and_isnt_saved_over() {
        let dir = scratch("damaged");
        let pos = ChunkPos::new(1, 1, 1);
        let path = {
            let files = ChunkFiles::open(&dir);
            files.put(vec![(pos, vec![9; 50])]);
            files.commit().unwrap();
            files.shared.region_path(Layer::Saved, region_of(pos).0)
        };
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.truncate(bytes.len() - 10);
        std::fs::write(&path, &bytes).unwrap();

        let files = ChunkFiles::open(&dir);
        assert!(files.read(pos).is_err(), "reading it fails");
        files.put(vec![(ChunkPos::new(2, 1, 1), vec![1])]);
        assert!(files.commit().is_err(), "saving into it fails");
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "and leaves it as it was");
        drop(files);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
