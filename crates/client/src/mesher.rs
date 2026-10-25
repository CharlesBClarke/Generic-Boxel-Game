//! Culled meshing: every block face next to an empty boxel becomes one quad. Output is plain
//! Z-up vertex data in chunk-local space; turning it into a Bevy mesh happens elsewhere.

use api::{CHUNK_SIZE, CHUNK_VOLUME, ChunkPos, I64Vec3, LocalPos};
use sim::{BlockId, BlockRegistry, Grid};

/// Vertex data for one chunk, Z-up, relative to the chunk's minimum corner.
#[derive(Default, Debug)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Linear RGBA.
    pub colors: Vec<[f32; 4]>,
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
    normal: [i64; 3],
    /// Corners of the unit cube, counterclockwise seen from outside.
    corners: [[f32; 3]; 4],
    /// Fake lighting so faces are told apart without a lighting system.
    shade: f32,
}

const FACES: [Face; 6] = [
    Face { normal: [1, 0, 0], corners: [[1., 0., 0.], [1., 1., 0.], [1., 1., 1.], [1., 0., 1.]], shade: 0.8 },
    Face { normal: [-1, 0, 0], corners: [[0., 1., 0.], [0., 0., 0.], [0., 0., 1.], [0., 1., 1.]], shade: 0.8 },
    Face { normal: [0, 1, 0], corners: [[1., 1., 0.], [0., 1., 0.], [0., 1., 1.], [1., 1., 1.]], shade: 0.65 },
    Face { normal: [0, -1, 0], corners: [[0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 1.]], shade: 0.65 },
    Face { normal: [0, 0, 1], corners: [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]], shade: 1.0 },
    Face { normal: [0, 0, -1], corners: [[0., 1., 0.], [1., 1., 0.], [1., 0., 0.], [0., 0., 0.]], shade: 0.5 },
];

/// Converts an sRGB byte to linear light.
fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Meshes the chunk at `pos`. Neighbors are read through `grid`, so faces on chunk borders
/// are culled against the next chunk. Unloaded neighbors count as empty.
pub fn mesh_chunk(grid: &Grid, pos: ChunkPos, blocks: &BlockRegistry) -> MeshData {
    let mut mesh = MeshData::default();
    let Some(chunk) = grid.chunk(pos) else {
        return mesh;
    };
    if chunk.blocks.uniform() == Some(BlockId::EMPTY) {
        return mesh;
    }

    let size = CHUNK_SIZE;
    let occupied = |local: I64Vec3| -> bool {
        let inside = local.cmpge(I64Vec3::ZERO).all() && local.cmplt(I64Vec3::splat(size)).all();
        let id = if inside {
            chunk.blocks.get(LocalPos::new(local.x as u8, local.y as u8, local.z as u8))
        } else {
            grid.block(pos.origin().offset(local))
        };
        !id.is_empty()
    };

    for index in 0..CHUNK_VOLUME {
        let local = LocalPos::from_index(index);
        let id = chunk.blocks.get(local);
        let Some(def) = blocks.get(id) else {
            continue;
        };
        let base = def.color.map(srgb_to_linear);
        let at = local.to_ivec();
        for face in &FACES {
            if occupied(at + I64Vec3::from_array(face.normal)) {
                continue;
            }
            let first = mesh.positions.len() as u32;
            let color = [base[0] * face.shade, base[1] * face.shade, base[2] * face.shade, 1.0];
            let normal = face.normal.map(|n| n as f32);
            for c in face.corners {
                mesh.positions.push([at.x as f32 + c[0], at.y as f32 + c[1], at.z as f32 + c[2]]);
                mesh.normals.push(normal);
                mesh.colors.push(color);
            }
            mesh.indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{BlockDef, BoxelPos, Vec3};

    fn setup() -> (Grid, BlockRegistry, BlockId) {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(BlockDef::new("test:stone", true, [128, 128, 128])).unwrap();
        (Grid::default(), blocks, stone)
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
}
