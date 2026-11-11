//! Greedy meshing: neighboring faces of a chunk's mesh that would look the same drawn apart
//! become one bigger quad. Only whole boxel faces merge, on the same plane, facing the same
//! way, with the same look, and the same color (so ambient occlusion) and light at every
//! corner; the edges between them must be flat. A merged quad's outer edges hold one boxel's
//! pattern, which the shader repeats every boxel along it, so each outer edge must have the
//! same pattern all along. Its look position runs on across the boxels, and the shader wraps it
//! every boxel, so the look repeats instead of stretching.

use std::collections::HashMap;

use crate::mesher::{FLAT, MeshData, face_unit};

/// What a whole boxel face is, if quad `q` is one: which way it faces, its plane, which way
/// its edges 0 and 3 run (backwards for 3), and where it is along those, in boxels.
fn whole_face(mesh: &MeshData, q: usize) -> Option<(Key, [i32; 2])> {
    let p = &mesh.positions[4 * q..4 * q + 4];
    let step = |to: [f32; 3]| -> Option<[i8; 3]> {
        let d: [f32; 3] = std::array::from_fn(|k| to[k] - p[0][k]);
        let ones = d.iter().filter(|c| c.abs() == 1.0).count();
        let zeros = d.iter().filter(|c| **c == 0.0).count();
        (ones == 1 && zeros == 2).then(|| d.map(|c| c as i8))
    };
    let (u, v) = (step(p[1])?, step(p[3])?);
    let opposite: [f32; 3] = std::array::from_fn(|k| p[1][k] + v[k] as f32);
    if opposite != p[2] {
        return None;
    }
    let along = |d: [i8; 3]| {
        let k = d.iter().position(|&c| c != 0).expect("a unit step");
        let at = p[0][k];
        (at.fract() == 0.0).then_some(at as i32 * d[k] as i32)
    };
    let cell = [along(u)?, along(v)?];
    let normal = mesh.normals[4 * q].map(|c| c as i8);
    let axis = normal.iter().position(|&c| c != 0)?;
    Some((Key { normal, plane: p[0][axis].to_bits(), u, v }, cell))
}

/// Faces that could merge share a key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    normal: [i8; 3],
    plane: u32,
    u: [i8; 3],
    v: [i8; 3],
}

/// Whether quad `q` has the same color and light at all four corners.
fn uniform(mesh: &MeshData, q: usize) -> bool {
    let i = 4 * q;
    let light = |k: usize| mesh.light.get(i + k).copied();
    (1..4).all(|k| mesh.colors[i + k] == mesh.colors[i] && light(k) == light(0))
}

/// Whether quads `a` and `b` would look the same drawn in the same place: the same look,
/// color and light. Their edges are checked apart.
fn alike(mesh: &MeshData, a: usize, b: usize) -> bool {
    let (i, j) = (4 * a, 4 * b);
    mesh.colors[i] == mesh.colors[j]
        && mesh.light.get(i) == mesh.light.get(j)
        && mesh.solid[i..i + 4] == mesh.solid[j..j + 4]
}

/// Merges the quads of a chunk's mesh where it can (see the module). Quads that don't merge
/// are kept as they were.
pub fn merge(mesh: &MeshData) -> MeshData {
    let quads = mesh.quad_count();
    let mut groups: HashMap<Key, HashMap<[i32; 2], usize>> = HashMap::new();
    let mut grouped = vec![false; quads];
    for (q, grouped) in grouped.iter_mut().enumerate() {
        if !uniform(mesh, q) {
            continue;
        }
        if let Some((key, cell)) = whole_face(mesh, q) {
            let cells = groups.entry(key).or_default();
            if let std::collections::hash_map::Entry::Vacant(e) = cells.entry(cell) {
                e.insert(q);
                *grouped = true;
            }
        }
    }

    let mut out = MeshData::default();
    for q in (0..quads).filter(|&q| !grouped[q]) {
        copy(mesh, q, &mut out);
    }
    // Sorted so the result doesn't depend on hashing order.
    let mut keys: Vec<Key> = groups.keys().copied().collect();
    keys.sort_by_key(|k| (k.normal, k.plane, k.u, k.v));
    for key in keys {
        let cells = &groups[&key];
        let mut order: Vec<[i32; 2]> = cells.keys().copied().collect();
        order.sort_by_key(|&[u, v]| (v, u));
        let mut used = std::collections::HashSet::new();
        for start in order {
            if used.contains(&start) {
                continue;
            }
            let first = cells[&start];
            let (w, h) = grow(mesh, cells, &used, start, first);
            for du in 0..w {
                for dv in 0..h {
                    used.insert([start[0] + du, start[1] + dv]);
                }
            }
            if (w, h) == (1, 1) {
                copy(mesh, first, &mut out);
            } else {
                let at = |du: i32, dv: i32| cells[&[start[0] + du, start[1] + dv]];
                let edges = [
                    mesh.edges[4 * first][0],
                    mesh.edges[4 * at(w - 1, 0)][1],
                    mesh.edges[4 * at(0, h - 1)][2],
                    mesh.edges[4 * first][3],
                ];
                merged(mesh, first, w, h, edges, &mut out);
            }
        }
    }
    out
}

/// How far a rectangle of faces starting at `start` (quad `first`) grows: as wide as it can
/// along edge 0, then as tall as it can along edge 3 backwards.
fn grow(mesh: &MeshData, cells: &HashMap<[i32; 2], usize>, used: &std::collections::HashSet<[i32; 2]>, start: [i32; 2], first: usize) -> (i32, i32) {
    let at = |du: i32, dv: i32| {
        let cell = [start[0] + du, start[1] + dv];
        cells.get(&cell).copied().filter(|&q| !used.contains(&cell) && alike(mesh, first, q))
    };
    let edge = |q: usize, k: usize| mesh.edges[4 * q][k];
    let mut w = 1;
    while let Some(q) = at(w, 0) {
        let before = at(w - 1, 0).expect("in the row");
        if edge(before, 1) != FLAT || edge(q, 3) != FLAT || edge(q, 0) != edge(first, 0) || edge(q, 2) != edge(first, 2) {
            break;
        }
        w += 1;
    }
    let right = at(w - 1, 0).expect("in the row");
    let mut h = 1;
    'rows: while let Some(row) = (0..w).map(|du| at(du, h)).collect::<Option<Vec<usize>>>() {
        for (du, &q) in row.iter().enumerate() {
            let below = at(du as i32, h - 1).expect("in the rectangle");
            let inside_left = du > 0;
            let inside_right = du + 1 < row.len();
            let ok = edge(below, 2) == FLAT
                && edge(q, 0) == FLAT
                && edge(q, 2) == edge(row[0], 2)
                && if inside_left { edge(q, 3) == FLAT } else { edge(q, 3) == edge(first, 3) }
                && if inside_right { edge(q, 1) == FLAT } else { edge(q, 1) == edge(right, 1) };
            if !ok {
                break 'rows;
            }
        }
        h += 1;
    }
    (w, h)
}

/// Copies quad `q` of `from` to the end of `to`, as it was.
fn copy(from: &MeshData, q: usize, to: &mut MeshData) {
    let first = to.positions.len() as u32;
    let i = 4 * q;
    to.positions.extend_from_slice(&from.positions[i..i + 4]);
    to.normals.extend_from_slice(&from.normals[i..i + 4]);
    to.colors.extend_from_slice(&from.colors[i..i + 4]);
    to.solid.extend_from_slice(&from.solid[i..i + 4]);
    to.edges.extend_from_slice(&from.edges[i..i + 4]);
    to.face.extend_from_slice(&from.face[i..i + 4]);
    if !from.light.is_empty() {
        to.light.extend_from_slice(&from.light[i..i + 4]);
    }
    to.indices.extend(from.indices[6 * q..6 * q + 6].iter().map(|&k| k - i as u32 + first));
}

/// Adds quad `first` grown `w` boxels along its edge 0 and `h` along its edge 3 backwards,
/// with `edges` past its four sides.
fn merged(from: &MeshData, first: usize, w: i32, h: i32, edges: [u32; 4], to: &mut MeshData) {
    let base = to.positions.len() as u32;
    let i = 4 * first;
    let (w, h) = (w as f32, h as f32);
    let scale = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
    let out = |v: [[f32; 3]; 4]| -> [[f32; 3]; 4] {
        std::array::from_fn(|k| {
            let (a, b) = scale[k];
            std::array::from_fn(|c| v[0][c] + a * (v[1][c] - v[0][c]) + b * (v[3][c] - v[0][c]))
        })
    };
    let p = out([from.positions[i], from.positions[i + 1], from.positions[i + 2], from.positions[i + 3]]);
    let s3 = |v: [f32; 4]| [v[0], v[1], v[2]];
    let s = out([s3(from.solid[i]), s3(from.solid[i + 1]), s3(from.solid[i + 2]), s3(from.solid[i + 3])]);
    let volume = from.solid[i][3];
    // Texels along a boxel face.
    let size = api::BlockBox::SIZE as f32;
    let (width, height) = (w * size, h * size);
    for k in 0..4 {
        let (a, b) = scale[k];
        to.positions.push(p[k]);
        to.normals.push(from.normals[i]);
        to.colors.push(from.colors[i]);
        to.solid.push([s[k][0], s[k][1], s[k][2], volume]);
        to.edges.push(edges);
        to.face.push([face_unit(a * size), face_unit(b * size), face_unit(width), face_unit(height)]);
        if !from.light.is_empty() {
            to.light.push(from.light[i]);
        }
    }
    to.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::{AoTuning, FACE_SCALE, mesh_chunk};
    use crate::volumes::Volumes;
    use api::{BlockBox, BlockDef, BoxelPos, ChunkPos, LookDef, Orientation};
    use sim::{BlockId, BlockRegistry, Grid, LookRegistry, Occupant};

    /// A flat-colored stone, a slab of it, and a log whose look (plain, named `test:log`)
    /// turns with it.
    fn blocks() -> (BlockRegistry, Volumes, [BlockId; 3]) {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(BlockDef::new("test:stone", true, [128, 128, 128])).unwrap();
        let slab = blocks.register(BlockDef::new("test:slab", true, [128, 128, 128]).with_shape(vec![BlockBox::new([0, 0, 0], [16, 16, 8])])).unwrap();
        let log = blocks.register(BlockDef::new("test:log", true, [0; 3]).with_look("test:log")).unwrap();
        let mut looks = LookRegistry::default();
        looks.register(LookDef { name: "test:log".into(), turns: true, rgba: vec![200; LookDef::VOLUME * 4], sheen: None, glow: 0.0 }).unwrap();
        let volumes = Volumes::build(&looks, &blocks);
        (blocks, volumes, [stone, slab, log])
    }

    /// The chunk at the origin meshed unmerged and merged, its light worked out first.
    fn both(grid: &mut Grid, blocks: &BlockRegistry, volumes: &Volumes, ao: &AoTuning) -> (MeshData, MeshData) {
        grid.relight_all(blocks);
        let plain = mesh_chunk(grid, ChunkPos::new(0, 0, 0), blocks, volumes, ao);
        let merged = merge(&plain);
        (plain, merged)
    }

    fn floor(grid: &mut Grid, block: BlockId, size: i64) {
        for (x, y) in (0..size).flat_map(|x| (0..size).map(move |y| (x, y))) {
            grid.set_block(BoxelPos::new(x, y, 0), block);
        }
    }

    /// What a pixel of a quad shows (`s`, `t`) of the way along its edge 0 and up its edge 3,
    /// as the shader reads it: color (in 100,000ths; mixing differs in the last bits), light, the
    /// look's texel and volume, and for each edge, what's past it at this texel and how many
    /// texels away, where that's close enough to draw a line (edge lines are at most 8 wide).
    type Pixel = ([i64; 4], Option<[u8; 4]>, [i32; 4], [(u32, i32); 4]);

    fn pixel(mesh: &MeshData, q: usize, s: f32, t: f32) -> Pixel {
        let i = 4 * q;
        let mix = |v: [f32; 4]| v[0] * (1.0 - s) * (1.0 - t) + v[1] * s * (1.0 - t) + v[2] * s * t + v[3] * (1.0 - s) * t;
        let channel = |f: &dyn Fn(usize) -> f32| mix([f(i), f(i + 1), f(i + 2), f(i + 3)]);
        let color = std::array::from_fn(|c| (channel(&|k| mesh.colors[k][c]) * 1e5).round() as i64);
        let light = (!mesh.light.is_empty()).then(|| mesh.light[i]);
        let solid: [f32; 3] = std::array::from_fn(|c| channel(&|k| mesh.solid[k][c]));
        let size = LookDef::SIZE as i32;
        let texel = [0, 1, 2].map(|c| (solid[c].floor() as i32).rem_euclid(size));
        let look = [texel[0], texel[1], texel[2], mesh.solid[i][3] as i32];
        let face: [f32; 4] = std::array::from_fn(|c| channel(&|k| mesh.face[k][c] as f32 / u16::MAX as f32 * FACE_SCALE));
        let [u, v, w, h] = face;
        let edges = mesh.edges[i];
        let edge = |kinds: u32, along: f32, length: f32, distance: f32| {
            let texel = (along.floor().clamp(0.0, (length - 1.0).max(0.0)) as u32) % 16;
            let kind = (kinds >> (2 * texel)) & 3;
            let row = distance.max(0.0).floor() as i32;
            if kind == FLAT || row >= 8 { (0, 0) } else { (kind, row) }
        };
        let around = [edge(edges[0], u, w, v), edge(edges[1], v, h, w - u), edge(edges[2], w - u, w, h - v), edge(edges[3], h - v, h, u)];
        (color, light, look, around)
    }

    /// Where on quad `q` a point is, as (s, t) along its edges 0 and 3 backwards, if it's on it.
    fn on_quad(mesh: &MeshData, q: usize, point: [f32; 3]) -> Option<(f32, f32)> {
        let p = &mesh.positions[4 * q..4 * q + 4];
        let n = mesh.normals[4 * q];
        let axis = n.iter().position(|&c| c != 0.0).unwrap();
        if (point[axis] - p[0][axis]).abs() > 1e-4 {
            return None;
        }
        let d = |a: [f32; 3], b: [f32; 3]| -> [f32; 3] { std::array::from_fn(|k| a[k] - b[k]) };
        let dot = |a: [f32; 3], b: [f32; 3]| (0..3).map(|k| a[k] * b[k]).sum::<f32>();
        let (eu, ev, at) = (d(p[1], p[0]), d(p[3], p[0]), d(point, p[0]));
        let (s, t) = (dot(at, eu) / dot(eu, eu), dot(at, ev) / dot(ev, ev));
        ((0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&t)).then_some((s, t))
    }

    /// Every pixel of `plain` looks the same in `merged`: a few points inside every texel row
    /// and column near each corner and the middle of each quad, found on the merged quad
    /// facing the same way that holds them. And the two cover the same area.
    fn looks_the_same(plain: &MeshData, merged: &MeshData) {
        let area = |m: &MeshData| -> f64 {
            (0..m.quad_count())
                .map(|q| {
                    let p = &m.positions[4 * q..4 * q + 4];
                    let len = |a: [f32; 3], b: [f32; 3]| (0..3).map(|k| (a[k] - b[k]).abs()).sum::<f32>() as f64;
                    len(p[0], p[1]) * len(p[0], p[3])
                })
                .sum()
        };
        assert!((area(plain) - area(merged)).abs() < 1e-3, "{} vs {}", area(plain), area(merged));
        let points = [0.03, 0.5 / 16.0, 3.5 / 16.0, 7.5 / 16.0, 12.5 / 16.0, 15.5 / 16.0, 0.97];
        for q in 0..plain.quad_count() {
            let p = &plain.positions[4 * q..4 * q + 4];
            for &s in &points {
                for &t in &points {
                    let point: [f32; 3] = std::array::from_fn(|k| p[0][k] + s * (p[1][k] - p[0][k]) + t * (p[3][k] - p[0][k]));
                    let found = (0..merged.quad_count())
                        .filter(|&m| merged.normals[4 * m] == plain.normals[4 * q])
                        .find_map(|m| on_quad(merged, m, point).map(|(ms, mt)| (m, ms, mt)));
                    let (m, ms, mt) = found.unwrap_or_else(|| panic!("nothing merged covers {point:?}"));
                    let (mut a, mut b) = (pixel(plain, q, s, t), pixel(merged, m, ms, mt));
                    let colors_close = (0..4).all(|c| (a.0[c] - b.0[c]).abs() <= 2);
                    assert!(colors_close, "at {point:?}: color {:?} vs {:?}", a.0, b.0);
                    (a.0, b.0) = ([0; 4], [0; 4]);
                    assert_eq!(a, b, "at {point:?}, facing {:?}", plain.normals[4 * q]);
                }
            }
        }
    }

    #[test]
    fn a_floor_becomes_one_quad_a_side() {
        let (blocks, volumes, [stone, ..]) = blocks();
        let mut grid = Grid::default();
        floor(&mut grid, stone, 32);
        let (plain, merged) = both(&mut grid, &blocks, &volumes, &AoTuning::default());
        assert_eq!(plain.quad_count(), 2 * 32 * 32 + 4 * 32);
        assert_eq!(merged.quad_count(), 6, "top, bottom and four sides");
        looks_the_same(&plain, &merged);
    }

    #[test]
    fn darkened_corners_stay_apart() {
        let (blocks, volumes, [stone, ..]) = blocks();
        let mut grid = Grid::default();
        floor(&mut grid, stone, 8);
        grid.set_block(BoxelPos::new(4, 4, 1), stone);
        let (plain, merged) = both(&mut grid, &blocks, &volumes, &AoTuning::default());
        // Around the block on the floor, the eight floor faces touching it are darkened at some
        // corner, so they stay as they were; the rest of the floor still merges.
        let up = |m: &MeshData| (0..m.quad_count()).filter(|&q| m.normals[4 * q] == [0.0, 0.0, 1.0]).count();
        assert_eq!(up(&plain), 64 - 1 + 1);
        assert!(up(&merged) > 9 && up(&merged) < 20, "{} upward quads", up(&merged));
        looks_the_same(&plain, &merged);
        // Without AO the floor's top merges around the block in a few rectangles, the faces
        // with a crease toward the block kept apart from those without.
        let (plain, merged) = both(&mut grid, &blocks, &volumes, &AoTuning::OFF);
        assert!(up(&merged) <= 12, "{} upward quads", up(&merged));
        looks_the_same(&plain, &merged);
    }

    #[test]
    fn different_light_stays_apart() {
        let (blocks, volumes, [stone, ..]) = blocks();
        let mut grid = Grid::default();
        floor(&mut grid, stone, 8);
        // A roof over half the floor, two boxels up: the floor under it gets no sky.
        for (x, y) in (0..4).flat_map(|x| (0..8).map(move |y| (x, y))) {
            grid.set_block(BoxelPos::new(x, y, 3), stone);
        }
        let (plain, merged) = both(&mut grid, &blocks, &volumes, &AoTuning::OFF);
        let floor_tops: Vec<usize> = (0..merged.quad_count())
            .filter(|&q| merged.normals[4 * q] == [0.0, 0.0, 1.0] && merged.positions[4 * q][2] == 1.0)
            .collect();
        assert!(floor_tops.len() >= 2, "lit and shaded floor stay apart");
        looks_the_same(&plain, &merged);
    }

    #[test]
    fn creases_and_ridges_merge_only_where_they_repeat() {
        let (blocks, volumes, [stone, slab, _]) = blocks();
        let mut grid = Grid::default();
        // Steps: a floor, a row of blocks on it, a row of slabs beside those, a pit.
        floor(&mut grid, stone, 12);
        for y in 0..12 {
            grid.set_block(BoxelPos::new(3, y, 1), stone);
            grid.set_block(BoxelPos::new(4, y, 1), slab);
        }
        grid.set_block(BoxelPos::new(8, 8, 0), BlockId::EMPTY);
        for ao in [AoTuning::OFF, AoTuning::default()] {
            let (plain, merged) = both(&mut grid, &blocks, &volumes, &ao);
            assert!(merged.quad_count() < plain.quad_count() / 3, "{} of {}", merged.quad_count(), plain.quad_count());
            looks_the_same(&plain, &merged);
        }
    }

    #[test]
    fn turned_looks_repeat_along_a_merged_face() {
        let (blocks, volumes, [_, _, log]) = blocks();
        let mut grid = Grid::default();
        // A row of logs lying along x, and one standing beside them.
        for x in 0..6 {
            grid.set_cell(BoxelPos::new(x, 0, 0), &[Occupant::turned(log, Orientation::z_along(0))]);
        }
        grid.set_cell(BoxelPos::new(0, 2, 0), &[Occupant::new(log)]);
        let (plain, merged) = both(&mut grid, &blocks, &volumes, &AoTuning::OFF);
        assert_eq!(merged.quad_count(), 6 + 6, "the row is one long box");
        looks_the_same(&plain, &merged);
    }

    #[test]
    fn faces_inside_a_boxel_and_partial_faces_are_kept() {
        let (blocks, volumes, [stone, slab, _]) = blocks();
        let mut grid = Grid::default();
        // Slabs in a row: their tops (half way up) merge; their sides are half a face, kept.
        for x in 0..4 {
            grid.set_block(BoxelPos::new(x, 0, 0), slab);
        }
        grid.set_block(BoxelPos::new(0, 3, 0), stone);
        let (plain, merged) = both(&mut grid, &blocks, &volumes, &AoTuning::OFF);
        let tops = (0..merged.quad_count()).filter(|&q| merged.normals[4 * q] == [0.0, 0.0, 1.0] && merged.positions[4 * q][2] == 0.5).count();
        assert_eq!(tops, 1);
        let sides = (0..merged.quad_count()).filter(|&q| merged.normals[4 * q] == [0.0, -1.0, 0.0] && merged.positions[4 * q][1] == 0.0).count();
        assert_eq!(sides, 4, "half faces aren't merged");
        looks_the_same(&plain, &merged);
    }

    #[test]
    fn the_shader_knows_the_face_scale() {
        let shader = include_str!("solid.wgsl");
        assert!(shader.contains(&format!("const FACE_SCALE: f32 = {FACE_SCALE:.1};")), "solid.wgsl's FACE_SCALE should be {FACE_SCALE}");
    }
}
