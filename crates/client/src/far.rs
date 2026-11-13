//! Far terrain on the client: the far columns the server sends (see [`sim::far`]), meshed in
//! the background each on its own, scaled to their level and drawn with the far material (each
//! face its look's average color, no AO or edges, its shadow looked up a cell higher); and which level is drawn where, picked
//! by the quadtree from the camera as columns become ready, which also says which ordinary
//! columns are drawn (see [`crate::view::apply_render_distance`]).

use std::collections::{HashMap, HashSet};

use api::{BoxelPos, ChunkPos, I64Vec3, WorldPos};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use sim::far::{FarColumn, Reach};
use sim::{BlockRegistry, ChunkData, Grid};

use crate::editing::ChunkMeshes;
use crate::meshing::BackgroundMesher;
use crate::mesher::{AoTuning, MeshData};
use crate::solid::SolidMaterial;
use crate::view::{ViewSwitches, ViewTuning};
use crate::volumes::Volumes;
use crate::world::FarNews;
use crate::{PlayerCamera, RenderOrigin, coords};

/// `SolidMaterial::place` for far meshes: the shader's sign to draw them plainly.
pub const FAR_PLACE: UVec4 = UVec4::new(0, 0, 0, 2);

/// A far column meshed in the background, each of its chunks with geometry.
pub(crate) struct FarMeshed {
    pub column: FarColumn,
    pub version: u64,
    pub meshes: Vec<(ChunkPos, MeshData)>,
}

/// One far chunk's mesh.
#[derive(Component)]
pub(crate) struct FarMesh {
    column: FarColumn,
    pos: ChunkPos,
}

/// A far column the server sent.
struct Entry {
    /// Goes up each time it's sent again, so an older mesh coming back is thrown away.
    version: u64,
    /// Its meshes are in, so it can be drawn.
    meshed: bool,
    entities: Vec<Entity>,
    quads: usize,
}

/// What far terrain draws, for the readout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FarViewStats {
    /// Far columns drawn, and ordinary ones.
    pub far: usize,
    pub ordinary: usize,
    /// Far chunks with geometry drawn.
    pub chunks: usize,
    /// Triangles in the far columns drawn.
    pub triangles: usize,
}

/// The far columns the client has, and which columns are drawn.
#[derive(Resource, Default)]
pub(crate) struct FarTerrain {
    columns: HashMap<FarColumn, Entry>,
    next_version: u64,
    material: Option<Handle<SolidMaterial>>,
    /// Whether far terrain is on: the view reaches past full detail.
    pub active: bool,
    /// The far columns drawn.
    drawn: HashSet<FarColumn>,
    /// The ordinary columns drawn, by chunk X and Y.
    pub ordinary: HashSet<(i64, i64)>,
    pub stats: FarViewStats,
}

impl FarTerrain {
    /// Forgets every far column, as when the world comes anew.
    pub fn clear(&mut self, commands: &mut Commands) {
        for (_, entry) in self.columns.drain() {
            for e in entry.entities {
                commands.entity(e).despawn();
            }
        }
        self.drawn.clear();
    }

    /// Takes in far columns built and dropped, and returns the built ones to mesh.
    pub fn take_news(&mut self, news: Vec<FarNews>, commands: &mut Commands) -> Vec<(FarColumn, u64, Vec<ChunkData>)> {
        let mut asked: Vec<(FarColumn, u64, Vec<ChunkData>)> = Vec::new();
        for n in news {
            match n {
                FarNews::Gone(column) => {
                    asked.retain(|(c, _, _)| *c != column);
                    if let Some(entry) = self.columns.remove(&column) {
                        for e in entry.entities {
                            commands.entity(e).despawn();
                        }
                    }
                }
                FarNews::Built(built) => {
                    self.next_version += 1;
                    let version = self.next_version;
                    // One built again keeps drawing the old meshes until the new ones are in.
                    let entry = self.columns.entry(built.column).or_insert(Entry { version, meshed: false, entities: Vec::new(), quads: 0 });
                    entry.version = version;
                    asked.retain(|(c, _, _)| *c != built.column);
                    asked.push((built.column, version, built.chunks));
                }
            }
        }
        asked
    }

    /// Puts in meshes that came back, replacing a column's old ones.
    pub fn put(&mut self, batch: Vec<FarMeshed>, commands: &mut Commands, meshes: &mut Assets<Mesh>, origin: I64Vec3) {
        let Some(material) = self.material.clone() else { return };
        for FarMeshed { column, version, meshes: list } in batch {
            let Some(entry) = self.columns.get_mut(&column) else { continue };
            if entry.version != version {
                continue;
            }
            for e in entry.entities.drain(..) {
                commands.entity(e).despawn();
            }
            entry.quads = list.iter().map(|(_, m)| m.quad_count()).sum();
            let shown = if self.drawn.contains(&column) { Visibility::Inherited } else { Visibility::Hidden };
            for (pos, data) in list {
                let entity = commands
                    .spawn((
                        Mesh3d(meshes.add(crate::to_bevy_mesh(data))),
                        MeshMaterial3d(material.clone()),
                        far_transform(column, pos, origin),
                        FarMesh { column, pos },
                        NotShadowCaster,
                        shown,
                    ))
                    .id();
                entry.entities.push(entity);
            }
            entry.meshed = true;
        }
    }
}

/// Where a far chunk is drawn: its corner, and scaled up to its level.
fn far_transform(column: FarColumn, pos: ChunkPos, origin: I64Vec3) -> Transform {
    let corner = WorldPos::at_boxel(BoxelPos(pos.0 * column.span()));
    Transform::from_translation(coords::render_translation(corner, origin)).with_scale(Vec3::splat(column.step() as f32))
}

/// Meshes far columns, each on its own (nothing beside it read, so its sides get walls), spread
/// over `threads` threads.
pub(crate) fn mesh_columns(
    list: Vec<(FarColumn, u64, Vec<ChunkData>)>,
    blocks: &BlockRegistry,
    volumes: &Volumes,
    merge: bool,
    threads: usize,
) -> Vec<FarMeshed> {
    let ao = AoTuning::default();
    let one = |(column, version, chunks): (FarColumn, u64, Vec<ChunkData>)| {
        let mut grid = Grid::default();
        for chunk in &chunks {
            grid.import_chunk(chunk);
        }
        let positions: Vec<ChunkPos> = chunks.iter().map(|c| c.pos).collect();
        let meshed = crate::mesher::mesh_chunks_on(&grid, &positions, blocks, volumes, &ao, false, 1);
        let meshes = positions
            .into_iter()
            .zip(meshed)
            .filter(|(_, m)| !m.is_empty())
            .map(|(p, m)| (p, if merge { crate::greedy::merge_far(&m) } else { m }))
            .collect();
        FarMeshed { column, version, meshes }
    };
    let threads = threads.clamp(1, list.len().max(1));
    if threads == 1 {
        return list.into_iter().map(one).collect();
    }
    let mut parts: Vec<Vec<(FarColumn, u64, Vec<ChunkData>)>> = (0..threads).map(|_| Vec::new()).collect();
    for (i, item) in list.into_iter().enumerate() {
        parts[i % threads].push(item);
    }
    std::thread::scope(|s| {
        let handles: Vec<_> = parts.into_iter().map(|part| s.spawn(move || part.into_iter().map(one).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().expect("meshing a far column")).collect()
    })
}

/// How far the view reaches, if far terrain is on.
pub fn reach(view: &ViewTuning, switches: &ViewSwitches) -> Option<Reach> {
    let reach = Reach { detail: view.render_distance, far: view.far_distance, level_reach: view.level_reach };
    (switches.far_terrain && reach.has_far()).then_some(reach)
}

/// Makes the far material once the block material exists (the same, read plainly), and keeps
/// its flattening as tuned.
pub(crate) fn sync_far_material(
    chunks: Option<Res<ChunkMeshes>>,
    view: Res<ViewTuning>,
    mut far: ResMut<FarTerrain>,
    mut materials: ResMut<Assets<SolidMaterial>>,
) {
    let flatten = Vec4::new(view.far_flatten, 0.0, 0.0, 0.0);
    match &far.material {
        Some(handle) => {
            if materials.get(handle).is_some_and(|m| m.far != flatten)
                && let Some(m) = materials.get_mut(handle)
            {
                m.far = flatten;
            }
        }
        None => {
            let Some(chunks) = chunks else { return };
            let Some(mut material) = materials.get(&chunks.material).cloned() else { return };
            material.place = FAR_PLACE;
            material.far = flatten;
            far.material = Some(materials.add(material));
        }
    }
}

/// Picks which columns are drawn, from the camera, and shows and hides the far meshes to match.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pick_far(
    view: Res<ViewTuning>,
    switches: Res<ViewSwitches>,
    origin: Res<RenderOrigin>,
    camera: Single<&Transform, With<PlayerCamera>>,
    bg: Res<BackgroundMesher>,
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
    let mut terrains: Vec<usize> = far.columns.keys().map(|c| c.terrain).collect();
    terrains.push(0);
    terrains.sort_unstable();
    terrains.dedup();
    let ready = |c: FarColumn| {
        if c.level == 0 {
            bg.column_drawable(c.x, c.y)
        } else {
            far.columns.get(&c).is_some_and(|e| e.meshed)
        }
    };
    let mut drawn = HashSet::new();
    let mut ordinary = HashSet::new();
    for terrain in terrains {
        for c in reach.pick(terrain, eye, &ready) {
            if c.level == 0 {
                ordinary.insert((c.x, c.y));
            } else {
                drawn.insert(c);
            }
        }
    }
    far.ordinary = ordinary;
    if drawn != far.drawn {
        for (mesh, mut v) in &mut meshes {
            v.set_if_neq(if drawn.contains(&mesh.column) { Visibility::Inherited } else { Visibility::Hidden });
        }
        far.drawn = drawn;
    }
    let drawn: Vec<&Entry> = far.drawn.iter().filter_map(|c| far.columns.get(c)).collect();
    let triangles = drawn.iter().map(|e| e.quads * 2).sum();
    let chunks = drawn.iter().map(|e| e.entities.len()).sum();
    far.stats = FarViewStats { far: far.drawn.len(), ordinary: far.ordinary.len(), chunks, triangles };
}

/// Keeps far meshes where they belong as the render origin moves.
pub(crate) fn sync_far_transforms(origin: Res<RenderOrigin>, mut meshes: Query<(&FarMesh, &mut Transform)>) {
    if !origin.is_changed() {
        return;
    }
    for (mesh, mut transform) in &mut meshes {
        *transform = far_transform(mesh.column, mesh.pos, origin.cell);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn far_chunks_sit_at_their_level_scaled_up() {
        let column = FarColumn { terrain: 0, level: 2, x: 1, y: -1 };
        let t = far_transform(column, ChunkPos(I64Vec3::new(1, -1, 0)), I64Vec3::ZERO);
        assert_eq!(t.scale, Vec3::splat(4.0));
        // Its corner, 128 boxels east and south, in render space.
        assert_eq!(coords::from_render(t.translation), Vec3::new(128.0, -128.0, 0.0));
    }

    #[test]
    fn far_columns_are_meshed_with_walls_on_their_sides() {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(api::BlockDef::new("t:stone", true, [0; 3])).unwrap();
        let looks = sim::LookRegistry::default();
        let volumes = Volumes::build(&looks, &blocks);
        // A slab of stone 2 cells thick filling the chunk's floor.
        let mut chunk = ChunkData {
            pos: ChunkPos(I64Vec3::ZERO),
            lists: vec![Vec::new(), vec![sim::Occupant::new(stone)]],
            cells: sim::Layer::Uniform(0),
            light: sim::Layer::Uniform(sim::Light::SKY),
            records: Vec::new(),
        };
        for i in 0..api::CHUNK_VOLUME {
            if api::LocalPos::from_index(i).z < 2 {
                chunk.cells.set_at(i, 1);
            }
        }
        let column = FarColumn { terrain: 0, level: 1, x: 0, y: 0 };
        let out = mesh_columns(vec![(column, 7, vec![chunk])], &blocks, &volumes, true, 2);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].version, 7);
        let mesh = &out[0].meshes[0].1;
        // Top, bottom and the four sides, which nothing beside it covers.
        for normal in [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0]] {
            assert!(mesh.normals.contains(&normal), "a face facing {normal:?}");
        }
        // Merged, and split into fans only along the bottom, the border with the chunk below
        // (a fan counts as a quad per triangle pair): the four sides' other edges and the top
        // stay whole. Split along the sides too, it was 264.
        assert!(mesh.quad_count() < 140, "{} quads", mesh.quad_count());
    }
}

#[cfg(test)]
mod timing {
    use std::time::Instant;

    use super::*;
    use crate::world::testing::base_sim;

    /// Times meshing the far columns around the terrain's center, step by step. Run with
    /// `--release --ignored --nocapture`.
    #[test]
    #[ignore]
    fn time_far_meshing() {
        for level_reach in [2.0, 1.6, 1.4] {
            time_far_meshing_at(level_reach);
        }
    }

    fn time_far_meshing_at(level_reach: f32) {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        sim.teleport_player(WorldPos::at_boxel(base::TERRAIN_CENTER.offset(I64Vec3::new(0, 0, 120))));
        sim.set_reach(Reach { detail: 256.0, far: 2048.0, level_reach });
        sim.generate_terrain().unwrap();
        sim.far_all();
        let built = sim.take_far_loaded();
        let volumes = Volumes::build(sim.looks(), sim.blocks());
        let ao = AoTuning::default();
        let (mut mesh_s, mut merge_s, mut chunks, mut quads, mut merged) = (0.0f32, 0.0f32, 0, 0, 0);
        let mut by_level = [0usize; 8];
        let mut per_column: HashMap<FarColumn, usize> = HashMap::new();
        for b in &built {
            let mut grid = Grid::default();
            for c in &b.chunks {
                grid.import_chunk(c);
            }
            let positions: Vec<ChunkPos> = b.chunks.iter().map(|c| c.pos).collect();
            chunks += positions.len();
            let clock = Instant::now();
            let meshed = crate::mesher::mesh_chunks_on(&grid, &positions, sim.blocks(), &volumes, &ao, false, 1);
            mesh_s += clock.elapsed().as_secs_f32();
            let clock = Instant::now();
            for m in &meshed {
                quads += m.quad_count();
                let q = crate::greedy::merge_far(m).quad_count();
                merged += q;
                by_level[b.column.level as usize] += q;
                *per_column.entry(b.column).or_insert(0) += q;
            }
            merge_s += clock.elapsed().as_secs_f32();
        }
        let pos = sim.player().unwrap().pos();
        let picked = sim.reach().pick(0, pos, &|c| c.level == 0 || per_column.contains_key(&c));
        let drawn: usize = picked.iter().filter_map(|c| per_column.get(c)).sum();
        println!(
            "level reach {level_reach}: {} far columns, {chunks} chunks: meshing {mesh_s:.2} s, merging {merge_s:.2} s | {quads} quads, {merged} merged, by level {by_level:?} | {drawn} far quads drawn",
            built.len()
        );
    }
}
