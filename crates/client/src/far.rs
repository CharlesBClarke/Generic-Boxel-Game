//! Far terrain on the client: the far chunks the server sends (see [`sim::far`]), meshed in
//! the background with the far chunks of their level beside them (so faces between two are
//! left out, and a far chunk is meshed again when one beside it comes), scaled to their level
//! and drawn with the far material (each face as the slope of the ground around it, see
//! [`crate::slope`]: the average colors and light of the full-detail faces such a slope is
//! made of, mixed with its cell's second material, no AO or edges, its shadow looked up a cell
//! higher); and which level is drawn where, picked by the octree from the camera as far
//! chunks become ready, which also says which ordinary chunks are drawn (see
//! [`crate::view::apply_render_distance`]). Ordinary chunks that are in are drawn even under a
//! far chunk still waiting for its finer levels; the far material is told where they are (the
//! mask, see [`mask_of`]) and leaves itself out there, so full detail is never hidden.
//!
//! The faces a far chunk leaves out toward a solid far chunk beside it are kept apart, as that
//! side's wall, and drawn only while the one beside it isn't drawn at its level (finer or
//! coarser terrain is there instead, whose ground can sit lower and would leave a hole).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use api::{BoxelPos, ChunkPos, I64Vec3, WorldPos};
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sim::far::{FarBuilt, FarCells, FarChunk, Mix, Reach};
use sim::{BlockRegistry, Grid};

use crate::editing::ChunkMeshes;
use crate::meshing::BackgroundMesher;
use crate::mesher::{AoTuning, MeshData};
use crate::solid::SolidMaterial;
use crate::view::{ViewSwitches, ViewTuning};
use crate::volumes::Volumes;
use crate::world::{ClientWorld, FarNews};
use crate::{PlayerCamera, RenderOrigin, coords};

/// `SolidMaterial::place` for far meshes: the shader's sign to draw them plainly.
pub const FAR_PLACE: UVec4 = UVec4::new(0, 0, 0, 2);

/// A far chunk to mesh: the version it's meshed for, its cells, and those of the far chunks
/// of its level beside it that the client has.
pub(crate) struct Asked {
    pub version: u64,
    pub chunk: FarChunk,
    pub cells: Arc<FarCells>,
    pub beside: Vec<Arc<FarCells>>,
}

/// A far chunk meshed in the background; `None` if it has no faces.
pub(crate) struct FarMeshed {
    pub chunk: FarChunk,
    pub version: u64,
    pub mesh: Option<MeshData>,
    /// Its wall on each side, in [`FarChunk::beside`]'s order: the faces left out of `mesh`
    /// as the far chunk beside it there is solid across them. `None` where there are none.
    pub walls: [Option<MeshData>; 6],
}

/// One far chunk's mesh, or its wall on one side (in [`FarChunk::beside`]'s order).
#[derive(Component)]
pub(crate) struct FarMesh {
    chunk: FarChunk,
    wall: Option<u8>,
}

/// A far chunk the server sent.
struct Entry {
    /// Goes up each time it's sent or meshed again, so an older mesh coming back is thrown
    /// away.
    version: u64,
    /// Its mesh is in (or it needs none), so it can be drawn.
    meshed: bool,
    /// Its cells; `None` if nothing in it is solid.
    cells: Option<Arc<FarCells>>,
    entity: Option<Entity>,
    quads: usize,
    /// Its walls (see [`FarMeshed::walls`]) and how many quads each has.
    walls: [Option<Entity>; 6],
    wall_quads: [usize; 6],
}

impl Entry {
    fn new(version: u64, meshed: bool) -> Self {
        Entry { version, meshed, cells: None, entity: None, quads: 0, walls: [None; 6], wall_quads: [0; 6] }
    }

    /// Takes its meshes out of the world.
    fn despawn(&mut self, commands: &mut Commands) {
        for e in self.entity.take().into_iter().chain(self.walls.iter_mut().filter_map(Option::take)) {
            commands.entity(e).despawn();
        }
        self.quads = 0;
        self.wall_quads = [0; 6];
    }
}

/// Whether a far chunk's mesh is shown, or its wall on side `wall`, given the far chunks
/// `drawn`: the mesh if it's drawn; a wall only while the far chunk beside it there isn't.
fn shown(drawn: &HashSet<FarChunk>, chunk: FarChunk, wall: Option<u8>) -> bool {
    drawn.contains(&chunk) && wall.is_none_or(|side| !drawn.contains(&chunk.beside()[side as usize]))
}

/// What far terrain draws, for the readout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FarViewStats {
    /// Far chunks drawn (with faces or not), and ordinary ones.
    pub far: usize,
    pub ordinary: usize,
    /// Far chunks with faces drawn.
    pub chunks: usize,
    /// Triangles in the far chunks drawn.
    pub triangles: usize,
}

/// The far chunks the client has, and which far and ordinary chunks are drawn.
#[derive(Resource, Default)]
pub(crate) struct FarTerrain {
    chunks: HashMap<FarChunk, Entry>,
    next_version: u64,
    material: Option<Handle<SolidMaterial>>,
    /// Whether far terrain is on: the view reaches past full detail.
    pub active: bool,
    /// The far chunks drawn.
    drawn: HashSet<FarChunk>,
    /// The ordinary chunks drawn, where far terrain is on.
    pub ordinary: HashSet<ChunkPos>,
    /// The mask the far material has, and the render origin it was made around.
    mask: Option<(Vec<u8>, I64Vec3)>,
    pub stats: FarViewStats,
}

impl FarTerrain {
    /// Forgets every far chunk, as when the world comes anew.
    pub fn clear(&mut self, commands: &mut Commands) {
        for (_, mut entry) in self.chunks.drain() {
            entry.despawn(commands);
        }
        self.drawn.clear();
    }

    /// Far chunks the server sent that have no mesh yet: still with the background mesher.
    pub fn unmeshed(&self) -> usize {
        self.chunks.values().filter(|e| !e.meshed).count()
    }

    /// Takes in far chunks built and dropped, and returns the ones to mesh: those built with
    /// something in them, and the ones beside a far chunk that came or changed, whose faces
    /// toward it may come or go.
    pub fn take_news(&mut self, news: Vec<FarNews>, commands: &mut Commands) -> Vec<Asked> {
        let mut again: HashSet<FarChunk> = HashSet::new();
        for n in news {
            match n {
                FarNews::Gone(chunk) => {
                    again.remove(&chunk);
                    if let Some(mut entry) = self.chunks.remove(&chunk) {
                        entry.despawn(commands);
                    }
                }
                FarNews::Built(FarBuilt { chunk, cells }) => {
                    self.next_version += 1;
                    let version = self.next_version;
                    // One built again keeps drawing the old mesh until the new one is in.
                    let entry = self.chunks.entry(chunk).or_insert(Entry::new(version, false));
                    let had = entry.cells.is_some();
                    entry.version = version;
                    entry.cells = cells.map(Arc::new);
                    if entry.cells.is_some() {
                        again.insert(chunk);
                    } else {
                        again.remove(&chunk);
                        entry.despawn(commands);
                        entry.meshed = true;
                    }
                    // The ones beside it whose faces toward it go: those solid where it's solid
                    // across the face between them. Built again (after an edit), all of them.
                    let came = entry.cells.clone();
                    for (side, b) in chunk.beside().into_iter().enumerate() {
                        let Some(theirs) = self.chunks.get(&b).and_then(|e| e.cells.as_ref()) else { continue };
                        if had || came.as_ref().is_some_and(|c| touching(&c.data, &theirs.data, side)) {
                            again.insert(b);
                        }
                    }
                }
            }
        }
        let mut asked = Vec::with_capacity(again.len());
        for chunk in again {
            let beside: Vec<Arc<FarCells>> = chunk.beside().iter().filter_map(|b| self.chunks.get(b).and_then(|e| e.cells.clone())).collect();
            self.next_version += 1;
            let version = self.next_version;
            let Some(entry) = self.chunks.get_mut(&chunk) else { continue };
            let Some(cells) = entry.cells.clone() else { continue };
            entry.version = version;
            asked.push(Asked { version, chunk, cells, beside });
        }
        asked
    }

    /// Puts in meshes that came back, replacing a far chunk's old one.
    pub fn put(&mut self, batch: Vec<FarMeshed>, commands: &mut Commands, meshes: &mut Assets<Mesh>, origin: I64Vec3) {
        let Some(material) = self.material.clone() else { return };
        for FarMeshed { chunk, version, mesh, walls } in batch {
            let Some(entry) = self.chunks.get_mut(&chunk) else { continue };
            if entry.version != version {
                continue;
            }
            entry.despawn(commands);
            let drawn = &self.drawn;
            let mut spawn = |data: MeshData, wall: Option<u8>| {
                let visibility = if shown(drawn, chunk, wall) { Visibility::Inherited } else { Visibility::Hidden };
                commands
                    .spawn((
                        Mesh3d(meshes.add(crate::to_bevy_mesh(data))),
                        MeshMaterial3d(material.clone()),
                        far_transform(chunk, origin),
                        FarMesh { chunk, wall },
                        NotShadowCaster,
                        visibility,
                    ))
                    .id()
            };
            if let Some(data) = mesh {
                entry.quads = data.quad_count();
                entry.entity = Some(spawn(data, None));
            }
            for (side, wall) in walls.into_iter().enumerate() {
                if let Some(data) = wall {
                    entry.wall_quads[side] = data.quad_count();
                    entry.walls[side] = Some(spawn(data, Some(side as u8)));
                }
            }
            entry.meshed = true;
        }
    }
}

/// Whether far chunk `a` and the one beside it on `side` (in [`FarChunk::beside`]'s order)
/// are both solid somewhere across the face between them.
fn touching(a: &sim::ChunkData, b: &sim::ChunkData, side: usize) -> bool {
    match (a.cells.uniform(), b.cells.uniform()) {
        (Some(0), _) | (_, Some(0)) => return false,
        (Some(_), Some(_)) => return true,
        _ => {}
    }
    let (axis, forward) = (side / 2, side.is_multiple_of(2));
    let last = (api::CHUNK_SIZE - 1) as u8;
    let (mine, theirs) = if forward { (last, 0) } else { (0, last) };
    let at = |k: u8, u: u8, v: u8| match axis {
        0 => api::LocalPos::new(k, u, v),
        1 => api::LocalPos::new(u, k, v),
        _ => api::LocalPos::new(u, v, k),
    };
    (0..=last).any(|u| (0..=last).any(|v| a.cells.get(at(mine, u, v)) != 0 && b.cells.get(at(theirs, u, v)) != 0))
}

/// Where a far chunk is drawn: its corner, and scaled up to its level.
fn far_transform(chunk: FarChunk, origin: I64Vec3) -> Transform {
    let corner = WorldPos::at_boxel(BoxelPos(chunk.pos * chunk.span()));
    Transform::from_translation(coords::render_translation(corner, origin)).with_scale(Vec3::splat(chunk.step() as f32))
}

/// Meshes far chunks, each with the far chunks beside it it was asked with (faces toward
/// them left out where they're solid and kept apart as that side's wall, see
/// [`FarMeshed::walls`]; faces toward a missing one kept in the mesh: both plug the cracks
/// between levels), every face given the slope of the ground around it (see
/// [`crate::slope`]) and its cell's mix of materials (see [`set_mixes`]), spread over
/// `threads` threads.
pub(crate) fn mesh_far(list: Vec<Asked>, blocks: &BlockRegistry, volumes: &Volumes, merge: bool, threads: usize) -> Vec<FarMeshed> {
    let ao = AoTuning::default();
    let one = |Asked { version, chunk, cells, beside }: Asked| {
        let mut grid = Grid::default();
        grid.import_chunk(&cells.data);
        for b in &beside {
            grid.import_chunk(&b.data);
        }
        let pos = cells.data.pos;
        let slopes = crate::slope::Cells::of(&grid, pos);
        // The faces toward the far chunks beside it are left out where those are solid. Near
        // the surface (see `wall_cells`) they're wanted all the same, apart, as that side's
        // wall: there the cells across the border are emptied before meshing, so the faces
        // come, and are then taken out of the mesh.
        let near = wall_cells(&cells.data, &beside, &slopes);
        let sides = chunk.beside();
        for (side, cells) in near.iter().enumerate().filter(|(_, cells)| !cells.is_empty()) {
            let Some(theirs) = beside.iter().find(|b| b.data.pos.0 == sides[side].pos) else { continue };
            let mut theirs = theirs.data.clone();
            for &i in cells {
                let at = api::LocalPos::from_index(i);
                let mut p = [at.x, at.y, at.z];
                p[side / 2] = if side.is_multiple_of(2) { 0 } else { (api::CHUNK_SIZE - 1) as u8 };
                theirs.cells.set_at(api::LocalPos::new(p[0], p[1], p[2]).index(), 0);
            }
            grid.import_chunk(&theirs);
        }
        let all = crate::mesher::mesh_chunks_on(&grid, &[pos], blocks, volumes, &ao, false, 1).pop().unwrap_or_default();
        let mut mesh = MeshData::default();
        let mut walls: [MeshData; 6] = Default::default();
        for q in 0..all.quad_count() {
            match border_side(&all, q).filter(|&side| near[side].contains(&cell_behind(&all, q))) {
                Some(side) => crate::greedy::copy(&all, q, &mut walls[side]),
                None => crate::greedy::copy(&all, q, &mut mesh),
            }
        }
        let finish = |mut mesh: MeshData| {
            (!mesh.is_empty()).then(|| {
                crate::slope::set_slopes(&mut mesh, &slopes);
                set_mixes(&mut mesh, Some(&cells.mix), volumes);
                if merge { crate::greedy::merge_far(&mesh) } else { mesh }
            })
        };
        FarMeshed { chunk, version, mesh: finish(mesh), walls: walls.map(finish) }
    };
    let threads = threads.clamp(1, list.len().max(1));
    if threads == 1 {
        return list.into_iter().map(one).collect();
    }
    let mut parts: Vec<Vec<Asked>> = (0..threads).map(|_| Vec::new()).collect();
    for (i, item) in list.into_iter().enumerate() {
        parts[i % threads].push(item);
    }
    std::thread::scope(|s| {
        let handles: Vec<_> = parts.into_iter().map(|part| s.spawn(move || part.into_iter().map(one).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().expect("meshing a far chunk")).collect()
    })
}

/// How many cells from the surface a wall reaches, along the border: the ground of what's
/// drawn beside a far chunk in place of its neighbor sits within a cell of the neighbor's own
/// if it's finer, two if it's coarser (twice the cell).
const WALL_DEPTH: i32 = 2;

/// The cells of far chunk `data` that get a wall on each side (in [`FarChunk::beside`]'s
/// order), by index: those on its border that are solid, facing a solid cell of the far chunk
/// `beside` it there, and within [`WALL_DEPTH`] cells along the border of one where either
/// isn't: near the surface, where a hole could show. Deep in the ground there's no wall, as
/// nothing drawn beside it could uncover it. Past the border's edges the surface is looked
/// for in `around` (the cells around the far chunk, as far as they're known): a cell there
/// that isn't known to be solid counts as open.
fn wall_cells(data: &sim::ChunkData, beside: &[Arc<FarCells>], around: &crate::slope::Cells) -> [HashSet<usize>; 6] {
    let mut out: [HashSet<usize>; 6] = Default::default();
    if data.cells.uniform() == Some(0) {
        return out;
    }
    let size = api::CHUNK_SIZE as i32;
    let last = (size - 1) as u8;
    let sides = [I64Vec3::X, I64Vec3::NEG_X, I64Vec3::Y, I64Vec3::NEG_Y, I64Vec3::Z, I64Vec3::NEG_Z];
    for (side, cells) in out.iter_mut().enumerate() {
        let Some(theirs) = beside.iter().find(|b| b.data.pos.0 == data.pos.0 + sides[side]) else { continue };
        if theirs.data.cells.uniform() == Some(0) {
            continue;
        }
        let (axis, forward) = (side / 2, side.is_multiple_of(2));
        let (mine_at, theirs_at) = if forward { (last, 0) } else { (0, last) };
        let at = |k: u8, u: i32, v: i32| match axis {
            0 => api::LocalPos::new(k, u as u8, v as u8),
            1 => api::LocalPos::new(u as u8, k, v as u8),
            _ => api::LocalPos::new(u as u8, v as u8, k),
        };
        // Across the border: both solid, the face left out.
        let both: Vec<bool> = (0..size * size).map(|i| data.cells.get(at(mine_at, i % size, i / size)) != 0 && theirs.data.cells.get(at(theirs_at, i % size, i / size)) != 0).collect();
        if both.iter().all(|&b| b) {
            continue;
        }
        let open = |u: i32, v: i32| {
            if u < 0 || v < 0 || u >= size || v >= size {
                let p = at(mine_at, u.rem_euclid(size), v.rem_euclid(size));
                let mut c = [p.x as i32, p.y as i32, p.z as i32];
                // Along the border, `u` and `v` run as `at` lays them out.
                c[[1, 0, 0][axis]] = u;
                c[[2, 2, 1][axis]] = v;
                return around.at(c[0], c[1], c[2]) != Some(true);
            }
            !both[(u + size * v) as usize]
        };
        for v in 0..size {
            for u in 0..size {
                if both[(u + size * v) as usize] && (-WALL_DEPTH..=WALL_DEPTH).any(|dv| (-WALL_DEPTH..=WALL_DEPTH).any(|du| open(u + du, v + dv))) {
                    cells.insert(at(mine_at, u, v).index());
                }
            }
        }
    }
    out
}

/// The index of the cell quad `q` of `mesh` is a face of.
fn cell_behind(mesh: &MeshData, q: usize) -> usize {
    let i = 4 * q;
    let normal = mesh.normals[i];
    let cell: [u8; 3] = std::array::from_fn(|k| {
        let center = mesh.positions[i..i + 4].iter().map(|p| p[k]).sum::<f32>() / 4.0;
        (center - 0.5 * normal[k]).floor().clamp(0.0, (api::CHUNK_SIZE - 1) as f32) as u8
    });
    api::LocalPos::new(cell[0], cell[1], cell[2]).index()
}

/// Which side of its far chunk (in [`FarChunk::beside`]'s order) quad `q` of `mesh` lies on,
/// facing out, if any: a face on the chunk's border, toward the far chunk beside it there.
fn border_side(mesh: &MeshData, q: usize) -> Option<usize> {
    let normal = mesh.normals[4 * q];
    let axis = normal.iter().position(|&c| c != 0.0)?;
    let forward = normal[axis] > 0.0;
    let border = if forward { api::CHUNK_SIZE as f32 } else { 0.0 };
    mesh.positions[4 * q..4 * q + 4].iter().all(|p| p[axis] == border).then_some(2 * axis + usize::from(!forward))
}

/// Every far chunk in `built` with something in it, to mesh with the others beside it, as
/// [`FarTerrain::take_news`] would ask for them all arriving at once.
#[cfg(test)]
pub(crate) fn ask_all(built: Vec<FarBuilt>) -> Vec<Asked> {
    let all: HashMap<FarChunk, Arc<FarCells>> = built.into_iter().filter_map(|b| Some((b.chunk, Arc::new(b.cells?)))).collect();
    all.iter()
        .map(|(&chunk, cells)| {
            let beside = chunk.beside().iter().filter_map(|b| all.get(b).cloned()).collect();
            Asked { version: 0, chunk, cells: Arc::clone(cells), beside }
        })
        .collect()
}

/// What a far face's vertex color's alpha holds: its cell's second material's volume times
/// this, plus the material's share in sixteenths (1 to 16); 0 for none. Far terrain is
/// opaque, so `solid.wgsl` reads it back rather than as alpha (see `far_shade`).
pub const MIX_PACK: f32 = 32.0;

/// Packs each face of a far chunk's mesh with its cell's second material and share (see
/// [`Mix`], [`MIX_PACK`]), from the cell just behind the face. A second material with no look
/// is left out: it has no colors of its own to mix.
pub(crate) fn set_mixes(mesh: &mut MeshData, mix: Option<&Mix>, volumes: &Volumes) {
    for q in 0..mesh.quad_count() {
        let i = 4 * q;
        let normal = mesh.normals[i];
        let mut packed = 0.0;
        if let Some(mix) = mix {
            let mut center = [0.0f32; 3];
            for p in &mesh.positions[i..i + 4] {
                for k in 0..3 {
                    center[k] += p[k] / 4.0;
                }
            }
            let cell = std::array::from_fn::<i32, 3, _>(|k| (center[k] - 0.5 * normal[k]).floor() as i32);
            let size = api::CHUNK_SIZE as i32;
            if cell.iter().all(|&c| (0..size).contains(&c)) {
                let index = api::LocalPos::new(cell[0] as u8, cell[1] as u8, cell[2] as u8).index();
                if let Some((second, share)) = mix.at(index)
                    && let Some(volume) = volumes.of(second)
                {
                    packed = volume as f32 * MIX_PACK + (share * 16.0).round();
                }
            }
        }
        for color in &mut mesh.colors[i..i + 4] {
            color[3] = packed;
        }
    }
}

/// Chunks along each side of the mask: the render origin's chunk and 32 before it, 31 after,
/// every way. The camera stays within about 8 chunks of the origin, so the mask covers the
/// ordinary chunks within some 700 m of it; past that far terrain isn't left out (only seen
/// with a render distance that long, while far chunks over full detail still load).
pub const MASK_SIDE: i64 = 64;

/// Which of the `ordinary` chunks drawn are where, for the far material: a byte a chunk, by X,
/// then Y, then Z, the chunk of the render origin's cell `origin` in the middle; 255 where an
/// ordinary chunk is drawn, 0 elsewhere. Far terrain leaves itself out in those chunks (see
/// `far_hidden` in `solid.wgsl`), as full detail is there.
pub(crate) fn mask_of(ordinary: &HashSet<ChunkPos>, origin: I64Vec3) -> Vec<u8> {
    let middle = BoxelPos(origin).chunk().0;
    let mut mask = vec![0; (MASK_SIDE * MASK_SIDE * MASK_SIDE) as usize];
    for chunk in ordinary {
        let at = chunk.0 - middle + I64Vec3::splat(MASK_SIDE / 2);
        if at.min_element() >= 0 && at.max_element() < MASK_SIDE {
            mask[(at.x + MASK_SIDE * (at.y + MASK_SIDE * at.z)) as usize] = 255;
        }
    }
    mask
}

/// Where the render origin's cell is in its chunk, in boxels: what the far material adds to a
/// fragment's place from the origin to find its chunk in the mask.
pub(crate) fn mask_offset(origin: I64Vec3) -> Vec3 {
    origin.rem_euclid(I64Vec3::splat(api::CHUNK_SIZE)).as_vec3()
}

/// A mask (see [`mask_of`]) as a texture, `side` chunks each way.
pub fn mask_image(mask: Vec<u8>, side: u32) -> Image {
    let mut image = Image::new(
        Extent3d { width: side, height: side, depth_or_array_layers: side },
        TextureDimension::D3,
        mask,
        TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// How far the view reaches, as tuned, far terrain on or not.
pub fn reach_of(view: &ViewTuning) -> Reach {
    Reach {
        detail: view.render_distance,
        far: view.far_distance,
        pixels: view.far_pixels,
        screen: view.screen,
        hold: view.far_hold,
        mix: view.far_mix,
    }
}

/// How far the view reaches, if far terrain is on.
pub fn reach(view: &ViewTuning, switches: &ViewSwitches) -> Option<Reach> {
    let reach = reach_of(view);
    (switches.far_terrain && reach.has_far()).then_some(reach)
}

/// Makes the far material once the block material exists (the same, read plainly), and keeps
/// its creases as tuned.
pub(crate) fn sync_far_material(
    chunks: Option<Res<ChunkMeshes>>,
    view: Res<ViewTuning>,
    mut far: ResMut<FarTerrain>,
    mut materials: ResMut<Assets<SolidMaterial>>,
) {
    let crease = view.far_crease;
    match &far.material {
        Some(handle) => {
            if materials.get(handle).is_some_and(|m| m.far.x != crease)
                && let Some(m) = materials.get_mut(handle)
            {
                m.far.x = crease;
            }
        }
        None => {
            let Some(chunks) = chunks else { return };
            let Some(mut material) = materials.get(&chunks.material).cloned() else { return };
            material.place = FAR_PLACE;
            material.far = Vec4::new(crease, 0.0, 0.0, 0.0);
            far.material = Some(materials.add(material));
            // Its mask comes with the next pick.
            far.mask = None;
        }
    }
}

/// Picks which far and ordinary chunks are drawn, from the camera, shows and hides the far
/// meshes to match (see [`pick`]), and gives the far material the mask of the ordinary chunks
/// drawn whenever those or the render origin change.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pick_far(
    view: Res<ViewTuning>,
    switches: Res<ViewSwitches>,
    origin: Res<RenderOrigin>,
    camera: Single<&Transform, With<PlayerCamera>>,
    bg: Res<BackgroundMesher>,
    world: Res<ClientWorld>,
    mut far: ResMut<FarTerrain>,
    mut meshes: Query<(&FarMesh, &mut Visibility)>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<SolidMaterial>>,
) {
    let Some(reach) = reach(&view, &switches) else {
        if far.active {
            far.active = false;
            far.drawn.clear();
            far.ordinary.clear();
            far.stats = FarViewStats::default();
            for (_, mut v) in &mut meshes {
                v.set_if_neq(Visibility::Hidden);
            }
        }
        return;
    };
    far.active = true;
    let eye = coords::from_render(camera.translation);
    let cell = eye.floor();
    let eye = WorldPos::new(origin.cell + cell.as_i64vec3(), eye - cell);
    let (drawn, ordinary) = pick(&reach, eye, &far, &|p| bg.is_shown(p) || world.is_air(p));
    let moved = far.mask.as_ref().is_none_or(|m| m.1 != origin.cell);
    if (moved || ordinary != far.ordinary)
        && let Some(handle) = far.material.clone()
    {
        let mask = mask_of(&ordinary, origin.cell);
        if (moved || far.mask.as_ref().is_none_or(|m| m.0 != mask))
            && let Some(m) = materials.get_mut(&handle)
        {
            m.mask = images.add(mask_image(mask.clone(), MASK_SIDE as u32));
            let offset = mask_offset(origin.cell);
            m.far = Vec4::new(m.far.x, offset.x, offset.y, offset.z);
            far.mask = Some((mask, origin.cell));
        }
    }
    far.ordinary = ordinary;
    if drawn != far.drawn {
        for (mesh, mut v) in &mut meshes {
            v.set_if_neq(if shown(&drawn, mesh.chunk, mesh.wall) { Visibility::Inherited } else { Visibility::Hidden });
        }
        far.drawn = drawn;
    }
    let drawn: Vec<(FarChunk, &Entry)> = far.drawn.iter().filter_map(|&c| Some((c, far.chunks.get(&c)?))).collect();
    let walls = |c: FarChunk, e: &Entry| (0..6).filter(|&side| shown(&far.drawn, c, Some(side as u8))).map(|side| e.wall_quads[side]).sum::<usize>();
    let triangles = drawn.iter().map(|&(c, e)| (e.quads + walls(c, e)) * 2).sum();
    let chunks = drawn.iter().filter(|(_, e)| e.entity.is_some()).count();
    far.stats = FarViewStats { far: far.drawn.len(), ordinary: far.ordinary.len(), chunks, triangles };
}

/// The far and ordinary chunks to draw from `eye`, picked by the octree (see [`Reach::pick`]).
/// A far chunk is ready to draw once its mesh is in; an ordinary chunk once it's `ready`: its
/// mesh is back, or it's known air (see [`ClientWorld::is_air`]), which is never sent and
/// mustn't keep the far chunk holding it from giving way.
fn pick(reach: &Reach, eye: WorldPos, far: &FarTerrain, ready: &dyn Fn(ChunkPos) -> bool) -> (HashSet<FarChunk>, HashSet<ChunkPos>) {
    let mut terrains: Vec<usize> = far.chunks.keys().map(|c| c.terrain).collect();
    terrains.push(0);
    terrains.sort_unstable();
    terrains.dedup();
    let ready = |c: FarChunk| if c.level == 0 { ready(ChunkPos(c.pos)) } else { far.chunks.get(&c).is_some_and(|e| e.meshed) };
    let mut drawn = HashSet::new();
    let mut ordinary = HashSet::new();
    for terrain in terrains {
        for c in reach.pick(terrain, eye, &ready) {
            if c.level == 0 {
                ordinary.insert(ChunkPos(c.pos));
            } else {
                drawn.insert(c);
            }
        }
    }
    (drawn, ordinary)
}

/// Keeps far meshes where they belong as the render origin moves.
pub(crate) fn sync_far_transforms(origin: Res<RenderOrigin>, mut meshes: Query<(&FarMesh, &mut Transform)>) {
    if !origin.is_changed() {
        return;
    }
    for (mesh, mut transform) in &mut meshes {
        *transform = far_transform(mesh.chunk, origin.cell);
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::world::CommandQueue;

    use super::*;
    use sim::ChunkData;

    fn far(level: u8, x: i64, y: i64, z: i64) -> FarChunk {
        FarChunk { terrain: 0, level, pos: I64Vec3::new(x, y, z) }
    }

    /// Cells of stone at the far chunk `at` where `solid` says.
    fn stone_cells(blocks: &BlockRegistry, at: FarChunk, solid: impl Fn(api::LocalPos) -> bool) -> FarCells {
        let stone = blocks.id("t:stone").expect("registered");
        let mut data = ChunkData {
            pos: ChunkPos(at.pos),
            lists: vec![Vec::new(), vec![sim::Occupant::new(stone)]],
            cells: sim::Layer::Uniform(0),
            light: sim::Layer::Uniform(sim::Light::SKY),
            records: Vec::new(),
        };
        for i in 0..api::CHUNK_VOLUME {
            if solid(api::LocalPos::from_index(i)) {
                data.cells.set_at(i, 1);
            }
        }
        data.cells.compact();
        FarCells { data, mix: Mix::default() }
    }

    fn stone() -> (BlockRegistry, Volumes) {
        let mut blocks = BlockRegistry::default();
        blocks.register(api::BlockDef::new("t:stone", true, [0; 3])).unwrap();
        let volumes = Volumes::build(&sim::LookRegistry::default(), &blocks);
        (blocks, volumes)
    }

    #[test]
    fn a_built_floor_with_no_air_sent_around_it_is_drawn_at_full_detail() {
        // The test area: only the floor chunk exists, no terrain near it, so the air around it
        // is never sent. The far chunk holding it is ready, but empty (its terrain cleared).
        let mut terrain = FarTerrain::default();
        terrain.chunks.insert(far(1, 0, 0, 0), Entry::new(1, true));
        let floor = ChunkPos(I64Vec3::ZERO);
        let eye = WorldPos::new(I64Vec3::new(16, 16, 18), Vec3::splat(0.5));
        let reach = Reach::default();
        // The air around it is known air (see `ClientWorld::is_air`): ready, as nothing comes.
        let (drawn, ordinary) = pick(&reach, eye, &terrain, &|_| true);
        assert!(ordinary.contains(&floor), "the floor is drawn");
        assert!(!drawn.contains(&far(1, 0, 0, 0)), "not the empty far chunk over it");
        // Over terrain, a chunk not sent yet is still waited for: the far chunk stands in, and
        // the floor, which is in, stays drawn (the far chunk is left out there, by the mask).
        let (drawn, ordinary) = pick(&reach, eye, &terrain, &|p| p == floor);
        assert!(drawn.contains(&far(1, 0, 0, 0)));
        assert_eq!(ordinary, HashSet::from([floor]));
    }

    #[test]
    fn far_chunks_sit_at_their_level_scaled_up() {
        let t = far_transform(far(2, 1, -1, 2), I64Vec3::ZERO);
        assert_eq!(t.scale, Vec3::splat(4.0));
        // Its corner, 128 boxels east, south and up, in render space.
        assert_eq!(coords::from_render(t.translation), Vec3::new(128.0, -128.0, 256.0));
    }

    #[test]
    fn the_bench_waits_for_far_chunks_still_being_meshed() {
        let chunk = far(1, 0, 0, 0);
        let mut terrain = FarTerrain::default();
        terrain.chunks.insert(chunk, Entry::new(1, false));
        assert_eq!(terrain.unmeshed(), 1);
        let mut world = World::new();
        world.insert_resource(terrain);
        assert!(crate::bench::switches::loading(&world));
        world.resource_mut::<FarTerrain>().chunks.get_mut(&chunk).unwrap().meshed = true;
        assert!(!crate::bench::switches::loading(&world));
    }

    #[test]
    fn the_shader_knows_how_mixes_are_packed() {
        let shader = include_str!("solid.wgsl");
        assert!(shader.contains(&format!("const FAR_MIX_PACK: f32 = {MIX_PACK:.1};")), "solid.wgsl's FAR_MIX_PACK should be {MIX_PACK}");
    }

    #[test]
    fn the_shader_knows_the_masks_size() {
        let shader = include_str!("solid.wgsl");
        assert!(shader.contains(&format!("const FAR_MASK_SIDE: i32 = {MASK_SIDE};")), "solid.wgsl's FAR_MASK_SIDE should be {MASK_SIDE}");
        assert!(shader.contains(&format!("const CHUNK: f32 = {:.1};", api::CHUNK_SIZE as f32)));
    }

    #[test]
    fn the_mask_marks_the_ordinary_chunks_drawn_around_the_origins_chunk() {
        // The origin's cell is in chunk (-1, 2, 0), 5 boxels in along X.
        let origin = I64Vec3::new(-27, 64, 31);
        assert_eq!(mask_offset(origin), Vec3::new(5.0, 0.0, 31.0));
        let texel = |x: i64, y: i64, z: i64| (x + MASK_SIDE * (y + MASK_SIDE * z)) as usize;
        let half = MASK_SIDE / 2;
        let chunks = [
            // The origin's own chunk: the middle texel.
            (I64Vec3::new(-1, 2, 0), Some(texel(half, half, half))),
            // The first and last chunks the mask reaches, and the first it doesn't.
            (I64Vec3::new(-1 - half, 2, 0), Some(texel(0, half, half))),
            (I64Vec3::new(-1, 2 + half - 1, 0), Some(texel(half, MASK_SIDE - 1, half))),
            (I64Vec3::new(-1, 2, half), None),
            (I64Vec3::new(-2 - half, 2, 0), None),
            (I64Vec3::new(0, 1, -3), Some(texel(half + 1, half - 1, half - 3))),
        ];
        let ordinary: HashSet<ChunkPos> = chunks.iter().map(|c| ChunkPos(c.0)).collect();
        let mask = mask_of(&ordinary, origin);
        let marked: HashSet<usize> = (0..mask.len()).filter(|&i| mask[i] != 0).collect();
        assert_eq!(marked, chunks.iter().filter_map(|c| c.1).collect());
        assert!(marked.iter().all(|&i| mask[i] == 255));
    }

    #[test]
    fn each_far_face_carries_the_mix_of_the_cell_behind_it() {
        let mut blocks = BlockRegistry::default();
        let mut looks = sim::LookRegistry::default();
        blocks.register(api::BlockDef::new("t:stone", true, [0; 3])).unwrap();
        let grass = blocks.register(api::BlockDef::new("t:grass", true, [0; 3]).with_look("t:grass")).unwrap();
        let rgba = vec![120; api::LookDef::VOLUME * 4];
        looks.register(api::LookDef { name: "t:grass".into(), turns: false, rgba, sheen: None, glow: 0.0 }).unwrap();
        let volumes = Volumes::build(&looks, &blocks);
        let green = volumes.of(grass).expect("grass has a look");
        // One stone cell at (3, 4, 5), a quarter grass; another at (9, 9, 9), unmixed.
        let mixed = api::LocalPos::new(3, 4, 5);
        let chunk = far(1, 0, 0, 0);
        let mut cells = stone_cells(&blocks, chunk, |p| p == mixed || p == api::LocalPos::new(9, 9, 9));
        cells.mix.set(mixed.index(), Some((grass, 0.25)));
        let out = mesh_far(vec![Asked { version: 1, chunk, cells: Arc::new(cells), beside: Vec::new() }], &blocks, &volumes, false, 1);
        let mesh = out[0].mesh.as_ref().expect("faces");
        let (mut near, mut plain) = (0, 0);
        for q in 0..mesh.quad_count() {
            let p = mesh.positions[4 * q];
            let alpha = mesh.colors[4 * q][3];
            if p.iter().all(|&c| c < 7.0) {
                assert_eq!(alpha, green as f32 * MIX_PACK + 4.0, "a quarter grass");
                near += 1;
            } else {
                assert_eq!(alpha, 0.0, "no mix");
                plain += 1;
            }
        }
        assert_eq!((near, plain), (6, 6));
    }

    /// The directions `mesh`'s faces face.
    fn facing(mesh: &MeshData) -> HashSet<[i32; 3]> {
        mesh.normals.iter().map(|n| n.map(|c| c as i32)).collect()
    }

    #[test]
    fn faces_toward_a_solid_far_chunk_beside_are_left_out() {
        let (blocks, volumes) = stone();
        // Solid all through, alone: a wall every way.
        let chunk = far(1, 0, 0, 0);
        let solid = Arc::new(stone_cells(&blocks, chunk, |_| true));
        let alone = mesh_far(vec![Asked { version: 1, chunk, cells: Arc::clone(&solid), beside: Vec::new() }], &blocks, &volumes, true, 1);
        assert_eq!(facing(alone[0].mesh.as_ref().unwrap()).len(), 6);
        // Buried: solid far chunks on every side but over it, which is empty.
        let beside: Vec<Arc<FarCells>> = chunk.beside().iter().filter(|b| b.pos.z <= 0).map(|&b| Arc::new(stone_cells(&blocks, b, |_| true))).collect();
        assert_eq!(beside.len(), 5);
        let buried = mesh_far(vec![Asked { version: 2, chunk, cells: solid, beside }], &blocks, &volumes, true, 1);
        assert_eq!(facing(buried[0].mesh.as_ref().unwrap()), HashSet::from([[0, 0, 1]]), "only its top shows");
    }

    #[test]
    fn faces_left_out_toward_a_far_chunk_beside_are_kept_as_that_sides_wall() {
        let (blocks, volumes) = stone();
        // Ground filling the lower half, with the same east of it and nothing else around.
        let chunk = far(2, 0, 0, 0);
        let ground = |c: FarChunk| Arc::new(stone_cells(&blocks, c, |p| p.z < 16));
        let east = chunk.beside()[0];
        let meshed = mesh_far(vec![Asked { version: 1, chunk, cells: ground(chunk), beside: vec![ground(east)] }], &blocks, &volumes, true, 1).pop().unwrap();
        // Its own mesh has no face toward the one east of it; that side's wall has them all,
        // the lower half of the border, and the other sides have no wall.
        let mesh = meshed.mesh.as_ref().unwrap();
        assert!(!facing(mesh).contains(&[1, 0, 0]));
        // Only near the surface, where a hole could show: the top two cells of the ground all
        // along, and all of it within two cells of the border's edges.
        let wall = meshed.walls[0].as_ref().expect("a wall east");
        assert_eq!(facing(wall), HashSet::from([[1, 0, 0]]));
        assert!(wall.positions.iter().all(|p| p[0] == 32.0 && p[2] <= 16.0));
        assert!(wall.positions.iter().all(|p| p[2] >= 14.0 || p[1] <= 2.0 || p[1] >= 30.0 || p[2] <= 2.0));
        let area = |m: &MeshData| {
            (0..m.indices.len() / 3)
                .map(|t| {
                    let [a, b, c] = std::array::from_fn(|k| Vec3::from_array(m.positions[m.indices[3 * t + k] as usize]));
                    (b - a).cross(c - a).length() / 2.0
                })
                .sum::<f32>()
        };
        assert_eq!(area(wall), 32.0 * 2.0 + 2.0 * 2.0 * 14.0 + 28.0 * 2.0);
        assert!(meshed.walls[1..].iter().all(Option::is_none));
        // Alone, the same faces are in its own mesh and it has no walls.
        let alone = mesh_far(vec![Asked { version: 1, chunk, cells: ground(chunk), beside: Vec::new() }], &blocks, &volumes, true, 1).pop().unwrap();
        assert!(facing(alone.mesh.as_ref().unwrap()).contains(&[1, 0, 0]));
        assert!(alone.walls.iter().all(Option::is_none));
        // Deep in the ground, solid all across the border: no wall, nothing could uncover it.
        let rock = |c: FarChunk| Arc::new(stone_cells(&blocks, c, |_| true));
        let deep = mesh_far(vec![Asked { version: 1, chunk, cells: rock(chunk), beside: vec![rock(east)] }], &blocks, &volumes, true, 1).pop().unwrap();
        assert!(deep.walls.iter().all(Option::is_none));
    }

    #[test]
    fn a_wall_is_shown_only_while_the_far_chunk_beside_it_isnt_drawn() {
        let (a, east) = (far(2, 0, 0, 0), far(2, 1, 0, 0));
        // Both drawn at their level: the ground carries on, no wall.
        let both = HashSet::from([a, east]);
        assert!(shown(&both, a, None) && !shown(&both, a, Some(0)));
        // The one east gave way to finer far chunks (or a coarser one stands in): the wall plugs
        // the gap its lower ground would leave. The other sides' walls likewise.
        let finer: HashSet<FarChunk> = east.children().into_iter().chain([a]).collect();
        assert!(shown(&finer, a, None) && shown(&finer, a, Some(0)) && shown(&finer, a, Some(3)));
        // Not drawn itself: nothing of it shows.
        let gone = HashSet::from([east]);
        assert!(!shown(&gone, a, None) && !shown(&gone, a, Some(0)));
    }

    #[test]
    fn a_far_chunk_is_meshed_again_when_one_beside_it_comes() {
        let (blocks, _) = stone();
        let mut world = World::new();
        let mut queue = CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        let mut terrain = FarTerrain::default();
        let (a, b, air) = (far(1, 0, 0, 0), far(1, 1, 0, 0), far(1, 0, 1, 0));
        let built = |c: FarChunk, solid: bool| FarNews::Built(FarBuilt { chunk: c, cells: solid.then(|| stone_cells(&blocks, c, |_| true)) });
        let asked = terrain.take_news(vec![built(a, true)], &mut commands);
        assert_eq!(asked.iter().map(|x| x.chunk).collect::<Vec<_>>(), [a]);
        assert!(asked[0].beside.is_empty());
        // One east of it: both meshed, each with the other beside it.
        let asked = terrain.take_news(vec![built(b, true)], &mut commands);
        let mut got: Vec<(FarChunk, usize)> = asked.iter().map(|x| (x.chunk, x.beside.len())).collect();
        got.sort();
        assert_eq!(got, [(a, 1), (b, 1)]);
        // One over it, solid only in its top half: it doesn't touch the first, so the first
        // isn't meshed again.
        let up = far(1, 0, 0, 1);
        let asked = terrain.take_news(vec![FarNews::Built(FarBuilt { chunk: up, cells: Some(stone_cells(&blocks, up, |p| p.z >= 16)) })], &mut commands);
        assert_eq!(asked.iter().map(|x| (x.chunk, x.beside.len())).collect::<Vec<_>>(), [(up, 1)]);
        // An empty one beside it needs no mesh and changes nothing beside it.
        let asked = terrain.take_news(vec![built(air, false)], &mut commands);
        assert!(asked.is_empty());
        assert_eq!(terrain.unmeshed(), 3, "the empty one is ready at once");
        // The east one built again empty: the first loses it from beside, so is meshed again.
        let asked = terrain.take_news(vec![built(b, false)], &mut commands);
        assert_eq!(asked.iter().map(|x| (x.chunk, x.beside.len())).collect::<Vec<_>>(), [(a, 1)], "with the one over it");
        // A mesh asked for before is thrown away: its version is old.
        let old = terrain.chunks[&a].version - 1;
        terrain.material = Some(Handle::default());
        terrain.put(vec![FarMeshed { chunk: a, version: old, mesh: None, walls: Default::default() }], &mut commands, &mut Assets::default(), I64Vec3::ZERO);
        assert!(!terrain.chunks[&a].meshed);
        queue.apply(&mut world);
    }
}

#[cfg(test)]
mod timing {
    use std::time::Instant;

    use super::*;
    use crate::world::testing::base_sim;

    /// Times meshing the far chunks around the terrain's center, step by step. Run with
    /// `--release --ignored --nocapture`.
    #[test]
    #[ignore]
    fn time_far_meshing() {
        let only: Option<f32> = std::env::var("FAR_PIXELS").ok().and_then(|v| v.parse().ok());
        let mix: u8 = std::env::var("FAR_MIX").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        for pixels in only.map_or(vec![5.0, 7.0, 9.0, 12.0], |p| vec![p]) {
            time_far_meshing_at(pixels, mix);
        }
    }

    /// Times [`mesh_far`] whole, walls and all, on one thread, for the far chunks around the
    /// terrain's center. Run with `--release --ignored --nocapture`.
    #[test]
    #[ignore]
    fn time_mesh_far() {
        let pixels: f32 = std::env::var("FAR_PIXELS").ok().and_then(|v| v.parse().ok()).unwrap_or(sim::far::DEFAULT_PIXELS);
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        sim.teleport_player(WorldPos::at_boxel(base::TERRAIN_CENTER.offset(I64Vec3::new(0, 0, 120))));
        sim.set_reach(Reach { detail: 256.0, far: 2048.0, pixels, ..Reach::default() });
        sim.generate_terrain().unwrap();
        sim.far_all();
        let volumes = Volumes::build(sim.looks(), sim.blocks());
        let asked = ask_all(sim.take_far_loaded());
        let count = asked.len();
        let clock = Instant::now();
        let meshed = mesh_far(asked, sim.blocks(), &volumes, true, 1);
        let seconds = clock.elapsed().as_secs_f32();
        let quads: usize = meshed.iter().filter_map(|m| m.mesh.as_ref()).map(MeshData::quad_count).sum();
        let walls: usize = meshed.iter().flat_map(|m| m.walls.iter().flatten()).map(MeshData::quad_count).sum();
        let wall_meshes = meshed.iter().flat_map(|m| m.walls.iter().flatten()).count();
        println!("pixels {pixels}: mesh_far of {count} far chunks in {seconds:.2} s | {quads} quads, {walls} in {wall_meshes} walls");
    }

    fn time_far_meshing_at(pixels: f32, mix: u8) {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        sim.teleport_player(WorldPos::at_boxel(base::TERRAIN_CENTER.offset(I64Vec3::new(0, 0, 120))));
        sim.set_reach(Reach { detail: 256.0, far: 2048.0, pixels, mix, ..Reach::default() });
        sim.generate_terrain().unwrap();
        let clock = Instant::now();
        sim.far_all();
        let (build_wall, build_s) = (clock.elapsed().as_secs_f32(), sim.far_stats().seconds);
        let built = sim.take_far_loaded();
        let count = built.len();
        let volumes = Volumes::build(sim.looks(), sim.blocks());
        let ao = AoTuning::default();
        let (mut mesh_s, mut slope_s, mut merge_s, mut chunks, mut quads, mut merged) = (0.0f32, 0.0f32, 0.0f32, 0, 0, 0);
        let mut mixed = 0;
        let mut by_level = [0usize; 8];
        let mut per_chunk: HashMap<FarChunk, usize> = HashMap::new();
        let mut empty: HashSet<FarChunk> = built.iter().filter(|b| b.cells.is_none()).map(|b| b.chunk).collect();
        for Asked { chunk, cells, beside, .. } in ask_all(built) {
            let mut grid = Grid::default();
            grid.import_chunk(&cells.data);
            for b in &beside {
                grid.import_chunk(&b.data);
            }
            let pos = cells.data.pos;
            chunks += 1;
            let clock = Instant::now();
            let mut m = crate::mesher::mesh_chunks_on(&grid, &[pos], sim.blocks(), &volumes, &ao, false, 1).pop().unwrap_or_default();
            mesh_s += clock.elapsed().as_secs_f32();
            let clock = Instant::now();
            crate::slope::set_slopes(&mut m, &crate::slope::Cells::of(&grid, pos));
            set_mixes(&mut m, Some(&cells.mix), &volumes);
            mixed += (0..m.quad_count()).filter(|&q| m.colors[4 * q][3] != 0.0).count();
            slope_s += clock.elapsed().as_secs_f32();
            let clock = Instant::now();
            quads += m.quad_count();
            let q = crate::greedy::merge_far(&m).quad_count();
            merged += q;
            by_level[chunk.level as usize] += q;
            per_chunk.insert(chunk, q);
            merge_s += clock.elapsed().as_secs_f32();
        }
        empty.extend(per_chunk.keys().copied());
        let pos = sim.player().unwrap().pos();
        let picked = sim.reach().pick(0, pos, &|c| c.level == 0 || empty.contains(&c));
        let drawn: usize = picked.iter().filter_map(|c| per_chunk.get(c)).sum();
        println!(
            "pixels {pixels} mix {mix}: built in {build_wall:.1} s ({build_s:.1} s on threads) | {count} far chunks, {chunks} with cells: meshing {mesh_s:.2} s, slopes {slope_s:.2} s, merging {merge_s:.2} s | {quads} quads ({mixed} mixed), {merged} merged, by level {by_level:?} | {drawn} far quads drawn"
        );
    }
}
