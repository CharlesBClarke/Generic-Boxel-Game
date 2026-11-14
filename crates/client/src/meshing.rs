//! Meshing in the background: chunks that stream in are meshed on threads of their own, from
//! a copy of the world kept from the same messages, so flying into new terrain never stalls a
//! frame. Terrain comes in a chunk at a time, and a chunk waits to be meshed until the 26
//! around it are in (their faces and corners read it), or [`NEIGHBOR_WAIT`] has passed, as at
//! the edge of what's loaded; one meshed without all its neighbors is meshed again when one
//! comes. So each chunk is meshed about once. Edits still remesh on the main thread right away (see
//! [`crate::editing::remesh_changed_chunks`]); a background mesh of a chunk remeshed there since
//! it was asked for is thrown away, by the chunk's version in [`ChunkMeshes`].

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use api::{ChunkPos, I64Vec3};
use bevy::prelude::*;
use sim::{BlockRegistry, ChunkData, Grid, LookRegistry};

use crate::editing::ChunkMeshes;
use crate::mesher::{AoTuning, MeshData};
use crate::volumes::Volumes;
use crate::world::ClientWorld;
use crate::{ChunkMesh, RenderOrigin};

/// What the main thread tells the background mesher, in order.
enum ToWorker {
    /// Start again from this world.
    Restart { chunks: Vec<ChunkData>, tops: sim::Tops, blocks: BlockRegistry, looks: LookRegistry },
    /// Chunks that arrived or changed.
    Chunks(Vec<ChunkData>),
    /// Chunks dropped.
    Gone(Vec<ChunkPos>),
    /// Tops that came, moved or were dropped (see [`sim::tops`]).
    Tops(Vec<sim::TopsNews>),
    /// The block registry grew, by crafting.
    Blocks(BlockRegistry),
    Settings { ao: AoTuning, merge: bool },
    /// Chunks to mesh, each with its version when asked.
    Mesh(Vec<(ChunkPos, u64)>),
    /// Far columns to mesh (see [`crate::far`]), each with its version, on their own: nothing
    /// beside a far column is read, so its sides get walls that plug the cracks between levels.
    Far(Vec<crate::far::Asked>),
}

/// What the thread sends back.
enum FromWorker {
    Near(Vec<Meshed>),
    Far(Vec<crate::far::FarMeshed>),
}

/// A chunk meshed in the background.
struct Meshed {
    pos: ChunkPos,
    version: u64,
    data: MeshData,
}

/// Seconds a chunk waits for its neighbors before it's meshed without them.
pub const NEIGHBOR_WAIT: f32 = 1.0;

/// Where a streamed chunk is in being meshed.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ChunkState {
    /// Waiting for its neighbors since then.
    Waiting(std::time::Instant),
    /// Meshed, with all its neighbors there or not.
    Meshed { whole: bool },
}

/// A streamed chunk: where it is in being meshed.
struct Streamed {
    state: ChunkState,
    /// Its mesh has come back once, so it can be drawn (meshed again later, its old mesh
    /// shows until the new one is in).
    shown: bool,
}

/// The background mesher's thread, and what's been asked of it.
#[derive(Resource)]
pub(crate) struct BackgroundMesher {
    /// Streamed chunks.
    chunks: HashMap<ChunkPos, Streamed>,
    /// Which chunk rows (Z) of each column of chunks hold a streamed chunk.
    columns: HashMap<(i64, i64), Vec<i64>>,
    /// The chunk rows that show each column's surface (see [`sim::tops::surface_rows`]), where
    /// its tops are known: worked out as the tops come, not every frame.
    surface: HashMap<(i64, i64), Option<(i64, i64)>>,
    to: Sender<ToWorker>,
    /// In a mutex only so the resource can be shared between threads; only one system uses it.
    from: Mutex<Receiver<FromWorker>>,
    /// Chunks asked for and not back yet.
    pending: HashSet<ChunkPos>,
    /// How many blocks and which settings the thread has, so changes are sent.
    blocks_sent: usize,
    settings_sent: Option<(AoTuning, bool)>,
}

impl Default for BackgroundMesher {
    fn default() -> Self {
        let (to, inbox) = channel::<ToWorker>();
        let (outbox, from) = channel::<FromWorker>();
        std::thread::Builder::new()
            .name("background mesher".into())
            .spawn(move || work(inbox, outbox))
            .expect("starting the background mesher");
        Self { chunks: HashMap::new(), columns: HashMap::new(), surface: HashMap::new(), to, from: Mutex::new(from), pending: HashSet::new(), blocks_sent: 0, settings_sent: None }
    }
}

impl BackgroundMesher {
    /// Whether the full-detail chunks that show the surface at chunk column `x, y` have all
    /// had their meshes back once, so the column can be drawn in place of far terrain: the
    /// chunks in the rows most of its tops are in (see [`sim::tops::surface_rows`]), or, where
    /// no tops are known (around what the mods built), every chunk the column holds.
    ///
    /// Far terrain's stand-in for picking by cube: a column whose surface chunks aren't all
    /// loaded (seen from high above, or from deep under it) is drawn as far terrain.
    pub fn surface_drawable(&self, x: i64, y: i64) -> bool {
        let shown = |z: i64| self.chunks.get(&ChunkPos(I64Vec3::new(x, y, z))).is_some_and(|c| c.shown);
        match self.surface.get(&(x, y)) {
            Some(rows) => rows.is_none_or(|(low, high)| (low..=high).all(shown)),
            None => self.columns.get(&(x, y)).is_some_and(|rows| rows.iter().all(|&z| shown(z))),
        }
    }

    /// Whether the streamed chunk at `pos` is one that shows its column's surface, rather than
    /// what's under or over it. Where no tops are known, every chunk is.
    pub fn shows_surface(&self, pos: ChunkPos) -> bool {
        match self.surface.get(&(pos.0.x, pos.0.y)) {
            Some(Some((low, high))) => (*low..=*high).contains(&pos.0.z),
            Some(None) | None => true,
        }
    }

    /// Works out the surface rows again for `columns`, whose tops came, moved or were dropped.
    fn note_tops(&mut self, tops: &sim::Tops, columns: impl Iterator<Item = (i64, i64)>) {
        for at in columns {
            match tops.column(at.0, at.1) {
                Some(column) => self.surface.insert(at, sim::tops::surface_rows(column)),
                None => self.surface.remove(&at),
            };
        }
    }

    /// Notes chunks whose meshes have come back for the first time.
    fn note_shown(&mut self) {
        let pending = &self.pending;
        for (pos, c) in self.chunks.iter_mut() {
            if !c.shown && matches!(c.state, ChunkState::Meshed { .. }) && !pending.contains(pos) {
                c.shown = true;
            }
        }
    }

    /// Whether the chunk at `pos` is a streamed chunk.
    pub fn is_streamed(&self, pos: ChunkPos) -> bool {
        self.chunks.contains_key(&pos)
    }

    /// Chunks asked for and not back yet, and chunks waiting for their neighbors.
    pub fn pending(&self) -> usize {
        self.pending.len() + self.chunks.values().filter(|c| matches!(c.state, ChunkState::Waiting(_))).count()
    }

    fn insert(&mut self, pos: ChunkPos, chunk: Streamed) {
        if self.chunks.insert(pos, chunk).is_none() {
            self.columns.entry((pos.0.x, pos.0.y)).or_default().push(pos.0.z);
        }
    }

    /// Notes chunks that arrived, and wakes the meshed chunks around them that were meshed
    /// without a neighbor.
    fn arrived(&mut self, chunks: &[ChunkPos], now: std::time::Instant) {
        for &pos in chunks {
            let shown = self.chunks.get(&pos).is_some_and(|c| c.shown);
            self.insert(pos, Streamed { state: ChunkState::Waiting(now), shown });
        }
        for &pos in chunks {
            for offset in around() {
                if let Some(c) = self.chunks.get_mut(&ChunkPos(pos.0 + offset))
                    && c.state == (ChunkState::Meshed { whole: false })
                {
                    c.state = ChunkState::Waiting(now);
                }
            }
        }
    }

    /// Forgets dropped chunks.
    fn gone(&mut self, chunks: &[ChunkPos]) {
        for &pos in chunks {
            if self.chunks.remove(&pos).is_some() {
                let key = (pos.0.x, pos.0.y);
                if let Some(rows) = self.columns.get_mut(&key) {
                    rows.retain(|&z| z != pos.0.z);
                    if rows.is_empty() {
                        self.columns.remove(&key);
                    }
                }
            }
        }
    }

    /// Chunks ready to mesh: every neighbor in, or waited long enough. Marks them meshed.
    fn ready(&mut self, now: std::time::Instant) -> Vec<ChunkPos> {
        let whole = |chunks: &HashMap<ChunkPos, Streamed>, pos: ChunkPos| around().all(|offset| chunks.contains_key(&ChunkPos(pos.0 + offset)));
        let due: Vec<(ChunkPos, bool)> = self
            .chunks
            .iter()
            .filter_map(|(&pos, c)| match c.state {
                ChunkState::Waiting(since) => {
                    let all = whole(&self.chunks, pos);
                    (all || now.duration_since(since).as_secs_f32() >= NEIGHBOR_WAIT).then_some((pos, all))
                }
                ChunkState::Meshed { .. } => None,
            })
            .collect();
        for &(pos, all) in &due {
            self.chunks.get_mut(&pos).expect("just found").state = ChunkState::Meshed { whole: all };
        }
        due.into_iter().map(|(pos, _)| pos).collect()
    }
}

/// The 26 chunks around one, as offsets.
fn around() -> impl Iterator<Item = I64Vec3> {
    (-1..=1).flat_map(|z| (-1..=1).flat_map(move |y| (-1..=1).map(move |x| I64Vec3::new(x, y, z)))).filter(|&o| o != I64Vec3::ZERO)
}

/// Threads the background mesher meshes on: half the cores, less one, as terrain is generated
/// on as many and the game and the renderer want theirs.
fn mesh_threads() -> usize {
    (std::thread::available_parallelism().map_or(2, |n| n.get()) / 2).saturating_sub(1).max(1)
}

/// The thread: keeps its copy of the world from what it's told, and meshes what it's asked
/// for, a batch at a time.
fn work(inbox: Receiver<ToWorker>, outbox: Sender<FromWorker>) {
    let mut grid = Grid::default();
    let mut blocks = BlockRegistry::default();
    let mut volumes: Option<Volumes> = None;
    let mut settings = (AoTuning::default(), true);
    while let Ok(first) = inbox.recv() {
        // Everything waiting is taken in before meshing, so a batch reads the newest world.
        let mut asked: HashMap<ChunkPos, u64> = HashMap::new();
        let mut far: Vec<crate::far::Asked> = Vec::new();
        for msg in std::iter::once(first).chain(inbox.try_iter()) {
            match msg {
                ToWorker::Restart { chunks, tops, blocks: b, looks } => {
                    grid = Grid::default();
                    grid.tops = tops;
                    for chunk in &chunks {
                        grid.import_chunk(chunk);
                    }
                    volumes = Some(Volumes::build(&looks, &b));
                    blocks = b;
                    asked.clear();
                    far.clear();
                }
                ToWorker::Chunks(chunks) => {
                    for chunk in &chunks {
                        grid.import_chunk(chunk);
                    }
                }
                ToWorker::Gone(gone) => {
                    for pos in gone {
                        grid.remove_chunk(pos);
                    }
                }
                ToWorker::Tops(news) => {
                    for tops in news {
                        for (x, y) in tops.dropped {
                            grid.tops.remove_column(x, y);
                        }
                        for ((x, y), column) in tops.set {
                            grid.tops.set_column(x, y, column);
                        }
                    }
                }
                ToWorker::Blocks(b) => {
                    blocks = b;
                    if let Some(v) = &mut volumes {
                        v.sync(&blocks);
                    }
                }
                ToWorker::Settings { ao, merge } => settings = (ao, merge),
                ToWorker::Mesh(list) => asked.extend(list),
                ToWorker::Far(list) => far.extend(list),
            }
        }
        let Some(volumes) = &volumes else { continue };
        if !asked.is_empty() {
            let list: Vec<(ChunkPos, u64)> = asked.into_iter().collect();
            let positions: Vec<ChunkPos> = list.iter().map(|(p, _)| *p).collect();
            let meshed = crate::mesher::mesh_chunks_on(&grid, &positions, &blocks, volumes, &settings.0, settings.1, mesh_threads());
            let out = list.into_iter().zip(meshed).map(|((pos, version), data)| Meshed { pos, version, data }).collect();
            if outbox.send(FromWorker::Near(out)).is_err() {
                return;
            }
        }
        if !far.is_empty() {
            let out = crate::far::mesh_columns(far, &blocks, volumes, settings.1, mesh_threads());
            if outbox.send(FromWorker::Far(out)).is_err() {
                return;
            }
        }
    }
}

/// Keeps the background mesher's world in step with the client's, asks it to mesh chunks that
/// arrived and their neighbors, drops the meshes of chunks that went, and puts in the meshes
/// that came back.
#[allow(clippy::too_many_arguments)]
pub(crate) fn stream_meshes(
    mut commands: Commands,
    mut world: ResMut<ClientWorld>,
    origin: Res<RenderOrigin>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut chunk_meshes: ResMut<ChunkMeshes>,
    mut handles: Query<&mut Mesh3d, With<ChunkMesh>>,
    ao: Res<AoTuning>,
    view: Res<crate::view::ViewSwitches>,
    mut bg: ResMut<BackgroundMesher>,
    mut heights: ResMut<crate::heights::Heights>,
    mut hidden: ResMut<crate::editing::HiddenGhosts>,
    mut far: ResMut<crate::far::FarTerrain>,
) {
    let mut news = world.take_chunk_news();
    let grid = world.universe().main();
    if news.restart {
        let chunks = grid.chunks().map(|(pos, _)| grid.export_chunk(pos)).collect();
        let _ = bg.to.send(ToWorker::Restart { chunks, tops: grid.tops.clone(), blocks: world.blocks().clone(), looks: world.looks().clone() });
        bg.blocks_sent = world.blocks().len();
        bg.pending.clear();
        // What came with the welcome was meshed with it (see `spawn_chunk_meshes`).
        bg.chunks.clear();
        bg.columns.clear();
        for (pos, _) in grid.chunks() {
            bg.insert(pos, Streamed { state: ChunkState::Meshed { whole: true }, shown: true });
        }
        heights.tops_changed(grid.tops.columns().map(|(at, _)| at));
        bg.surface.clear();
        bg.note_tops(&grid.tops, grid.tops.columns().map(|(at, _)| at));
        far.clear(&mut commands);
    } else {
        // Tops first: the chunks that came with them are lit and meshed by them.
        if !news.tops.is_empty() {
            let moved: Vec<(i64, i64)> = news.tops.iter().flat_map(|t| t.dropped.iter().copied().chain(t.set.iter().map(|(at, _)| *at))).collect();
            heights.tops_changed(moved.iter().copied());
            bg.note_tops(&grid.tops, moved.into_iter());
            let _ = bg.to.send(ToWorker::Tops(std::mem::take(&mut news.tops)));
        }
        if !news.data.is_empty() {
            let _ = bg.to.send(ToWorker::Chunks(std::mem::take(&mut news.data)));
        }
    }
    let asked = far.take_news(std::mem::take(&mut news.far), &mut commands);
    if !asked.is_empty() {
        let _ = bg.to.send(ToWorker::Far(asked));
    }
    if world.blocks().len() != bg.blocks_sent {
        bg.blocks_sent = world.blocks().len();
        let _ = bg.to.send(ToWorker::Blocks(world.blocks().clone()));
    }
    let settings = (*ao, view.merge_faces);
    if bg.settings_sent != Some(settings) {
        bg.settings_sent = Some(settings);
        let _ = bg.to.send(ToWorker::Settings { ao: settings.0, merge: settings.1 });
    }

    let now = std::time::Instant::now();
    if !news.gone.is_empty() {
        for &pos in &news.gone {
            chunk_meshes.forget(&mut commands, pos);
            bg.pending.remove(&pos);
        }
        bg.gone(&news.gone);
        heights.changed(&news.gone);
        let _ = bg.to.send(ToWorker::Gone(news.gone));
    }
    if !news.arrived.is_empty() {
        heights.changed(&news.arrived);
        bg.arrived(&news.arrived, now);
    }
    let ready = bg.ready(now);
    if !ready.is_empty() {
        let list: Vec<(ChunkPos, u64)> = ready.into_iter().filter(|p| grid.chunk(*p).is_some()).map(|p| (p, chunk_meshes.version(p))).collect();
        bg.pending.extend(list.iter().map(|(p, _)| *p));
        let _ = bg.to.send(ToWorker::Mesh(list));
    }

    let came: Vec<FromWorker> = match bg.from.get_mut() {
        Ok(from) => from.try_iter().collect(),
        Err(_) => Vec::new(),
    };
    let mut near: Vec<Vec<Meshed>> = Vec::new();
    for out in came {
        match out {
            FromWorker::Near(batch) => near.push(batch),
            FromWorker::Far(batch) => far.put(batch, &mut commands, &mut meshes, origin.cell),
        }
    }
    for batch in near {
        for Meshed { pos, version, data } in batch {
            bg.pending.remove(&pos);
            if chunk_meshes.version(pos) != version || grid.chunk(pos).is_none() {
                // Remeshed on the main thread since, or gone.
                continue;
            }
            hidden.dirty = true;
            hidden.changes += 1;
            chunk_meshes.put(&mut commands, &mut meshes, &mut handles, pos, data, origin.cell);
        }
    }
    bg.note_shown();
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn chunk(x: i64, y: i64, z: i64) -> ChunkPos {
        ChunkPos(I64Vec3::new(x, y, z))
    }

    /// Marks everything meshed so far as having its mesh back.
    fn show(bg: &mut BackgroundMesher) {
        bg.pending.clear();
        bg.note_shown();
    }

    #[test]
    fn a_chunk_waits_for_the_26_around_it_or_a_while() {
        let mut bg = BackgroundMesher::default();
        let start = Instant::now();
        let middle = chunk(5, 5, 5);
        bg.arrived(&[middle], start);
        assert!(bg.ready(start).is_empty(), "alone, it waits");
        // All but the one under a corner: still waiting.
        let mut around: Vec<ChunkPos> = around().map(|o| ChunkPos(middle.0 + o)).collect();
        assert_eq!(around.len(), 26);
        let last = around.pop().unwrap();
        bg.arrived(&around, start);
        assert!(!bg.ready(start).contains(&middle));
        bg.arrived(&[last], start);
        assert_eq!(bg.ready(start), [middle], "its neighbors are in; theirs aren't");
        assert_eq!(bg.pending(), 26, "the rest still wait");
        // The rest are meshed without their neighbors once they've waited long enough.
        let later = start + Duration::from_secs_f32(NEIGHBOR_WAIT + 0.1);
        assert_eq!(bg.ready(later).len(), 26);
        assert_eq!(bg.pending(), 0);
        // A chunk arriving beside ones meshed without it has them meshed again; the middle
        // one, meshed whole, isn't.
        let beyond = chunk(7, 5, 5);
        bg.arrived(&[beyond], later);
        let much_later = later + Duration::from_secs_f32(NEIGHBOR_WAIT + 0.1);
        let again = bg.ready(much_later);
        assert!(again.contains(&beyond) && again.contains(&chunk(6, 5, 5)) && again.contains(&chunk(6, 6, 4)));
        assert!(!again.contains(&middle) && !again.contains(&chunk(4, 5, 5)));
        assert_eq!(again.len(), 10, "the new one and the nine it touches");
    }

    #[test]
    fn dropped_chunks_are_forgotten() {
        let mut bg = BackgroundMesher::default();
        let now = Instant::now();
        bg.arrived(&[chunk(0, 0, 0), chunk(0, 0, 1)], now);
        assert!(bg.is_streamed(chunk(0, 0, 1)));
        bg.gone(&[chunk(0, 0, 1)]);
        assert!(!bg.is_streamed(chunk(0, 0, 1)) && bg.is_streamed(chunk(0, 0, 0)));
        assert_eq!(bg.columns[&(0, 0)], [0]);
        bg.gone(&[chunk(0, 0, 0)]);
        assert!(bg.columns.is_empty() && bg.chunks.is_empty());
    }

    #[test]
    fn a_column_is_drawable_once_the_chunks_showing_its_surface_are() {
        let mut bg = BackgroundMesher::default();
        let now = Instant::now();
        let later = now + Duration::from_secs_f32(NEIGHBOR_WAIT + 0.1);
        // Ground between 40 and 70 boxels up: chunk rows 1 and 2. A shaft and a tower don't
        // count.
        let mut tops = [40; api::TOPS_LEN];
        for top in tops.iter_mut().skip(500) {
            *top = 70;
        }
        tops[0] = -4000;
        tops[1] = 4000;
        let mut known = sim::Tops::default();
        known.set_column(3, 3, std::sync::Arc::new(tops));
        bg.note_tops(&known, [(3, 3)].into_iter());
        assert!(!bg.surface_drawable(3, 3), "nothing loaded");
        // Deep under it, as when the player is far underground: not the surface.
        bg.arrived(&[chunk(3, 3, -8), chunk(3, 3, -9)], now);
        bg.ready(later);
        show(&mut bg);
        assert!(!bg.surface_drawable(3, 3));
        assert!(!bg.shows_surface(chunk(3, 3, -8)), "always drawn: it's under the surface");
        // One of the two surface chunks: not yet.
        bg.arrived(&[chunk(3, 3, 1)], now);
        bg.ready(later);
        show(&mut bg);
        assert!(!bg.surface_drawable(3, 3));
        // Both, but the second's mesh isn't back.
        bg.arrived(&[chunk(3, 3, 2)], now);
        bg.ready(later);
        bg.pending.insert(chunk(3, 3, 2));
        bg.note_shown();
        assert!(!bg.surface_drawable(3, 3));
        show(&mut bg);
        assert!(bg.surface_drawable(3, 3));
        assert!(bg.shows_surface(chunk(3, 3, 1)) && bg.shows_surface(chunk(3, 3, 2)) && !bg.shows_surface(chunk(3, 3, 3)));
        // Meshed again later (a neighbor came), its old mesh still shows.
        bg.arrived(&[chunk(3, 3, 2)], later);
        assert!(bg.surface_drawable(3, 3));
        // Flying up, the lower surface chunk is dropped: far terrain stands in again.
        bg.gone(&[chunk(3, 3, 1)]);
        assert!(!bg.surface_drawable(3, 3));
        // The tops dropped with the column: nothing known, nothing drawable.
        bg.note_tops(&sim::Tops::default(), [(3, 3)].into_iter());
        bg.gone(&[chunk(3, 3, 2), chunk(3, 3, -8), chunk(3, 3, -9)]);
        assert!(!bg.surface_drawable(3, 3));
    }

    #[test]
    fn where_no_tops_are_known_a_column_is_drawable_once_all_its_chunks_are() {
        // Around what the mods built: no terrain, no tops.
        let mut bg = BackgroundMesher::default();
        let now = Instant::now();
        bg.insert(chunk(0, 0, 0), Streamed { state: ChunkState::Meshed { whole: true }, shown: true });
        assert!(bg.surface_drawable(0, 0) && bg.shows_surface(chunk(0, 0, 0)));
        bg.arrived(&[chunk(0, 0, 1)], now);
        assert!(!bg.surface_drawable(0, 0));
        bg.ready(now + Duration::from_secs_f32(NEIGHBOR_WAIT + 0.1));
        show(&mut bg);
        assert!(bg.surface_drawable(0, 0));
        assert!(!bg.surface_drawable(9, 9), "a column with nothing in it");
        // A column of terrain where nothing at any height stops the sky has no surface to wait for.
        let mut known = sim::Tops::default();
        known.set_column(9, 9, std::sync::Arc::new([api::NO_TOP; api::TOPS_LEN]));
        bg.note_tops(&known, [(9, 9)].into_iter());
        assert!(bg.surface_drawable(9, 9));
    }
}

#[cfg(test)]
mod stream_timing {
    use std::time::Instant;

    use api::{I64Vec3, Vec3, WorldPos};
    use protocol::{ClientMsg, Command, ServerMsg};
    use server::Server;
    use sim::PlayerInput;

    use super::*;
    use crate::world::testing::base_sim;

    /// Moves the player across the terrain at `speed` m/s for `seconds`, timing each part of
    /// a tick: the server's, the client taking the messages in, and meshing what arrived with
    /// its neighbors. Run with `--release --ignored --nocapture`.
    fn time_streaming(speed: f32, seconds: f32) {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        let start = WorldPos::at_boxel(base::TERRAIN_CENTER.offset(I64Vec3::new(0, 0, 120)));
        sim.teleport_player(start);
        sim.generate_terrain().unwrap();
        let mut server = Server::new(sim);
        let mut world = ClientWorld::new(server.welcome());
        let reach = sim::far::Reach::default();
        server.handle(ClientMsg::Command(Command::ViewDistance(reach)));
        let fly = PlayerInput { toggle_fly: true, ..Default::default() };
        for m in server.handle(ClientMsg::Input(Box::new(fly))) {
            world.apply(m);
        }
        world.take_chunk_news();
        let volumes = Volumes::build(world.looks(), world.blocks());
        let ao = AoTuning::default();
        let ticks = (seconds * 60.0) as usize;
        let mut columns = BackgroundMesher::default();
        let epoch = Instant::now();
        let (mut tick_s, mut apply_s, mut mesh_s, mut worst_tick, mut worst_apply) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let (mut arrived, mut meshed, mut changed, mut nonempty) = (0, 0, 0, 0);
        let (mut far_s, mut far_columns, mut far_quads) = (0.0f32, 0, 0);
        for t in 0..ticks {
            let x = speed * t as f32 / 60.0;
            let at = WorldPos::new(start.cell() + I64Vec3::new(x as i64, 0, 0), Vec3::new(x.fract(), 0.5, 0.5));
            server.handle(ClientMsg::Command(Command::TeleportPlayer(at)));
            let clock = Instant::now();
            let msgs = server.handle(ClientMsg::Input(Box::default()));
            let s = clock.elapsed().as_secs_f32();
            tick_s += s;
            worst_tick = worst_tick.max(s);
            let clock = Instant::now();
            for m in msgs {
                if let ServerMsg::Chunk(_) = &m {
                    changed += 1;
                }
                world.apply(m);
            }
            let s = clock.elapsed().as_secs_f32();
            apply_s += s;
            worst_apply = worst_apply.max(s);
            let mut news = world.take_chunk_news();
            world.take_changed_chunks();
            let far: Vec<crate::far::Asked> = std::mem::take(&mut news.far)
                .into_iter()
                .filter_map(|n| match n {
                    crate::world::FarNews::Built(b) => Some((0, b)),
                    crate::world::FarNews::Gone(_) => None,
                })
                .collect();
            far_columns += far.len();
            let clock = Instant::now();
            let far_meshed = crate::far::mesh_columns(far, world.blocks(), &volumes, true, mesh_threads());
            far_s += clock.elapsed().as_secs_f32();
            far_quads += far_meshed.iter().flat_map(|f| &f.meshes).map(|(_, m)| m.quad_count()).sum::<usize>();
            let grid = world.universe().main();
            // Time as the game would see it, a tick at a time.
            let now = epoch + std::time::Duration::from_secs_f32(t as f32 / 60.0);
            columns.gone(&news.gone);
            columns.arrived(&news.arrived, now);
            let list: Vec<ChunkPos> = columns.ready(now).into_iter().filter(|p| grid.chunk(*p).is_some()).collect();
            arrived += news.arrived.len();
            meshed += list.len();
            let clock = Instant::now();
            let out = crate::mesher::mesh_chunks_on(grid, &list, world.blocks(), &volumes, &ao, true, mesh_threads());
            mesh_s += clock.elapsed().as_secs_f32();
            nonempty += out.iter().filter(|m| !m.is_empty()).count();
        }
        let n = ticks as f32;
        let t = world.terrain();
        println!(
            "{speed} m/s for {seconds} s: server tick avg {:.2} ms, worst {:.1} ms | client apply avg {:.2} ms, worst {:.1} ms | \
             meshing {:.2} s wall on {} threads for {seconds} s ({meshed} meshes for {arrived} chunks arrived, {nonempty} with faces) | \
             {changed} changed chunks sent | {} chunks loaded, {} waiting, workers {:.1} s, joining {:.2} s | \
             far: {far_columns} columns arrived, meshed in {far_s:.2} s ({far_quads} quads), {} loaded, {} waiting, built in {:.1} s",
            tick_s / n * 1000.0,
            worst_tick * 1000.0,
            apply_s / n * 1000.0,
            worst_apply * 1000.0,
            mesh_s,
            mesh_threads(),
            t.chunks,
            t.waiting,
            t.seconds,
            t.join_seconds,
            world.far().columns,
            world.far().waiting,
            world.far().seconds,
        );
    }

    #[test]
    #[ignore]
    fn time_streaming_while_walking() {
        time_streaming(4.3, 20.0);
    }

    #[test]
    #[ignore]
    fn time_streaming_while_flying() {
        time_streaming(30.0, 20.0);
    }
}
