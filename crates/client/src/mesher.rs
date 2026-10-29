//! Culled meshing: every face of every occupant's boxes becomes a quad, unless it lies on the
//! boxel's side against a full block. Where faces of two occupants of one boxel lie on the same
//! plane facing the same way, the part they share is drawn only from the older occupant (the
//! one placed first), so they never fight over which shows. Output is plain Z-up vertex data in chunk-local space;
//! turning it into a Bevy mesh happens elsewhere. Textured faces take UVs from the atlas,
//! cropped to the part of the boxel a box covers; flat-colored ones point at its white cell
//! and carry their color in the vertices. Textures stay lined up with the world unless the
//! block turns its textures with it, like a log.

use api::{BlockBox, CHUNK_SIZE, CHUNK_VOLUME, ChunkPos, I64Vec3, LocalPos, Orientation};
use sim::{BlockRegistry, CellId, Grid, Occupant};

use crate::atlas::Atlas;

/// Vertex data for one chunk, Z-up, relative to the chunk's minimum corner.
#[derive(Default, Debug)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Linear RGBA, multiplied with the texture.
    pub colors: Vec<[f32; 4]>,
    /// Atlas coordinates, 0 to 1 from the top left.
    pub uvs: Vec<[f32; 2]>,
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
    /// Corners of the unit cube, counterclockwise seen from outside. On the sides they run
    /// bottom left, bottom right, top right, top left, so textures stand upright.
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

/// Which of a block's textures a face uses, by its normal before any turn the textures follow:
/// top, bottom or side.
fn texture_slot(face: &Face) -> usize {
    match face.normal[2] {
        1 => 0,
        -1 => 1,
        _ => 2,
    }
}

/// Meshes the chunk at `pos`. Neighbors are read through `grid`, so faces on chunk borders
/// are culled against the next chunk. Unloaded neighbors count as empty.
pub fn mesh_chunk(grid: &Grid, pos: ChunkPos, blocks: &BlockRegistry, atlas: &Atlas) -> MeshData {
    let mut mesh = MeshData::default();
    let Some(chunk) = grid.chunk(pos) else {
        return mesh;
    };
    if chunk.cells.uniform() == Some(CellId::EMPTY) {
        return mesh;
    }

    let size = CHUNK_SIZE;
    let full = |local: I64Vec3| -> bool {
        let inside = local.cmpge(I64Vec3::ZERO).all() && local.cmplt(I64Vec3::splat(size)).all();
        let cell = if inside {
            grid.occupants(chunk.cells.get(LocalPos::new(local.x as u8, local.y as u8, local.z as u8)))
        } else {
            grid.cell(pos.origin().offset(local))
        };
        cell.iter().any(|o| blocks.get(o.block).is_some_and(|def| def.is_full()))
    };

    for index in 0..CHUNK_VOLUME {
        let local = LocalPos::from_index(index);
        let id = chunk.cells.get(local);
        if id.is_empty() {
            continue;
        }
        let at = local.to_ivec();
        let mut drawn = Vec::new();
        for &occupant in grid.occupants(id) {
            let hidden = |n: [i32; 3]| full(at + I64Vec3::new(n[0] as i64, n[1] as i64, n[2] as i64));
            mesh_occupant(&mut mesh, occupant, at.as_vec3().to_array(), blocks, atlas, hidden, &mut drawn);
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
/// occupant's faces are added to `drawn`.
pub fn mesh_occupant(
    mesh: &mut MeshData,
    occupant: Occupant,
    at: [f32; 3],
    blocks: &BlockRegistry,
    atlas: &Atlas,
    hidden: impl Fn([i32; 3]) -> bool,
    drawn: &mut Vec<DrawnFace>,
) {
    let Some(def) = blocks.get(occupant.block) else {
        return;
    };
    let textures = atlas.faces(occupant.block);
    // A textured block shows its texture's own colors; a flat one tints the white cell.
    let base = if textures.is_some() { [1.0; 3] } else { def.color.map(srgb_to_linear) };
    // Textures lined up with the world: mesh the turned boxes as if unturned, so each face
    // takes its texture and layout from the way it faces in the world.
    let (turn, shape): (Orientation, Vec<BlockBox>) = if def.turn_textures {
        (occupant.orientation, def.shape.clone())
    } else {
        (Orientation::IDENTITY, def.shape.iter().map(|&b| occupant.orientation.apply_box(b)).collect())
    };
    let half = BlockBox::SIZE as i32 / 2;
    // A point in sixteenths, unturned, to boxel space turned about the center.
    let place = |p: [i32; 3]| turn.apply(p.map(|c| c - half)).map(|c| (c + half) as f32 / BlockBox::SIZE as f32);

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
                let rect = textures.map_or(atlas.white(), |t| t[texture_slot(face)]);
                quad(mesh, face, &turn.inverse().apply_box(piece), normal, at, base, rect, &place);
            }
        }
    }
}

/// Adds one face of box `b` (in sixteenths, before turning) as a quad facing `normal` once
/// turned by `place`.
#[allow(clippy::too_many_arguments)]
fn quad(
    mesh: &mut MeshData,
    face: &Face,
    b: &BlockBox,
    normal: [i32; 3],
    at: [f32; 3],
    base: [f32; 3],
    rect: crate::atlas::Rect,
    place: &impl Fn([i32; 3]) -> [f32; 3],
) {
    let first = mesh.positions.len() as u32;
    let s = shade(normal);
    let color = [base[0] * s, base[1] * s, base[2] * s, 1.0];
    let [c0, c1, _, c3] = face.corners;
    let (u_dir, up_dir) = ([0, 1, 2].map(|i| c1[i] - c0[i]), [0, 1, 2].map(|i| c3[i] - c0[i]));
    for c in face.corners {
        // The corner of this box, in sixteenths, before turning.
        let p = [0, 1, 2].map(|i| if c[i] == 0 { b.min[i] as i32 } else { b.max[i] as i32 });
        let along = |dir: [i32; 3]| -> f32 {
            let d: i32 = (0..3).map(|i| (p[i] - c0[i] * BlockBox::SIZE as i32) * dir[i]).sum();
            d as f32 / BlockBox::SIZE as f32
        };
        let (u, up) = (along(u_dir), along(up_dir));
        let uv = [rect.min[0] + u * (rect.max[0] - rect.min[0]), rect.max[1] - up * (rect.max[1] - rect.min[1])];
        let q = place(p);
        mesh.positions.push([at[0] + q[0], at[1] + q[1], at[2] + q[2]]);
        mesh.normals.push(normal.map(|n| n as f32));
        mesh.colors.push(color);
        mesh.uvs.push(uv);
    }
    mesh.indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{BlockDef, BoxelPos, FaceTextures, Orientation, TextureDef, Vec3};
    use crate::atlas::Rect;
    use sim::{BlockId, TextureRegistry};

    fn setup() -> (Grid, BlockRegistry, BlockId) {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(BlockDef::new("test:stone", true, [128, 128, 128])).unwrap();
        (Grid::default(), blocks, stone)
    }

    /// An atlas for blocks without textures.
    fn flat(blocks: &BlockRegistry) -> Atlas {
        Atlas::build(&TextureRegistry::default(), blocks)
    }

    fn mesh_chunk(grid: &Grid, pos: ChunkPos, blocks: &BlockRegistry) -> MeshData {
        super::mesh_chunk(grid, pos, blocks, &flat(blocks))
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

    #[test]
    fn faces_use_their_textures_upright() {
        let mut textures = TextureRegistry::default();
        for name in ["test:top", "test:bottom", "test:side"] {
            textures.register(TextureDef { name: name.into(), width: 1, height: 1, rgba: vec![200; 4] }).unwrap();
        }
        let mut blocks = BlockRegistry::default();
        let faces = FaceTextures { top: "test:top".into(), bottom: "test:bottom".into(), side: "test:side".into() };
        let grass = blocks.register(BlockDef::new("test:grass", true, [0, 0, 0]).with_textures(faces)).unwrap();
        let plain = blocks.register(BlockDef::new("test:plain", true, [255, 0, 0])).unwrap();
        let atlas = Atlas::build(&textures, &blocks);
        let mut grid = Grid::default();
        grid.set_block(BoxelPos::new(0, 0, 0), grass);
        grid.set_block(BoxelPos::new(5, 0, 0), plain);
        let mesh = super::mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks, &atlas);
        assert_eq!(mesh.uvs.len(), mesh.positions.len());

        let [top, bottom, side] = *atlas.faces(grass).unwrap();
        for quad in 0..mesh.quad_count() {
            let corners = quad * 4..quad * 4 + 4;
            let normal = mesh.normals[quad * 4];
            let uvs = &mesh.uvs[corners.clone()];
            if mesh.positions[quad * 4][0] >= 5.0 {
                // The flat block: the white cell, tinted by its color.
                assert!(uvs.iter().all(|uv| within(uv, atlas.white())));
                assert!(mesh.colors[corners].iter().all(|c| c[0] > 0.0 && c[1] == 0.0));
                continue;
            }
            let expected = match normal[2] {
                1.0 => top,
                -1.0 => bottom,
                _ => side,
            };
            assert!(uvs.iter().all(|uv| within(uv, expected)), "normal {normal:?}");
            assert!(mesh.colors[corners.clone()].iter().all(|c| c[0] == c[1] && c[1] == c[2]), "untinted");
            if normal[2] == 0.0 {
                // Upright: the corners higher up get the texture's top row (smaller v).
                for (i, j) in [(0, 3), (1, 2)] {
                    let (low, high) = (corners.start + i, corners.start + j);
                    assert!(mesh.positions[high][2] > mesh.positions[low][2]);
                    assert!(mesh.uvs[high][1] < mesh.uvs[low][1]);
                }
            }
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
    fn side_textures_are_cropped_to_the_box() {
        let mut textures = TextureRegistry::default();
        textures.register(TextureDef { name: "test:t".into(), width: 1, height: 1, rgba: vec![200; 4] }).unwrap();
        let mut blocks = BlockRegistry::default();
        let def = BlockDef::new("test:slab", true, [0; 3]).with_textures(FaceTextures::all("test:t")).with_shape(vec![SLAB]);
        let slab = blocks.register(def).unwrap();
        let atlas = Atlas::build(&textures, &blocks);
        let mut grid = Grid::default();
        grid.set_block(BoxelPos::new(0, 0, 0), slab);
        let mesh = super::mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks, &atlas);
        let side = atlas.faces(slab).unwrap()[2];
        let mid = (side.min[1] + side.max[1]) / 2.0;
        for quad in 0..mesh.quad_count() {
            if mesh.normals[quad * 4][2] != 0.0 {
                continue;
            }
            // A bottom slab's sides show the texture's lower half.
            for i in quad * 4..quad * 4 + 4 {
                let v = mesh.uvs[i][1];
                let expected = if mesh.positions[i][2] > 0.0 { mid } else { side.max[1] };
                assert!((v - expected).abs() < 1e-6, "v {v}, expected {expected}");
            }
        }
    }

    #[test]
    fn a_log_on_its_side_shows_its_ends_east_and_west() {
        let mut textures = TextureRegistry::default();
        for name in ["test:rings", "test:bark"] {
            textures.register(TextureDef { name: name.into(), width: 1, height: 1, rgba: vec![200; 4] }).unwrap();
        }
        let mut blocks = BlockRegistry::default();
        let faces = FaceTextures { top: "test:rings".into(), bottom: "test:rings".into(), side: "test:bark".into() };
        let def = BlockDef::new("test:log", true, [0; 3]).with_textures(faces).with_turn_textures(true);
        let log = blocks.register(def).unwrap();
        let atlas = Atlas::build(&textures, &blocks);
        let mut grid = Grid::default();
        grid.set_cell(BoxelPos::new(0, 0, 0), &[Occupant::turned(log, Orientation::z_along(0))]);
        let mesh = super::mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks, &atlas);
        assert_eq!(mesh.quad_count(), 6);
        let [rings, _, bark] = *atlas.faces(log).unwrap();
        for quad in 0..6 {
            let expected = if mesh.normals[quad * 4][0] != 0.0 { rings } else { bark };
            assert!(mesh.uvs[quad * 4..quad * 4 + 4].iter().all(|uv| within(uv, expected)));
        }
    }

    #[test]
    fn world_lined_textures_stay_upright_on_a_turned_block() {
        let mut textures = TextureRegistry::default();
        for name in ["test:top", "test:side"] {
            textures.register(TextureDef { name: name.into(), width: 1, height: 1, rgba: vec![200; 4] }).unwrap();
        }
        let mut blocks = BlockRegistry::default();
        let faces = FaceTextures { top: "test:top".into(), bottom: "test:top".into(), side: "test:side".into() };
        let def = BlockDef::new("test:slab", true, [0; 3]).with_textures(faces).with_shape(vec![SLAB]);
        let slab = blocks.register(def).unwrap();
        let atlas = Atlas::build(&textures, &blocks);
        let mut grid = Grid::default();
        // A slab standing against the west side of its boxel.
        grid.set_cell(BoxelPos::new(0, 0, 0), &[Occupant::turned(slab, Orientation::z_along(0))]);
        let mesh = super::mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks, &atlas);
        let [top, _, side] = *atlas.faces(slab).unwrap();
        for quad in 0..mesh.quad_count() {
            let corners = quad * 4..quad * 4 + 4;
            let up = mesh.normals[quad * 4][2] != 0.0;
            let expected = if up { top } else { side };
            assert!(mesh.uvs[corners.clone()].iter().all(|uv| within(uv, expected)));
            if !up {
                // Upright: higher corners get smaller v.
                for (i, j) in [(0, 3), (1, 2)] {
                    let (low, high) = (corners.start + i, corners.start + j);
                    assert!(mesh.positions[high][2] > mesh.positions[low][2]);
                    assert!(mesh.uvs[high][1] < mesh.uvs[low][1]);
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

    /// Two different textured slabs, and a mesh of one boxel holding `cell`.
    fn two_slabs() -> (BlockRegistry, Atlas, BlockId, BlockId) {
        let mut textures = TextureRegistry::default();
        for name in ["test:a", "test:b"] {
            textures.register(TextureDef { name: name.into(), width: 1, height: 1, rgba: vec![200; 4] }).unwrap();
        }
        let mut blocks = BlockRegistry::default();
        let slab = |name: &str, texture: &str| {
            BlockDef::new(name, true, [0; 3]).with_textures(FaceTextures::all(texture)).with_shape(vec![SLAB])
        };
        let a = blocks.register(slab("test:slab_a", "test:a")).unwrap();
        let b = blocks.register(slab("test:slab_b", "test:b")).unwrap();
        let atlas = Atlas::build(&textures, &blocks);
        (blocks, atlas, a, b)
    }

    /// A slab standing against the east side of its boxel.
    fn east(block: BlockId) -> Occupant {
        Occupant::turned(block, Orientation::from_axes([-1, 0, 0], [0, 1, 0]).unwrap())
    }

    #[test]
    fn coinciding_faces_in_a_boxel_never_overlap() {
        let (blocks, atlas, a, b) = two_slabs();
        for cell in [[Occupant::new(b), east(a)], [east(a), Occupant::new(b)]] {
            let mut grid = Grid::default();
            grid.set_cell(BoxelPos::new(3, 3, 3), &cell);
            let quads = quads(&super::mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks, &atlas));
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
        let (blocks, atlas, a, b) = two_slabs();
        let ta = atlas.faces(a).unwrap()[2];
        // Which texture shows on the boxel's east side, by height: (z range, texture).
        let east_side = |cell: &[Occupant]| {
            let mut grid = Grid::default();
            grid.set_cell(BoxelPos::new(3, 3, 3), cell);
            let mesh = super::mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks, &atlas);
            let mut sides: Vec<_> = quads(&mesh)
                .into_iter()
                .enumerate()
                .filter(|(_, (n, min, _))| n[0] == 1.0 && min[0] == 4.0)
                .map(|(q, (_, min, max))| ((min[2], max[2]), within(&mesh.uvs[q * 4], ta)))
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

    fn within(uv: &[f32; 2], r: Rect) -> bool {
        (r.min[0]..=r.max[0]).contains(&uv[0]) && (r.min[1]..=r.max[1]).contains(&uv[1])
    }
}
