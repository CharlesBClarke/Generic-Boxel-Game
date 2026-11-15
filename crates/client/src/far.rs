//! Far terrain on the client: the far chunks the server sends (see [`sim::far`]), meshed in
//! the background with the far chunks of their level beside them (so faces between two are
//! left out, and a far chunk is meshed again when one beside it comes), scaled to their level
//! and drawn with the far material (each face as the slope of the ground around it, see
//! [`crate::slope`]: the average colors and light of the full-detail faces such a slope is
//! made of, mixed with its cell's second material, no AO or edges, its shadow looked up a cell
//! higher); and which level is drawn where, picked by the octree from the camera as far
//! chunks become ready, which also says which ordinary chunks are drawn (see
//! [`crate::view::apply_render_distance`]).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use api::{BoxelPos, ChunkPos, I64Vec3, WorldPos};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
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
}

/// One far chunk's mesh.
#[derive(Component)]
pub(crate) struct FarMesh {
    chunk: FarChunk,
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
    pub stats: FarViewStats,
}

impl FarTerrain {
    /// Forgets every far chunk, as when the world comes anew.
    pub fn clear(&mut self, commands: &mut Commands) {
        for (_, entry) in self.chunks.drain() {
            if let Some(e) = entry.entity {
                commands.entity(e).despawn();
            }
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
                    if let Some(entry) = self.chunks.remove(&chunk)
                        && let Some(e) = entry.entity
                    {
                        commands.entity(e).despawn();
                    }
                }
                FarNews::Built(FarBuilt { chunk, cells }) => {
                    self.next_version += 1;
                    let version = self.next_version;
                    // One built again keeps drawing the old mesh until the new one is in.
                    let entry = self.chunks.entry(chunk).or_insert(Entry { version, meshed: false, cells: None, entity: None, quads: 0 });
                    let had = entry.cells.is_some();
                    entry.version = version;
                    entry.cells = cells.map(Arc::new);
                    if entry.cells.is_some() {
                        again.insert(chunk);
                    } else {
                        again.remove(&chunk);
                        if let Some(e) = entry.entity.take() {
                            commands.entity(e).despawn();
                        }
                        entry.meshed = true;
                        entry.quads = 0;
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
        for FarMeshed { chunk, version, mesh } in batch {
            let Some(entry) = self.chunks.get_mut(&chunk) else { continue };
            if entry.version != version {
                continue;
            }
            if let Some(e) = entry.entity.take() {
                commands.entity(e).despawn();
            }
            entry.quads = mesh.as_ref().map_or(0, |m| m.quad_count());
            let shown = if self.drawn.contains(&chunk) { Visibility::Inherited } else { Visibility::Hidden };
            if let Some(data) = mesh {
                let entity = commands
                    .spawn((
                        Mesh3d(meshes.add(crate::to_bevy_mesh(data))),
                        MeshMaterial3d(material.clone()),
                        far_transform(chunk, origin),
                        FarMesh { chunk },
                        NotShadowCaster,
                        shown,
                    ))
                    .id();
                entry.entity = Some(entity);
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
/// them left out where they're solid, faces toward a missing one kept, as walls that plug the
/// cracks between levels), every face given the slope of the ground around it (see
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
        let mut mesh = crate::mesher::mesh_chunks_on(&grid, &[pos], blocks, volumes, &ao, false, 1).pop().unwrap_or_default();
        let mesh = (!mesh.is_empty()).then(|| {
            crate::slope::set_slopes(&mut mesh, &crate::slope::Cells::of(&grid, pos));
            set_mixes(&mut mesh, Some(&cells.mix), volumes);
            if merge { crate::greedy::merge_far(&mesh) } else { mesh }
        });
        FarMeshed { chunk, version, mesh }
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
    let crease = Vec4::new(view.far_crease, 0.0, 0.0, 0.0);
    match &far.material {
        Some(handle) => {
            if materials.get(handle).is_some_and(|m| m.far != crease)
                && let Some(m) = materials.get_mut(handle)
            {
                m.far = crease;
            }
        }
        None => {
            let Some(chunks) = chunks else { return };
            let Some(mut material) = materials.get(&chunks.material).cloned() else { return };
            material.place = FAR_PLACE;
            material.far = crease;
            far.material = Some(materials.add(material));
        }
    }
}

/// Picks which far and ordinary chunks are drawn, from the camera, and shows and hides the far
/// meshes to match (see [`pick`]).
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
    far.ordinary = ordinary;
    if drawn != far.drawn {
        for (mesh, mut v) in &mut meshes {
            v.set_if_neq(if drawn.contains(&mesh.chunk) { Visibility::Inherited } else { Visibility::Hidden });
        }
        far.drawn = drawn;
    }
    let drawn: Vec<&Entry> = far.drawn.iter().filter_map(|c| far.chunks.get(c)).collect();
    let triangles = drawn.iter().map(|e| e.quads * 2).sum();
    let chunks = drawn.iter().filter(|e| e.entity.is_some()).count();
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
        terrain.chunks.insert(far(1, 0, 0, 0), Entry { version: 1, meshed: true, cells: None, entity: None, quads: 0 });
        let floor = ChunkPos(I64Vec3::ZERO);
        let eye = WorldPos::new(I64Vec3::new(16, 16, 18), Vec3::splat(0.5));
        let reach = Reach::default();
        // The air around it is known air (see `ClientWorld::is_air`): ready, as nothing comes.
        let (drawn, ordinary) = pick(&reach, eye, &terrain, &|_| true);
        assert!(ordinary.contains(&floor), "the floor is drawn");
        assert!(!drawn.contains(&far(1, 0, 0, 0)), "not the empty far chunk over it");
        // Over terrain, a chunk not sent yet is still waited for: the far chunk stands in.
        let (drawn, ordinary) = pick(&reach, eye, &terrain, &|p| p == floor);
        assert!(drawn.contains(&far(1, 0, 0, 0)) && !ordinary.contains(&floor));
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
        terrain.chunks.insert(chunk, Entry { version: 1, meshed: false, cells: None, entity: None, quads: 0 });
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
        terrain.put(vec![FarMeshed { chunk: a, version: old, mesh: None }], &mut commands, &mut Assets::default(), I64Vec3::ZERO);
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
