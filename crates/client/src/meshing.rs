//! Meshing in the background: chunks that stream in are meshed on a thread of their own, from
//! its own copy of the world kept from the same messages, so flying into new terrain never
//! stalls a frame. A chunk that arrives remeshes its loaded neighbors too, as their border faces
//! and corners read it. Edits still remesh on the main thread right away (see
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

/// The background mesher's thread, and what's been asked of it.
#[derive(Resource)]
pub(crate) struct BackgroundMesher {
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
        Self { to, from: Mutex::new(from), pending: HashSet::new(), blocks_sent: 0, settings_sent: None }
    }
}

impl BackgroundMesher {
    /// Chunks asked for and not back yet.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
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
        let meshed = crate::mesher::mesh_chunks(&grid, &positions, &blocks, volumes, &settings.0, settings.1);
        let out = list.into_iter().zip(meshed).map(|((pos, version), data)| Meshed { pos, version, data }).collect();
        if outbox.send(out).is_err() {
            return;
        }
    }
}

/// The chunk at `pos` and every loaded chunk around it, corners included: what reads the
/// chunk when meshed.
fn with_neighbors(grid: &Grid, pos: ChunkPos) -> impl Iterator<Item = ChunkPos> + '_ {
    (-1..=1).flat_map(move |z| {
        (-1..=1).flat_map(move |y| (-1..=1).map(move |x| ChunkPos(pos.0 + I64Vec3::new(x, y, z))))
    })
    .filter(|p| grid.chunk(*p).is_some())
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

    if !news.gone.is_empty() {
        for &pos in &news.gone {
            chunk_meshes.forget(&mut commands, pos);
            bg.pending.remove(&pos);
        }
        heights.changed(&news.gone);
        let _ = bg.to.send(ToWorker::Gone(news.gone));
    }
    if !news.arrived.is_empty() {
        heights.changed(&news.arrived);
        let mut asked: HashSet<ChunkPos> = HashSet::new();
        for &pos in &news.arrived {
            asked.extend(with_neighbors(grid, pos));
        }
        let list: Vec<(ChunkPos, u64)> = asked.into_iter().map(|p| (p, chunk_meshes.version(p))).collect();
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
