//! Culled meshing: every face of every occupant's boxes becomes a quad, unless it lies on the
//! boxel's side against a full block. Where faces of two occupants of one boxel lie on the same
//! plane facing the same way, the part they share is drawn only from the older occupant (the
//! one placed first), so they never fight over which shows. Output is plain Z-up vertex data in chunk-local space;
//! turning it into a Bevy mesh happens elsewhere. Every vertex carries where it sits in its
//! block's look volume, so each face shows the part of the look it cuts through; flat-colored
//! blocks point at the white volume and carry their color in the vertices. Looks stay lined up
//! with the world unless the look turns with its blocks, like a log's.
//!
//! Ambient occlusion is baked into the vertex colors: each corner of a face darkens by how much
//! of the space in front of it, around the corner, block shapes fill (see [`AoTuning`]).

use std::cell::OnceCell;

use api::{BlockBox, CHUNK_SIZE, CHUNK_VOLUME, ChunkPos, I64Vec3, LocalPos, LookDef, Orientation, Tunable, Tunables};
use bevy::prelude::Resource;
use sim::{BlockRegistry, CellId, Chunk, Grid, Occupant};

use crate::volumes::Volumes;

/// Vertex data for one chunk, Z-up, relative to the chunk's minimum corner.
#[derive(Default, Debug)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Linear RGBA, multiplied with the texture.
    pub colors: Vec<[f32; 4]>,
    /// Where in the block's look, in texels, already half a texel inside the face so the face
    /// shows the layer just under it; then which volume.
    pub solid: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

impl MeshData {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn quad_count(&self) -> usize {
        self.indices.len() / 6
    }
}

struct Face {
    normal: [i32; 3],
    /// Corners of the unit cube, counterclockwise seen from outside.
    corners: [[i32; 3]; 4],
}

const FACES: [Face; 6] = [
    Face { normal: [1, 0, 0], corners: [[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]] },
    Face { normal: [-1, 0, 0], corners: [[0, 1, 0], [0, 0, 0], [0, 0, 1], [0, 1, 1]] },
    Face { normal: [0, 1, 0], corners: [[1, 1, 0], [0, 1, 0], [0, 1, 1], [1, 1, 1]] },
    Face { normal: [0, -1, 0], corners: [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]] },
    Face { normal: [0, 0, 1], corners: [[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]] },
    Face { normal: [0, 0, -1], corners: [[0, 1, 0], [1, 1, 0], [1, 0, 0], [0, 0, 0]] },
];

/// Fake lighting so faces are told apart without a lighting system, by which way they face
/// once turned.
fn shade(normal: [i32; 3]) -> f32 {
    match normal {
        [_, _, 1] => 1.0,
        [_, _, -1] => 0.5,
        [0, _, 0] => 0.65,
        _ => 0.8,
    }
}

/// Converts an sRGB byte to linear light.
fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Ambient occlusion: how dark inside corners and edges get. Live-tunable; changing it remeshes
/// every chunk.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct AoTuning {
    /// How much a corner darkens with all the space around it in front of the face filled: 0
    /// turns ambient occlusion off, 1 makes such a corner black. A floor corner against one
    /// full wall is half filled; where two walls meet, three quarters.
    pub strength: f32,
    /// How far from a corner shapes count, in sixteenths, rounded; 16 is a whole block.
    pub reach: f32,
}

impl AoTuning {
    pub const OFF: AoTuning = AoTuning { strength: 0.0, reach: 16.0 };

    fn reach(&self) -> i32 {
        (self.reach.round() as i32).clamp(1, BlockBox::SIZE as i32)
    }
}

impl Default for AoTuning {
    fn default() -> Self {
        Self { strength: 0.6, reach: 16.0 }
    }
}

impl Tunables for AoTuning {
    fn tunables(&self) -> Vec<Tunable> {
        vec![Tunable::new("strength", self.strength, 0.0, 1.0), Tunable::new("reach", self.reach, 1.0, 16.0)]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "strength" => self.strength = value,
            "reach" => self.reach = value,
            _ => return false,
        }
        true
    }
}

/// A box in sixteenths, relative to some boxel's minimum corner: min and max.
type Sixteenths = ([i32; 3], [i32; 3]);

/// The boxes filled by everything in the 27 boxels around (and including) the boxel at `at`,
/// in sixteenths relative to that boxel, turned as they stand in the world.
fn neighborhood<'a>(at: I64Vec3, occupants_at: impl Fn(I64Vec3) -> &'a [Occupant], blocks: &BlockRegistry) -> Vec<Sixteenths> {
    let size = BlockBox::SIZE as i32;
    let mut boxes = Vec::new();
    for d in (0..27).map(|i| [i % 3 - 1, i / 3 % 3 - 1, i / 9 - 1]) {
        for occupant in occupants_at(at + I64Vec3::new(d[0] as i64, d[1] as i64, d[2] as i64)) {
            let Some(def) = blocks.get(occupant.block) else {
                continue;
            };
            for &b in &def.shape {
                let b = occupant.orientation.apply_box(b);
                let shift = |p: [u8; 3]| [0, 1, 2].map(|i| p[i] as i32 + d[i] * size);
                boxes.push((shift(b.min), shift(b.max)));
            }
        }
    }
    boxes
}

/// How much of the space in front of a face around its corner `corner` is filled, from 0 to 1.
/// The space is four cubes `reach` on a side, all in front of the face (the way `normal`
/// points) and meeting at the corner: over the face, past each of the corner's two edges, and
/// past the corner. `boxes` and `corner` are in sixteenths relative to the same boxel.
fn occlusion(boxes: &[Sixteenths], corner: [i32; 3], normal: [i32; 3], reach: i32) -> f32 {
    let axis = normal.iter().position(|&n| n != 0).expect("a unit normal");
    // The cubes together: in front along the normal, either side along the other two axes.
    let (mut lo, mut hi) = (corner.map(|c| c - reach), corner.map(|c| c + reach));
    if normal[axis] > 0 { lo[axis] = corner[axis] } else { hi[axis] = corner[axis] }
    let mut filled = 0;
    for (min, max) in boxes {
        let (cut_min, cut_max) = ([0, 1, 2].map(|i| min[i].max(lo[i])), [0, 1, 2].map(|i| max[i].min(hi[i])));
        if (0..3).any(|i| cut_min[i] >= cut_max[i]) {
            continue;
        }
        filled += (0..3).map(|i| cut_max[i] - cut_min[i]).product::<i32>();
    }
    // Boxes of one boxel's occupants never overlap, but clamp in case.
    (filled as f32 / (4 * reach * reach * reach) as f32).min(1.0)
}

/// The occupants of the boxel at `local` in `chunk` at `pos`, read from the next chunk past
/// its edges.
fn occupants_at<'a>(grid: &'a Grid, chunk: &'a Chunk, pos: ChunkPos, local: I64Vec3) -> &'a [Occupant] {
    let inside = local.cmpge(I64Vec3::ZERO).all() && local.cmplt(I64Vec3::splat(CHUNK_SIZE)).all();
    if inside {
        grid.occupants(chunk.cells.get(LocalPos::new(local.x as u8, local.y as u8, local.z as u8)))
    } else {
        grid.cell(pos.origin().offset(local))
    }
}

/// Whether the chunk at `pos` and its six neighbors are each one full block all through, so
/// none of its faces can show. Most of a generated terrain's underground is.
fn buried(grid: &Grid, pos: ChunkPos, blocks: &BlockRegistry) -> bool {
    let solid = |p: ChunkPos| {
        let Some(id) = grid.chunk(p).and_then(|c| c.cells.uniform()) else {
            return false;
        };
        grid.occupants(id).iter().any(|o| blocks.get(o.block).is_some_and(|d| d.is_full()))
    };
    let sides = [I64Vec3::X, I64Vec3::NEG_X, I64Vec3::Y, I64Vec3::NEG_Y, I64Vec3::Z, I64Vec3::NEG_Z];
    solid(pos) && sides.iter().all(|&d| solid(ChunkPos(pos.0 + d)))
}

/// Meshes every chunk in `positions`, spread over every core. Comes back in the same order.
pub fn mesh_chunks(grid: &Grid, positions: &[ChunkPos], blocks: &BlockRegistry, volumes: &Volumes, ao: &AoTuning) -> Vec<MeshData> {
    if positions.len() < 8 {
        return positions.iter().map(|&p| mesh_chunk(grid, p, blocks, volumes, ao)).collect();
    }
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    // Small parts, handed out as threads free up, since chunks differ a lot in cost.
    let parts: Vec<&[ChunkPos]> = positions.chunks(16).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut done: Vec<(usize, Vec<MeshData>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads.min(parts.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(part) = parts.get(i) else {
                            break out;
                        };
                        out.push((i, part.iter().map(|&p| mesh_chunk(grid, p, blocks, volumes, ao)).collect()));
                    }
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("a meshing thread panicked")).collect()
    });
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().flat_map(|(_, meshes)| meshes).collect()
}

/// Meshes the chunk at `pos`. Neighbors are read through `grid`, so faces on chunk borders
/// are culled and occluded against the next chunk. Unloaded neighbors count as empty.
pub fn mesh_chunk(grid: &Grid, pos: ChunkPos, blocks: &BlockRegistry, volumes: &Volumes, ao: &AoTuning) -> MeshData {
    let mut mesh = MeshData::default();
    let Some(chunk) = grid.chunk(pos) else {
        return mesh;
    };
    if chunk.cells.uniform() == Some(CellId::EMPTY) || buried(grid, pos, blocks) {
        return mesh;
    }

    let near = |local: I64Vec3| occupants_at(grid, chunk, pos, local);
    let full = |local: I64Vec3| near(local).iter().any(|o| blocks.get(o.block).is_some_and(|def| def.is_full()));
    let reach = ao.reach();

    for index in 0..CHUNK_VOLUME {
        let local = LocalPos::from_index(index);
        let id = chunk.cells.get(local);
        if id.is_empty() {
            continue;
        }
        let at = local.to_ivec();
        let mut drawn = Vec::new();
        // Only gathered once a face of this boxel is drawn; most boxels are buried.
        let boxes = OnceCell::new();
        let light = |corner: [i32; 3], normal: [i32; 3]| -> f32 {
            if ao.strength <= 0.0 {
                return 1.0;
            }
            let boxes = boxes.get_or_init(|| neighborhood(at, near, blocks));
            1.0 - ao.strength * occlusion(boxes, corner, normal, reach)
        };
        for &occupant in grid.occupants(id) {
            let hidden = |n: [i32; 3]| full(at + I64Vec3::new(n[0] as i64, n[1] as i64, n[2] as i64));
            mesh_occupant(&mut mesh, occupant, at.as_vec3().to_array(), blocks, volumes, hidden, &light, &mut drawn);
        }
    }
    mesh
}

/// A face already drawn in a boxel, in sixteenths: which way it faces, where its plane is along
/// that axis, and the rectangle it covers on the two axes after it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawnFace {
    normal: [i32; 3],
    plane: u8,
    min: [u8; 2],
    max: [u8; 2],
}

/// What's left of rectangle `r` after cutting out `cut`: up to four rectangles.
fn subtract((min, max): ([u8; 2], [u8; 2]), cut: &DrawnFace) -> Vec<([u8; 2], [u8; 2])> {
    let overlaps = (0..2).all(|i| min[i] < cut.max[i] && cut.min[i] < max[i]);
    if !overlaps {
        return vec![(min, max)];
    }
    let mut pieces = Vec::new();
    if min[0] < cut.min[0] {
        pieces.push((min, [cut.min[0], max[1]]));
    }
    if cut.max[0] < max[0] {
        pieces.push(([cut.max[0], min[1]], max));
    }
    let (u0, u1) = (min[0].max(cut.min[0]), max[0].min(cut.max[0]));
    if min[1] < cut.min[1] {
        pieces.push(([u0, min[1]], [u1, cut.min[1]]));
    }
    if cut.max[1] < max[1] {
        pieces.push(([u0, cut.max[1]], [u1, max[1]]));
    }
    pieces
}

/// Adds the faces of one occupant whose boxel's minimum corner is at `at`. A face on the
/// boxel's side facing direction `n` is left out if `hidden(n)`. Parts of faces that lie on
/// faces in `drawn`, from older occupants of the same boxel, are left out too, and this
/// occupant's faces are added to `drawn`. Each corner's color is multiplied by
/// `light(corner, normal)`, the corner in sixteenths of the boxel as it stands in the world;
/// pass [`unlit`] where there are no neighbors.
#[allow(clippy::too_many_arguments)]
pub fn mesh_occupant(
    mesh: &mut MeshData,
    occupant: Occupant,
    at: [f32; 3],
    blocks: &BlockRegistry,
    volumes: &Volumes,
    hidden: impl Fn([i32; 3]) -> bool,
    light: &impl Fn([i32; 3], [i32; 3]) -> f32,
    drawn: &mut Vec<DrawnFace>,
) {
    let Some(def) = blocks.get(occupant.block) else {
        return;
    };
    let look = volumes.of(occupant.block);
    // A block with a look shows its look's own colors; a flat one tints the white volume.
    let base = if look.is_some() { [1.0; 3] } else { def.color.map(srgb_to_linear) };
    let volume = look.unwrap_or(Volumes::WHITE);
    // A look lined up with the world: mesh the turned boxes as if unturned, so positions in
    // the look are positions in the boxel as it stands in the world.
    let (turn, shape): (Orientation, Vec<BlockBox>) = if volumes.turns(occupant.block) {
        (occupant.orientation, def.shape.clone())
    } else {
        (Orientation::IDENTITY, def.shape.iter().map(|&b| occupant.orientation.apply_box(b)).collect())
    };
    let half = BlockBox::SIZE as i32 / 2;
    // A point in sixteenths, unturned, to sixteenths of the boxel turned about the center.
    let place = |p: [i32; 3]| turn.apply(p.map(|c| c - half)).map(|c| c + half);

    let mine_from = drawn.len();
    for b in &shape {
        let turned = turn.apply_box(*b);
        for face in &FACES {
            let normal = turn.apply(face.normal);
            let axis = normal.iter().position(|&n| n != 0).expect("a unit normal");
            let plane = if normal[axis] > 0 { turned.max[axis] } else { turned.min[axis] };
            let on_side = plane == if normal[axis] > 0 { BlockBox::SIZE } else { 0 };
            if on_side && hidden(normal) {
                continue;
            }
            let (a, c) = ((axis + 1) % 3, (axis + 2) % 3);
            let whole = ([turned.min[a], turned.min[c]], [turned.max[a], turned.max[c]]);
            let mut pieces = vec![whole];
            for cut in drawn[..mine_from].iter().filter(|d| d.normal == normal && d.plane == plane) {
                pieces = pieces.iter().flat_map(|&p| subtract(p, cut)).collect();
            }
            drawn.push(DrawnFace { normal, plane, min: whole.0, max: whole.1 });
            for (min, max) in pieces {
                // The piece as a box, turned back to how the block stands unturned.
                let mut piece = turned;
                (piece.min[a], piece.min[c], piece.max[a], piece.max[c]) = (min[0], min[1], max[0], max[1]);
                quad(mesh, face, &turn.inverse().apply_box(piece), normal, at, base, volume, &place, light);
            }
        }
    }
}

/// No ambient occlusion, for meshes drawn away from the world: the ghost, items, pictures.
pub fn unlit(_corner: [i32; 3], _normal: [i32; 3]) -> f32 {
    1.0
}

/// Adds one face of box `b` (in sixteenths, before turning) as a quad facing `normal` once
/// turned by `place`, drawn from look `volume`, each corner lit by `light`.
#[allow(clippy::too_many_arguments)]
fn quad(
    mesh: &mut MeshData,
    face: &Face,
    b: &BlockBox,
    normal: [i32; 3],
    at: [f32; 3],
    base: [f32; 3],
    volume: u32,
    place: &impl Fn([i32; 3]) -> [i32; 3],
    light: &impl Fn([i32; 3], [i32; 3]) -> f32,
) {
    let first = mesh.positions.len() as u32;
    let s = shade(normal);
    let texels = LookDef::SIZE as f32 / BlockBox::SIZE as f32;
    let mut lit = [0.0; 4];
    for (k, c) in face.corners.into_iter().enumerate() {
        // The corner of this box, in sixteenths, before turning.
        let p = [0, 1, 2].map(|i| if c[i] == 0 { b.min[i] as i32 } else { b.max[i] as i32 });
        // In the look, half a texel back from the face, into the block.
        let [x, y, z] = [0, 1, 2].map(|i| p[i] as f32 * texels - face.normal[i] as f32 * 0.5);
        let q = place(p);
        lit[k] = light(q, normal);
        let l = s * lit[k];
        mesh.positions.push([0, 1, 2].map(|i| at[i] + q[i] as f32 / BlockBox::SIZE as f32));
        mesh.normals.push(normal.map(|n| n as f32));
        mesh.colors.push([base[0] * l, base[1] * l, base[2] * l, 1.0]);
        mesh.solid.push([x, y, z, volume as f32]);
    }
    // Split along the diagonal whose ends are lit most alike. Splitting along the other one
    // would drag a lone dark (or lit) corner's color across the whole quad in a streak.
    if (lit[0] - lit[2]).abs() <= (lit[1] - lit[3]).abs() {
        mesh.indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    } else {
        mesh.indices.extend([first, first + 1, first + 3, first + 1, first + 2, first + 3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{BlockDef, BoxelPos, Orientation, Vec3};
    use sim::{BlockId, LookRegistry};

    fn setup() -> (Grid, BlockRegistry, BlockId) {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(BlockDef::new("test:stone", true, [128, 128, 128])).unwrap();
        (Grid::default(), blocks, stone)
    }

    /// Volumes for blocks without looks.
    fn flat(blocks: &BlockRegistry) -> Volumes {
        Volumes::build(&LookRegistry::default(), blocks)
    }

    /// Looks named `test:a`, `test:b` and so on, each one plain color. Those named in `turning`
    /// turn with their blocks.
    fn looks_turning(names: &[&str], turning: &[&str]) -> LookRegistry {
        let mut looks = LookRegistry::default();
        for name in names {
            let turns = turning.contains(name);
            looks.register(LookDef { name: (*name).into(), turns, rgba: vec![200; LookDef::VOLUME * 4] }).unwrap();
        }
        looks
    }

    fn looks(names: &[&str]) -> LookRegistry {
        looks_turning(names, &[])
    }

    /// Meshed without ambient occlusion, so colors are just the face shading.
    fn mesh_chunk(grid: &Grid, pos: ChunkPos, blocks: &BlockRegistry) -> MeshData {
        super::mesh_chunk(grid, pos, blocks, &flat(blocks), &AoTuning::OFF)
    }

    /// Fills the chunk at `c` with `id` all through, stored as one value as generated terrain is.
    fn fill_chunk(grid: &mut Grid, c: I64Vec3, id: BlockId) {
        let lists = vec![Vec::new(), vec![Occupant::new(id)]];
        grid.import_chunk(&sim::ChunkData { pos: ChunkPos(c), lists, cells: sim::Layer::Uniform(1) });
    }

    #[test]
    fn a_buried_chunk_has_no_faces_until_a_neighbor_opens() {
        let (mut grid, blocks, stone) = setup();
        let sides = [I64Vec3::X, I64Vec3::NEG_X, I64Vec3::Y, I64Vec3::NEG_Y, I64Vec3::Z, I64Vec3::NEG_Z];
        for c in std::iter::once(I64Vec3::ZERO).chain(sides) {
            fill_chunk(&mut grid, c, stone);
        }
        assert!(grid.chunk(ChunkPos(I64Vec3::ZERO)).unwrap().cells.uniform().is_some(), "stored as one value");
        assert!(buried(&grid, ChunkPos(I64Vec3::ZERO), &blocks));
        assert!(mesh_chunk(&grid, ChunkPos(I64Vec3::ZERO), &blocks).is_empty());
        // Hollow out one boxel of the neighbor above: the face under it shows.
        grid.set_block(BoxelPos::new(4, 4, 32), BlockId::EMPTY);
        assert!(!buried(&grid, ChunkPos(I64Vec3::ZERO), &blocks));
        assert_eq!(mesh_chunk(&grid, ChunkPos(I64Vec3::ZERO), &blocks).quad_count(), 1);
    }

    #[test]
    fn meshing_many_at_once_matches_one_at_a_time() {
        let (mut grid, blocks, stone) = setup();
        let mut positions = Vec::new();
        for i in 0..40i64 {
            let c = I64Vec3::new(i % 5, i / 5, 0);
            grid.set_block(ChunkPos(c).origin().offset(I64Vec3::new(i % 32, (i * 7) % 32, (i * 3) % 32)), stone);
            grid.set_block(ChunkPos(c).origin().offset(I64Vec3::new(31, 31, 0)), stone);
            positions.push(ChunkPos(c));
        }
        let volumes = flat(&blocks);
        let together = mesh_chunks(&grid, &positions, &blocks, &volumes, &AoTuning::default());
        for (pos, mesh) in positions.iter().zip(&together) {
            let alone = super::mesh_chunk(&grid, *pos, &blocks, &volumes, &AoTuning::default());
            assert_eq!((alone.positions.clone(), alone.indices.clone()), (mesh.positions.clone(), mesh.indices.clone()), "{:?}", pos.0);
        }
    }

    #[test]
    fn one_block_has_six_faces() {
        let (mut grid, blocks, stone) = setup();
        grid.set_block(BoxelPos::new(5, 5, 5), stone);
        let mesh = mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks);
        assert_eq!(mesh.quad_count(), 6);
        assert_eq!(mesh.positions.len(), 24);
    }

    #[test]
    fn touching_blocks_hide_shared_faces() {
        let (mut grid, blocks, stone) = setup();
        grid.set_block(BoxelPos::new(5, 5, 5), stone);
        grid.set_block(BoxelPos::new(6, 5, 5), stone);
        assert_eq!(mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks).quad_count(), 10);
    }

    #[test]
    fn faces_are_culled_across_chunk_borders() {
        let (mut grid, blocks, stone) = setup();
        grid.set_block(BoxelPos::new(31, 0, 0), stone);
        grid.set_block(BoxelPos::new(32, 0, 0), stone);
        assert_eq!(mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks).quad_count(), 5);
        assert_eq!(mesh_chunk(&grid, ChunkPos::new(1, 0, 0), &blocks).quad_count(), 5);

        // Negative side too.
        grid.set_block(BoxelPos::new(0, 0, -1), stone);
        grid.set_block(BoxelPos::new(0, 0, 0), stone);
        assert_eq!(mesh_chunk(&grid, ChunkPos::new(0, 0, -1), &blocks).quad_count(), 5);
    }

    #[test]
    fn empty_and_missing_chunks_have_no_geometry() {
        let (mut grid, blocks, stone) = setup();
        assert!(mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks).is_empty());
        grid.set_block(BoxelPos::new(1, 1, 1), stone);
        grid.set_block(BoxelPos::new(1, 1, 1), BlockId::EMPTY);
        assert!(mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks).is_empty());
    }

    #[test]
    fn a_full_chunk_only_shows_its_outside() {
        let (mut grid, blocks, stone) = setup();
        for i in 0..CHUNK_VOLUME {
            grid.set_block(ChunkPos::new(0, 0, 0).boxel(LocalPos::from_index(i)), stone);
        }
        let side = CHUNK_SIZE as usize;
        assert_eq!(mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks).quad_count(), 6 * side * side);
    }

    #[test]
    fn quads_wind_counterclockwise_from_outside() {
        let (mut grid, blocks, stone) = setup();
        grid.set_block(BoxelPos::new(0, 0, 0), stone);
        let mesh = mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks);
        for tri in mesh.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|i| Vec3::from_array(mesh.positions[tri[i] as usize]));
            let normal = Vec3::from_array(mesh.normals[tri[0] as usize]);
            assert!((b - a).cross(c - a).normalize().dot(normal) > 0.99);
        }
    }

    /// Each vertex's position in its look, back on the face (undoing the half-texel step
    /// inward), in texels.
    fn on_face(mesh: &MeshData, i: usize) -> [f32; 3] {
        [0, 1, 2].map(|k| mesh.solid[i][k] + mesh.normals[i][k] * 0.5)
    }

    #[test]
    fn faces_sample_their_look_just_inside_themselves() {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(BlockDef::new("test:stone", true, [0; 3]).with_look("test:a")).unwrap();
        let plain = blocks.register(BlockDef::new("test:plain", true, [255, 0, 0])).unwrap();
        let volumes = Volumes::build(&looks(&["test:a"]), &blocks);
        let mut grid = Grid::default();
        grid.set_block(BoxelPos::new(0, 0, 0), stone);
        grid.set_block(BoxelPos::new(5, 0, 0), plain);
        let mesh = super::mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks, &volumes, &AoTuning::OFF);
        assert_eq!(mesh.solid.len(), mesh.positions.len());
        for i in 0..mesh.positions.len() {
            let n = mesh.normals[i];
            let axis = n.iter().position(|&c| c != 0.0).unwrap();
            if mesh.positions[i][0] >= 5.0 {
                // The flat block: the white volume, tinted by its color.
                assert_eq!(mesh.solid[i][3], Volumes::WHITE as f32);
                assert!(mesh.colors[i][0] > 0.0 && mesh.colors[i][1] == 0.0);
                continue;
            }
            assert_eq!(mesh.solid[i][3], volumes.of(stone).unwrap() as f32);
            assert!(mesh.colors[i][0] == mesh.colors[i][1] && mesh.colors[i][1] == mesh.colors[i][2], "untinted");
            // The outermost layer: 15.5 on a face looking up an axis, 0.5 looking down it.
            let expected = if n[axis] > 0.0 { 15.5 } else { 0.5 };
            assert_eq!(mesh.solid[i][axis], expected, "normal {n:?}");
            // Lined up with the world: the look's position is the position in the boxel.
            let in_boxel = mesh.positions[i].map(|c| c * LookDef::SIZE as f32);
            assert_eq!(on_face(&mesh, i), in_boxel);
        }
    }

    const SLAB: BlockBox = BlockBox::new([0, 0, 0], [16, 16, 8]);

    #[test]
    fn a_slab_has_six_faces_and_half_height() {
        let (mut grid, mut blocks, stone) = setup();
        let slab = blocks.register(BlockDef::new("test:slab", true, [9; 3]).with_shape(vec![SLAB])).unwrap();
        grid.set_block(BoxelPos::new(5, 5, 5), slab);
        let mesh = mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks);
        assert_eq!(mesh.quad_count(), 6);
        let top = mesh.positions.iter().map(|p| p[2]).fold(f32::MIN, f32::max);
        assert_eq!(top, 5.5);

        // On stone, the slab's bottom is hidden but the stone's top isn't: the slab doesn't
        // cover all of it from every side.
        grid.set_block(BoxelPos::new(5, 5, 4), stone);
        assert_eq!(mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks).quad_count(), 5 + 6);
        // A full block beside it hides the slab's side, and keeps its own.
        grid.set_block(BoxelPos::new(6, 5, 5), stone);
        assert_eq!(mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks).quad_count(), 4 + 6 + 6);
    }

    #[test]
    fn a_top_slab_sits_in_the_upper_half_and_partial_faces_inside_stay() {
        let (mut grid, mut blocks, stone) = setup();
        let slab = blocks.register(BlockDef::new("test:slab", true, [9; 3]).with_shape(vec![SLAB])).unwrap();
        let top = Occupant::turned(slab, Orientation::quarter_turns(0, 2));
        grid.set_cell(BoxelPos::new(5, 5, 5), &[top]);
        let mesh = mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks);
        let zs: Vec<f32> = mesh.positions.iter().map(|p| p[2]).collect();
        assert!(zs.iter().all(|&z| (5.5..=6.0).contains(&z)), "{zs:?}");
        // Its bottom face, in the middle of the boxel, is never hidden by stone below, and the
        // stone's top shows through the gap.
        grid.set_block(BoxelPos::new(5, 5, 4), stone);
        assert_eq!(mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks).quad_count(), 6 + 6);
        let mesh = mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks);
        for tri in mesh.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|i| Vec3::from_array(mesh.positions[tri[i] as usize]));
            let normal = Vec3::from_array(mesh.normals[tri[0] as usize]);
            assert!((b - a).cross(c - a).normalize().dot(normal) > 0.99, "turned faces still wind outward");
        }
    }

    #[test]
    fn a_slab_shows_the_half_of_its_look_it_cuts_through() {
        let mut blocks = BlockRegistry::default();
        let slab = blocks.register(BlockDef::new("test:slab", true, [0; 3]).with_look("test:a").with_shape(vec![SLAB])).unwrap();
        let volumes = Volumes::build(&looks(&["test:a"]), &blocks);
        let mut grid = Grid::default();
        grid.set_block(BoxelPos::new(0, 0, 0), slab);
        let mesh = super::mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks, &volumes, &AoTuning::OFF);
        for i in 0..mesh.positions.len() {
            let z = mesh.solid[i][2];
            match mesh.normals[i][2] {
                // Its cut top reads the middle of the look, not the look's top.
                1.0 => assert_eq!(z, 7.5),
                -1.0 => assert_eq!(z, 0.5),
                _ => assert!((0.0..=8.0).contains(&z), "sides show the lower half, not {z}"),
            }
        }
    }

    #[test]
    fn two_slabs_in_one_boxel_read_one_look_without_a_seam() {
        let mut blocks = BlockRegistry::default();
        let slab = blocks.register(BlockDef::new("test:slab", true, [0; 3]).with_look("test:a").with_shape(vec![SLAB])).unwrap();
        let volumes = Volumes::build(&looks(&["test:a"]), &blocks);
        let mut grid = Grid::default();
        let top = Occupant::turned(slab, Orientation::quarter_turns(0, 2));
        grid.set_cell(BoxelPos::new(0, 0, 0), &[Occupant::new(slab), top]);
        let mesh = super::mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks, &volumes, &AoTuning::OFF);
        // On the east side, the two halves read one look: the same layer, heights running on.
        let mut east: Vec<_> =
            (0..mesh.positions.len()).filter(|&i| mesh.normals[i][0] == 1.0).map(|i| (mesh.solid[i][0], mesh.solid[i][2])).collect();
        east.sort_by(|a, b| a.1.total_cmp(&b.1));
        assert!(east.iter().all(|&(x, _)| x == 15.5));
        let heights: Vec<f32> = east.iter().map(|&(_, z)| z).collect();
        assert_eq!(heights, [0.0, 0.0, 8.0, 8.0, 8.0, 8.0, 16.0, 16.0]);
        for i in 0..mesh.positions.len() {
            assert_eq!(on_face(&mesh, i), mesh.positions[i].map(|c| c * LookDef::SIZE as f32), "lined up with the world");
        }
    }

    #[test]
    fn a_turned_log_reads_its_own_look_and_a_turned_slab_the_world() {
        let mut blocks = BlockRegistry::default();
        let log = blocks.register(BlockDef::new("test:log", true, [0; 3]).with_look("test:a")).unwrap();
        let slab = blocks.register(BlockDef::new("test:slab", true, [0; 3]).with_look("test:b").with_shape(vec![SLAB])).unwrap();
        let volumes = Volumes::build(&looks_turning(&["test:a", "test:b"], &["test:a"]), &blocks);
        let mut grid = Grid::default();
        // Lying along X; and a slab standing against the west side of its boxel.
        grid.set_cell(BoxelPos::new(0, 0, 0), &[Occupant::turned(log, Orientation::z_along(0))]);
        grid.set_cell(BoxelPos::new(4, 0, 0), &[Occupant::turned(slab, Orientation::z_along(0))]);
        let mesh = super::mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks, &volumes, &AoTuning::OFF);
        for i in 0..mesh.positions.len() {
            let n = mesh.normals[i];
            if mesh.positions[i][0] < 2.0 {
                // The log's ends, east and west, show its look's top and bottom: the rings.
                if n[0] != 0.0 {
                    assert!([0.5, 15.5].contains(&mesh.solid[i][2]), "an end reads z {:?}", mesh.solid[i]);
                } else {
                    // Its long sides show the look's sides: the bark.
                    let s = mesh.solid[i];
                    assert!([0.5, 15.5].contains(&s[0]) || [0.5, 15.5].contains(&s[1]), "a side reads {s:?}");
                }
            } else {
                // The slab's look stays lined up with the world: its cut east face reads x 7.5.
                let at = [mesh.positions[i][0] - 4.0, mesh.positions[i][1], mesh.positions[i][2]];
                assert_eq!(on_face(&mesh, i), at.map(|c| c * LookDef::SIZE as f32));
                if n[0] == 1.0 {
                    assert_eq!(mesh.solid[i][0], 7.5);
                }
            }
        }
    }

    /// The corners of each quad: its normal, and its minimum and maximum corner.
    fn quads(mesh: &MeshData) -> Vec<([f32; 3], [f32; 3], [f32; 3])> {
        (0..mesh.quad_count())
            .map(|q| {
                let corners = &mesh.positions[q * 4..q * 4 + 4];
                let min = [0, 1, 2].map(|i| corners.iter().map(|c| c[i]).fold(f32::MAX, f32::min));
                let max = [0, 1, 2].map(|i| corners.iter().map(|c| c[i]).fold(f32::MIN, f32::max));
                (mesh.normals[q * 4], min, max)
            })
            .collect()
    }

    /// Two slabs with different looks.
    fn two_slabs() -> (BlockRegistry, Volumes, BlockId, BlockId) {
        let mut blocks = BlockRegistry::default();
        let slab = |name: &str, look: &str| BlockDef::new(name, true, [0; 3]).with_look(look).with_shape(vec![SLAB]);
        let a = blocks.register(slab("test:slab_a", "test:a")).unwrap();
        let b = blocks.register(slab("test:slab_b", "test:b")).unwrap();
        let volumes = Volumes::build(&looks(&["test:a", "test:b"]), &blocks);
        (blocks, volumes, a, b)
    }

    /// A slab standing against the east side of its boxel.
    fn east(block: BlockId) -> Occupant {
        Occupant::turned(block, Orientation::from_axes([-1, 0, 0], [0, 1, 0]).unwrap())
    }

    #[test]
    fn coinciding_faces_in_a_boxel_never_overlap() {
        let (blocks, volumes, a, b) = two_slabs();
        for cell in [[Occupant::new(b), east(a)], [east(a), Occupant::new(b)]] {
            let mut grid = Grid::default();
            grid.set_cell(BoxelPos::new(3, 3, 3), &cell);
            let quads = quads(&super::mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks, &volumes, &AoTuning::OFF));
            for (i, (n1, min1, max1)) in quads.iter().enumerate() {
                for (n2, min2, max2) in &quads[i + 1..] {
                    let axis = n1.iter().position(|&c| c != 0.0).unwrap();
                    let plane = |min: &[f32; 3], max: &[f32; 3]| if n1[axis] > 0.0 { max[axis] } else { min[axis] };
                    if n1 != n2 || plane(min1, max1) != plane(min2, max2) {
                        continue;
                    }
                    let overlap = (0..3).filter(|&k| k != axis).all(|k| min1[k] < max2[k] && min2[k] < max1[k]);
                    assert!(!overlap, "quads overlap on one plane: {min1:?}-{max1:?} and {min2:?}-{max2:?}");
                }
            }
        }
    }

    #[test]
    fn the_oldest_occupant_shows_where_faces_coincide() {
        let (blocks, volumes, a, b) = two_slabs();
        let va = volumes.of(a).unwrap() as f32;
        // Which slab's look shows on the boxel's east side, by height: (z range, whether a's).
        let east_side = |cell: &[Occupant]| {
            let mut grid = Grid::default();
            grid.set_cell(BoxelPos::new(3, 3, 3), cell);
            let mesh = super::mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks, &volumes, &AoTuning::OFF);
            let mut sides: Vec<_> = quads(&mesh)
                .into_iter()
                .enumerate()
                .filter(|(_, (n, min, _))| n[0] == 1.0 && min[0] == 4.0)
                .map(|(q, (_, min, max))| ((min[2], max[2]), mesh.solid[q * 4][3] == va))
                .collect();
            sides.sort_by(|x, y| x.0.0.total_cmp(&y.0.0));
            sides
        };
        // The bottom slab first: its side shows below, the newer standing slab's above.
        assert_eq!(east_side(&[Occupant::new(b), east(a)]), [((3.0, 3.5), false), ((3.5, 4.0), true)]);
        // The standing slab first: it covers the whole side.
        assert_eq!(east_side(&[east(a), Occupant::new(b)]), [((3.0, 4.0), true)]);
        // Break the older one and the newer one's whole face comes back.
        assert_eq!(east_side(&[east(a)]), [((3.0, 4.0), true)]);
        assert_eq!(east_side(&[Occupant::new(b)]), [((3.0, 3.5), false)]);
    }

    #[test]
    fn shared_boxels_mesh_every_occupant() {
        let (mut grid, mut blocks, _) = setup();
        let slab = blocks.register(BlockDef::new("test:slab", true, [9; 3]).with_shape(vec![SLAB])).unwrap();
        let post = BlockBox::new([6, 6, 0], [10, 10, 16]);
        let fence = blocks.register(BlockDef::new("test:post", true, [9; 3]).with_shape(vec![post])).unwrap();
        grid.set_cell(BoxelPos::new(1, 1, 1), &[Occupant::new(slab), Occupant::new(fence)]);
        // The post's bottom lies on the older slab's bottom, so only the slab's is drawn.
        assert_eq!(mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks).quad_count(), 6 + 5);
    }

    /// White full blocks and white bottom slabs, so a top face's color is just its light.
    fn white() -> (Grid, BlockRegistry, BlockId, BlockId) {
        let mut blocks = BlockRegistry::default();
        let cube = blocks.register(BlockDef::new("test:white", true, [255; 3])).unwrap();
        let slab = blocks.register(BlockDef::new("test:white_slab", true, [255; 3]).with_shape(vec![SLAB])).unwrap();
        (Grid::default(), blocks, cube, slab)
    }

    /// A 4 by 4 floor at z = 0, from (0, 0) to (3, 3).
    fn floor(grid: &mut Grid, block: BlockId) {
        for (x, y) in (0..4).flat_map(|x| (0..4).map(move |y| (x, y))) {
            grid.set_block(BoxelPos::new(x, y, 0), block);
        }
    }

    fn lit(grid: &Grid, blocks: &BlockRegistry, chunk: ChunkPos) -> MeshData {
        super::mesh_chunk(grid, chunk, blocks, &flat(blocks), &AoTuning::default())
    }

    /// The light of every upward-facing vertex at `at`, chunk-local; they must all agree.
    fn light_up(mesh: &MeshData, at: [f32; 3]) -> f32 {
        let found: Vec<f32> = (0..mesh.positions.len())
            .filter(|&i| mesh.positions[i] == at && mesh.normals[i] == [0.0, 0.0, 1.0])
            .map(|i| mesh.colors[i][0])
            .collect();
        assert!(!found.is_empty(), "no upward vertex at {at:?}");
        assert!(found.iter().all(|&l| (l - found[0]).abs() < 1e-6), "vertices at {at:?} disagree: {found:?}");
        found[0]
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn a_lone_block_and_open_floor_are_not_darkened() {
        let (mut grid, blocks, cube, _) = white();
        grid.set_block(BoxelPos::new(5, 5, 5), cube);
        let chunk = ChunkPos::new(0, 0, 0);
        let off = super::mesh_chunk(&grid, chunk, &blocks, &flat(&blocks), &AoTuning::OFF);
        assert_eq!(lit(&grid, &blocks, chunk).colors, off.colors);
        let mut grid = Grid::default();
        floor(&mut grid, cube);
        assert!(close(light_up(&lit(&grid, &blocks, chunk), [2.0, 2.0, 1.0]), 1.0));
    }

    #[test]
    fn a_floor_corner_darkens_by_how_many_walls_meet_there() {
        let (mut grid, blocks, cube, _) = white();
        floor(&mut grid, cube);
        // A wall along x = 3 on the floor.
        for y in 0..4 {
            grid.set_block(BoxelPos::new(3, y, 1), cube);
        }
        let strength = AoTuning::default().strength;
        let mesh = lit(&grid, &blocks, ChunkPos::new(0, 0, 0));
        assert!(close(light_up(&mesh, [3.0, 1.0, 1.0]), 1.0 - strength * 0.5), "against one wall");
        assert!(close(light_up(&mesh, [2.0, 1.0, 1.0]), 1.0), "a block away from it");
        // A second wall along y = 3 makes an inside corner.
        for x in 0..3 {
            grid.set_block(BoxelPos::new(x, 3, 1), cube);
        }
        let mesh = lit(&grid, &blocks, ChunkPos::new(0, 0, 0));
        assert!(close(light_up(&mesh, [3.0, 3.0, 1.0]), 1.0 - strength * 0.75), "where two walls meet");
    }

    #[test]
    fn a_slab_beside_a_wall_darkens_less_than_a_full_block() {
        let (mut grid, blocks, cube, slab) = white();
        floor(&mut grid, cube);
        for y in 0..4 {
            grid.set_block(BoxelPos::new(3, y, 1), slab);
        }
        let strength = AoTuning::default().strength;
        let mesh = lit(&grid, &blocks, ChunkPos::new(0, 0, 0));
        // Half as tall, so half the space a full wall fills.
        assert!(close(light_up(&mesh, [3.0, 1.0, 1.0]), 1.0 - strength * 0.25));
        // The slabs' own tops, at half height, have nothing over them.
        assert!(close(light_up(&mesh, [4.0, 1.0, 1.5]), 1.0));
    }

    #[test]
    fn quads_split_along_the_diagonal_that_keeps_a_lone_dark_corner_to_itself() {
        let (mut grid, blocks, cube, _) = white();
        floor(&mut grid, cube);
        // A pillar touching the top of (1, 1, 0) only at its corner (2, 2).
        grid.set_block(BoxelPos::new(2, 2, 1), cube);
        let mesh = lit(&grid, &blocks, ChunkPos::new(0, 0, 0));
        let corners = [[1.0, 1.0, 1.0], [2.0, 1.0, 1.0], [2.0, 2.0, 1.0], [1.0, 2.0, 1.0]];
        let q = (0..mesh.quad_count())
            .find(|&q| corners.iter().all(|c| mesh.positions[q * 4..q * 4 + 4].contains(c)))
            .expect("the floor tile's top");
        let dark = (q * 4..q * 4 + 4).find(|&i| mesh.positions[i] == [2.0, 2.0, 1.0]).unwrap() as u32;
        assert!(mesh.colors[dark as usize][0] < 1.0);
        let triangles = &mesh.indices[q * 6..q * 6 + 6];
        assert_eq!(triangles.chunks(3).filter(|t| t.contains(&dark)).count(), 1);
    }

    #[test]
    fn darkening_reads_across_chunk_borders() {
        let (mut grid, blocks, cube, _) = white();
        // Floor at the east edge of chunk 0, wall in chunk 1.
        for y in 0..4 {
            grid.set_block(BoxelPos::new(31, y, 0), cube);
            grid.set_block(BoxelPos::new(32, y, 1), cube);
        }
        let mesh = lit(&grid, &blocks, ChunkPos::new(0, 0, 0));
        assert!(close(light_up(&mesh, [32.0, 1.0, 1.0]), 1.0 - AoTuning::default().strength * 0.5));
    }

    #[test]
    fn reach_narrows_how_far_darkening_spreads() {
        let (mut grid, blocks, cube, _) = white();
        floor(&mut grid, cube);
        for y in 0..4 {
            grid.set_block(BoxelPos::new(3, y, 1), cube);
        }
        // A half-block reach still sees the wall fill half the space at its base.
        let ao = AoTuning { reach: 8.0, ..AoTuning::default() };
        let mesh = super::mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks, &flat(&blocks), &ao);
        assert!(close(light_up(&mesh, [3.0, 1.0, 1.0]), 1.0 - ao.strength * 0.5));
        // Turning strength to 0 is exactly the unoccluded mesh.
        let off = AoTuning { strength: 0.0, reach: 8.0 };
        let mesh = super::mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks, &flat(&blocks), &off);
        let none = mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks);
        assert_eq!((mesh.colors, mesh.indices), (none.colors, none.indices));
    }
}
