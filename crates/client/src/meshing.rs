//! Meshing in the background: chunks that stream in are meshed on threads of their own, from
//! a copy of the world kept from the same messages, so flying into new terrain never stalls a
//! frame. Terrain comes in whole columns, and a column waits to be meshed until the eight
//! around it are in (their faces and corners read it), or [`NEIGHBOR_WAIT`] has passed, as at
//! the edge of what's loaded; one meshed without all its neighbors is meshed again when one
//! comes. So each chunk is meshed about once. Edits still remesh on the main thread right away (see
//! [`crate::editing::remesh_changed_chunks`]); a background mesh of a chunk remeshed there since
//! it was asked for is thrown away, by the chunk's version in [`ChunkMeshes`].

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use api::ChunkPos;
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
    Restart { chunks: Vec<ChunkData>, blocks: BlockRegistry, looks: LookRegistry },
    /// Chunks that arrived or changed.
    Chunks(Vec<ChunkData>),
    /// Chunks dropped.
    Gone(Vec<ChunkPos>),
    /// The block registry grew, by crafting.
    Blocks(BlockRegistry),
    Settings { ao: AoTuning, merge: bool },
    /// Chunks to mesh, each with its version when asked.
    Mesh(Vec<(ChunkPos, u64)>),
}

/// A chunk meshed in the background.
struct Meshed {
    pos: ChunkPos,
    version: u64,
    data: MeshData,
}

/// Seconds a column waits for its neighbors before it's meshed without them.
pub const NEIGHBOR_WAIT: f32 = 1.0;

/// Where a streamed column of chunks is in being meshed.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ColumnState {
    /// Waiting for its neighbors since then.
    Waiting(std::time::Instant),
    /// Meshed, with all its neighbors there or not.
    Meshed { whole: bool },
}

/// A streamed column of chunks: its chunks, and where it is in being meshed.
struct Column {
    chunks: Vec<ChunkPos>,
    state: ColumnState,
}

/// The background mesher's thread, and what's been asked of it.
#[derive(Resource)]
pub(crate) struct BackgroundMesher {
    /// Streamed columns, by chunk X and Y.
    columns: HashMap<(i64, i64), Column>,
    to: Sender<ToWorker>,
    /// In a mutex only so the resource can be shared between threads; only one system uses it.
    from: Mutex<Receiver<Vec<Meshed>>>,
    /// Chunks asked for and not back yet.
    pending: HashSet<ChunkPos>,
    /// How many blocks and which settings the thread has, so changes are sent.
    blocks_sent: usize,
    settings_sent: Option<(AoTuning, bool)>,
}

impl Default for BackgroundMesher {
    fn default() -> Self {
        let (to, inbox) = channel::<ToWorker>();
        let (outbox, from) = channel::<Vec<Meshed>>();
        std::thread::Builder::new()
            .name("background mesher".into())
            .spawn(move || work(inbox, outbox))
            .expect("starting the background mesher");
        Self { columns: HashMap::new(), to, from: Mutex::new(from), pending: HashSet::new(), blocks_sent: 0, settings_sent: None }
    }
}

impl BackgroundMesher {
    /// Chunks asked for and not back yet, and chunks of columns waiting for their neighbors.
    pub fn pending(&self) -> usize {
        let waiting: usize =
            self.columns.values().filter(|c| matches!(c.state, ColumnState::Waiting(_))).map(|c| c.chunks.len()).sum();
        self.pending.len() + waiting
    }

    /// Notes chunks that arrived, by column, and wakes the meshed columns beside them that were
    /// meshed without a neighbor.
    fn arrived(&mut self, chunks: &[ChunkPos], now: std::time::Instant) {
        for &pos in chunks {
            let column = self.columns.entry((pos.0.x, pos.0.y)).or_insert(Column { chunks: Vec::new(), state: ColumnState::Waiting(now) });
            if !column.chunks.contains(&pos) {
                column.chunks.push(pos);
            }
            column.state = ColumnState::Waiting(now);
        }
        let landed: HashSet<(i64, i64)> = chunks.iter().map(|p| (p.0.x, p.0.y)).collect();
        for (x, y) in landed {
            for (dx, dy) in AROUND {
                if let Some(c) = self.columns.get_mut(&(x + dx, y + dy))
                    && c.state == (ColumnState::Meshed { whole: false })
                {
                    c.state = ColumnState::Waiting(now);
                }
            }
        }
    }

    /// Forgets dropped chunks, and columns left with none.
    fn gone(&mut self, chunks: &[ChunkPos]) {
        for &pos in chunks {
            let key = (pos.0.x, pos.0.y);
            if let Some(c) = self.columns.get_mut(&key) {
                c.chunks.retain(|&p| p != pos);
                if c.chunks.is_empty() {
                    self.columns.remove(&key);
                }
            }
        }
    }

    /// Columns ready to mesh: every neighbor in, or waited long enough. Marks them meshed and
    /// gives their chunks.
    fn ready(&mut self, now: std::time::Instant) -> Vec<ChunkPos> {
        let whole = |columns: &HashMap<(i64, i64), Column>, (x, y): (i64, i64)| {
            AROUND.iter().all(|(dx, dy)| columns.contains_key(&(x + dx, y + dy)))
        };
        let due: Vec<((i64, i64), bool)> = self
            .columns
            .iter()
            .filter_map(|(&key, c)| match c.state {
                ColumnState::Waiting(since) => {
                    let all = whole(&self.columns, key);
                    (all || now.duration_since(since).as_secs_f32() >= NEIGHBOR_WAIT).then_some((key, all))
                }
                ColumnState::Meshed { .. } => None,
            })
            .collect();
        let mut out = Vec::new();
        for (key, all) in due {
            let c = self.columns.get_mut(&key).expect("just found");
            c.state = ColumnState::Meshed { whole: all };
            out.extend_from_slice(&c.chunks);
        }
        out
    }
}

/// The eight columns around one, as chunk offsets.
const AROUND: [(i64, i64); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/// Threads the background mesher meshes on: half the cores, less one, as terrain is generated
/// on as many and the game and the renderer want theirs.
fn mesh_threads() -> usize {
    (std::thread::available_parallelism().map_or(2, |n| n.get()) / 2).saturating_sub(1).max(1)
}

/// The thread: keeps its copy of the world from what it's told, and meshes what it's asked
/// for, a batch at a time.
fn work(inbox: Receiver<ToWorker>, outbox: Sender<Vec<Meshed>>) {
    let mut grid = Grid::default();
    let mut blocks = BlockRegistry::default();
    let mut volumes: Option<Volumes> = None;
    let mut settings = (AoTuning::default(), true);
    while let Ok(first) = inbox.recv() {
        // Everything waiting is taken in before meshing, so a batch reads the newest world.
        let mut asked: HashMap<ChunkPos, u64> = HashMap::new();
        for msg in std::iter::once(first).chain(inbox.try_iter()) {
            match msg {
                ToWorker::Restart { chunks, blocks: b, looks } => {
                    grid = Grid::default();
                    for chunk in &chunks {
                        grid.import_chunk(chunk);
                    }
                    volumes = Some(Volumes::build(&looks, &b));
                    blocks = b;
                    asked.clear();
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
                ToWorker::Blocks(b) => {
                    blocks = b;
                    if let Some(v) = &mut volumes {
                        v.sync(&blocks);
                    }
                }
                ToWorker::Settings { ao, merge } => settings = (ao, merge),
                ToWorker::Mesh(list) => asked.extend(list),
            }
        }
        let Some(volumes) = &volumes else { continue };
        if asked.is_empty() {
            continue;
        }
        let list: Vec<(ChunkPos, u64)> = asked.into_iter().collect();
        let positions: Vec<ChunkPos> = list.iter().map(|(p, _)| *p).collect();
        let meshed = crate::mesher::mesh_chunks_on(&grid, &positions, &blocks, volumes, &settings.0, settings.1, mesh_threads());
        let out = list.into_iter().zip(meshed).map(|((pos, version), data)| Meshed { pos, version, data }).collect();
        if outbox.send(out).is_err() {
            return;
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
) {
    let news = world.take_chunk_news();
    let grid = world.universe().main();
    if news.restart {
        let chunks = grid.chunks().map(|(pos, _)| grid.export_chunk(pos)).collect();
        let _ = bg.to.send(ToWorker::Restart { chunks, blocks: world.blocks().clone(), looks: world.looks().clone() });
        bg.blocks_sent = world.blocks().len();
        bg.pending.clear();
    } else if !news.data.is_empty() {
        let _ = bg.to.send(ToWorker::Chunks(news.data));
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

    let came: Vec<Vec<Meshed>> = match bg.from.get_mut() {
        Ok(from) => from.try_iter().collect(),
        Err(_) => Vec::new(),
    };
    for batch in came {
        for Meshed { pos, version, data } in batch {
            bg.pending.remove(&pos);
            if chunk_meshes.version(pos) != version || grid.chunk(pos).is_none() {
                // Remeshed on the main thread since, or gone.
                continue;
            }
            hidden.dirty = true;
            chunk_meshes.put(&mut commands, &mut meshes, &mut handles, pos, data, origin.cell);
        }
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
            let news = world.take_chunk_news();
            world.take_changed_chunks();
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
             {changed} changed chunks sent | {} columns loaded, {} waiting, workers {:.1} s, joining {:.2} s",
            tick_s / n * 1000.0,
            worst_tick * 1000.0,
            apply_s / n * 1000.0,
            worst_apply * 1000.0,
            mesh_s,
            mesh_threads(),
            t.columns,
            t.waiting,
            t.seconds,
            t.join_seconds,
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
