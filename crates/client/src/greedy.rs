//! Greedy meshing: neighboring faces of a chunk's mesh that would look the same drawn apart
//! become one bigger quad. Only whole boxel faces merge, on the same plane, facing the same
//! way, with the same look, and the same color and light at every corner; the edges between
//! them must be flat. A merged quad's outer edges hold one boxel's pattern, which the shader
//! repeats every boxel along it, so each outer edge must have the same pattern all along. Its
//! look position runs on across the boxels, and the shader wraps it every boxel, so the look
//! repeats instead of stretching.
//!
//! Ambient occlusion doesn't stop a merge: the shader works it out for each boxel of the quad
//! from what's in front of the quad (see [`crate::mesher::lattice_fill`]). So the faces must
//! have the same space over them (usually empty), and each outer edge the same space past it all
//! along; past each corner it's whatever the face at that corner has.

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
/// color and light, and the same space over them. Their edges are checked apart.
fn alike(mesh: &MeshData, a: usize, b: usize) -> bool {
    let (i, j) = (4 * a, 4 * b);
    let over = |q: usize| mesh.ao.get(q).map(|w| w.map(|c| c & 0xff));
    mesh.colors[i] == mesh.colors[j]
        && mesh.light.get(i) == mesh.light.get(j)
        && mesh.solid[i..i + 4] == mesh.solid[j..j + 4]
        && over(i) == over(j)
}

/// How full the space past edge `k` of quad `q` is at the edge's two ends, as
/// [`MeshData::ao`] holds it: the cube past it at corner `k`, and at corner `k + 1`.
fn past(mesh: &MeshData, q: usize, k: usize) -> Option<(u32, u32)> {
    mesh.ao.get(4 * q).map(|w| (w[k] >> 8 & 0xff, w[(k + 1) % 4] >> 16 & 0xff))
}

/// Merges the quads of a chunk's mesh where it can (see the module), then closes the seams
/// that leaves (see [`close_t_junctions`]).
pub fn merge(mesh: &MeshData) -> MeshData {
    close_t_junctions(&merge_quads(mesh))
}

/// [`merge`] for a far chunk (see [`crate::far`]): the same, as far chunks of its level may be
/// beside it every way.
pub fn merge_far(mesh: &MeshData) -> MeshData {
    merge(mesh)
}

/// Closes T-junctions: where a corner of one quad sits partway along another's edge, the GPU
/// rounds the two sides of the seam apart and leaves pixel gaps, seen as sparkles. A quad
/// more than a boxel long with such corners on its edges becomes a fan of triangles from its
/// middle around its outline, its edges split at those corners, so both sides share them.
///
/// Corners are looked for a whole boxel apart along each edge, from its start, which finds
/// every whole face's and every merged quad's. On a chunk's border the next chunk's corners
/// aren't known here, so an edge there is split at every boxel. Corners partway through a
/// boxel along an edge (a turned slab's side, say) aren't looked for.
///
/// A fan keeps two triangles a quad in [`MeshData::quad_count`]'s count, and the quad's own
/// values on every vertex: a merged quad's values change evenly across it, so they come out
/// the same at every point.
pub fn close_t_junctions(mesh: &MeshData) -> MeshData {
    close_t_junctions_on(mesh, [true; 3])
}

/// [`close_t_junctions`] for a chunk with neighbors only across its borders on the axes
/// `borders` marks: edges on its other borders split only where its own corners are.
pub fn close_t_junctions_on(mesh: &MeshData, borders: [bool; 3]) -> MeshData {
    let key = |p: [f32; 3]| p.map(f32::to_bits);
    let corners: std::collections::HashSet<[u32; 3]> = mesh.positions.iter().map(|&p| key(p)).collect();
    let border = api::CHUNK_SIZE as f32;
    let mut out = MeshData::default();
    for q in 0..mesh.quad_count() {
        let p = &mesh.positions[4 * q..4 * q + 4];
        // Along each edge, the boxels from its start where it splits.
        let splits: [Vec<i32>; 4] = std::array::from_fn(|k| {
            let (a, b) = (p[k], p[(k + 1) % 4]);
            let d: [f32; 3] = std::array::from_fn(|c| b[c] - a[c]);
            if d.iter().filter(|&&c| c != 0.0).count() != 1 {
                return Vec::new();
            }
            let length = d.iter().map(|c| c.abs()).sum::<f32>();
            let unit = d.map(|c| c / length);
            let on_border = (0..3).any(|c| borders[c] && d[c] == 0.0 && (a[c] == 0.0 || a[c] == border));
            let steps = length.ceil() as i32;
            (1..steps)
                .filter(|&i| on_border || corners.contains(&key(std::array::from_fn(|c| a[c] + unit[c] * i as f32))))
                .collect()
        });
        if splits.iter().all(Vec::is_empty) {
            copy(mesh, q, &mut out);
        } else {
            fan(mesh, q, &splits, &mut out);
        }
    }
    out
}

/// Adds quad `q` of `from` as a fan of triangles from its middle, its edges split `splits`
/// boxels from each one's start (see [`close_t_junctions`]).
fn fan(from: &MeshData, q: usize, splits: &[Vec<i32>; 4], to: &mut MeshData) {
    let i = 4 * q;
    let p = &from.positions[i..i + 4];
    // Each vertex as a weight on each of the quad's corners.
    let corner = |k: usize| std::array::from_fn::<f32, 4, _>(|c| if c == k { 1.0 } else { 0.0 });
    let mut outline: Vec<([f32; 4], [f32; 3])> = Vec::new();
    for k in 0..4 {
        outline.push((corner(k), p[k]));
        let (a, b) = (p[k], p[(k + 1) % 4]);
        let length = (0..3).map(|c| (b[c] - a[c]).abs()).sum::<f32>();
        for &step in &splits[k] {
            let f = step as f32 / length;
            let mut weights = [0.0; 4];
            weights[k] = 1.0 - f;
            weights[(k + 1) % 4] = f;
            // Placed exactly a whole number of boxels along, to land on the other corner.
            let at = std::array::from_fn(|c| a[c] + (b[c] - a[c]) / length * step as f32);
            outline.push((weights, at));
        }
    }
    let middle: [f32; 3] = std::array::from_fn(|c| (p[0][c] + p[1][c] + p[2][c] + p[3][c]) / 4.0);

    let base = to.positions.len() as u32;
    let mut push = |weights: [f32; 4], at: [f32; 3]| {
        let mix = |v: [f32; 4]| (0..4).map(|k| v[k] * weights[k]).sum::<f32>();
        let mix4 = |field: &[[f32; 4]]| std::array::from_fn::<f32, 4, _>(|c| mix(std::array::from_fn(|k| field[i + k][c])));
        to.positions.push(at);
        to.normals.push(from.normals[i]);
        to.colors.push(mix4(&from.colors));
        to.solid.push(mix4(&from.solid));
        to.edges.push(from.edges[i]);
        to.face.push(std::array::from_fn(|c| mix(std::array::from_fn(|k| from.face[i + k][c] as f32)).round() as u16));
        if !from.light.is_empty() {
            to.light.push(std::array::from_fn(|c| mix(std::array::from_fn(|k| from.light[i + k][c] as f32)).round() as u8));
        }
        if !from.ao.is_empty() {
            to.ao.push(from.ao[i]);
        }
    };
    push([0.25; 4], middle);
    for &(weights, at) in &outline {
        push(weights, at);
    }
    // Wound as the quad's first triangle is.
    let tri = &from.indices[6 * q..6 * q + 3];
    let at = |v: u32| from.positions[v as usize];
    let normal = |a: [f32; 3], b: [f32; 3], c: [f32; 3]| {
        let (d, e): ([f32; 3], [f32; 3]) = (std::array::from_fn(|k| b[k] - a[k]), std::array::from_fn(|k| c[k] - a[k]));
        [d[1] * e[2] - d[2] * e[1], d[2] * e[0] - d[0] * e[2], d[0] * e[1] - d[1] * e[0]]
    };
    let (n1, n2) = (normal(at(tri[0]), at(tri[1]), at(tri[2])), normal(p[0], p[1], p[2]));
    let backwards = (0..3).map(|k| n1[k] * n2[k]).sum::<f32>() < 0.0;
    let n = outline.len() as u32;
    for k in 0..n {
        let (a, b) = (base + 1 + k, base + 1 + (k + 1) % n);
        to.indices.extend(if backwards { [base, b, a] } else { [base, a, b] });
    }
}

/// Merges the quads of a chunk's mesh where it can (see the module). Quads that don't merge
/// are kept as they were.
fn merge_quads(mesh: &MeshData) -> MeshData {
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
                // Corner k's word from the face at that corner: over the face, it's the same
                // as every other's; past its two edges, the same all along them.
                let corners = [first, at(w - 1, 0), at(w - 1, h - 1), at(0, h - 1)];
                let ao = (!mesh.ao.is_empty()).then(|| std::array::from_fn(|k| mesh.ao[4 * corners[k]][k]));
                merged(mesh, first, w, h, edges, ao, &mut out);
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
    // An outer edge must look the same all along: its pattern, and the space past it.
    let edge = |q: usize, k: usize| mesh.edges[4 * q][k];
    let same = |q: usize, other: usize, k: usize| edge(q, k) == edge(other, k) && past(mesh, q, k) == past(mesh, other, k);
    let mut w = 1;
    while let Some(q) = at(w, 0) {
        let before = at(w - 1, 0).expect("in the row");
        if edge(before, 1) != FLAT || edge(q, 3) != FLAT || !same(q, first, 0) || !same(q, first, 2) {
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
                && same(q, row[0], 2)
                && if inside_left { edge(q, 3) == FLAT } else { same(q, first, 3) }
                && if inside_right { edge(q, 1) == FLAT } else { same(q, right, 1) };
            if !ok {
                break 'rows;
            }
        }
        h += 1;
    }
    (w, h)
}

/// Copies quad `q` of `from` to the end of `to`, as it was.
pub(crate) fn copy(from: &MeshData, q: usize, to: &mut MeshData) {
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
    if !from.ao.is_empty() {
        to.ao.extend_from_slice(&from.ao[i..i + 4]);
    }
    to.indices.extend(from.indices[6 * q..6 * q + 6].iter().map(|&k| k - i as u32 + first));
}

/// Adds quad `first` grown `w` boxels along its edge 0 and `h` along its edge 3 backwards,
/// with `edges` past its four sides and `ao` around its corners.
#[allow(clippy::too_many_arguments)]
fn merged(from: &MeshData, first: usize, w: i32, h: i32, edges: [u32; 4], ao: Option<[u32; 4]>, to: &mut MeshData) {
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
        if let Some(ao) = ao {
            to.ao.push(ao);
        }
    }
    to.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::{AoTuning, FACE_SCALE, ao_lit, mesh_chunk};
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
        let merged = merge_quads(&plain);
        (plain, merged)
    }

    fn floor(grid: &mut Grid, block: BlockId, size: i64) {
        for (x, y) in (0..size).flat_map(|x| (0..size).map(move |y| (x, y))) {
            grid.set_block(BoxelPos::new(x, y, 0), block);
        }
    }

    /// What a pixel of a quad shows (`s`, `t`) of the way along its edge 0 and up its edge 3,
    /// as the shader reads it: color with ambient occlusion as the shader works it out (drawn
    /// with the default tuning; in 100,000ths, as mixing differs in the last bits), light, the
    /// look's texel and volume, and for each edge, what's past it at this texel and how many
    /// texels away, where that's close enough to draw a line (edge lines are at most 8 wide).
    type Pixel = ([i64; 4], Option<[u8; 4]>, [i32; 4], [(u32, i32); 4]);

    fn pixel(mesh: &MeshData, q: usize, s: f32, t: f32) -> Pixel {
        let i = 4 * q;
        let mix = |v: [f32; 4]| v[0] * (1.0 - s) * (1.0 - t) + v[1] * s * (1.0 - t) + v[2] * s * t + v[3] * (1.0 - s) * t;
        let channel = |f: &dyn Fn(usize) -> f32| mix([f(i), f(i + 1), f(i + 2), f(i + 3)]);
        let face: [f32; 4] = std::array::from_fn(|c| channel(&|k| mesh.face[k][c] as f32 / u16::MAX as f32 * FACE_SCALE));
        let ao = mesh.ao.get(i).map_or(1.0, |w| ao_lit(w, &AoTuning::default(), face));
        let color = std::array::from_fn(|c| (channel(&|k| mesh.colors[k][c]) * if c < 3 { ao } else { 1.0 } * 1e5).round() as i64);
        let light = (!mesh.light.is_empty()).then(|| mesh.light[i]);
        let solid: [f32; 3] = std::array::from_fn(|c| channel(&|k| mesh.solid[k][c]));
        let size = LookDef::SIZE as i32;
        let texel = [0, 1, 2].map(|c| (solid[c].floor() as i32).rem_euclid(size));
        let look = [texel[0], texel[1], texel[2], mesh.solid[i][3] as i32];
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
    fn darkened_corners_merge_as_much_as_plain_ones() {
        let (blocks, volumes, [stone, ..]) = blocks();
        let mut grid = Grid::default();
        floor(&mut grid, stone, 8);
        grid.set_block(BoxelPos::new(4, 4, 1), stone);
        let up = |m: &MeshData| (0..m.quad_count()).filter(|&q| m.normals[4 * q] == [0.0, 0.0, 1.0]).count();
        // The shader darkens the floor around the block, so it merges as it would without
        // AO, in a few rectangles: faces with a crease toward the block kept apart.
        let (_, without) = both(&mut grid, &blocks, &volumes, &AoTuning::OFF);
        let (plain, merged) = both(&mut grid, &blocks, &volumes, &AoTuning::default());
        assert_eq!(up(&plain), 64 - 1 + 1);
        assert_eq!(up(&merged), up(&without));
        assert!(up(&merged) <= 12, "{} upward quads", up(&merged));
        looks_the_same(&plain, &merged);
    }

    #[test]
    fn a_floor_along_a_wall_merges_into_one_quad() {
        let (blocks, volumes, [stone, ..]) = blocks();
        let mut grid = Grid::default();
        floor(&mut grid, stone, 8);
        // A wall along the floor's north edge, and one block's corner touching it diagonally.
        for x in 0..8 {
            grid.set_block(BoxelPos::new(x, 8, 1), stone);
        }
        grid.set_block(BoxelPos::new(8, 8, 1), stone);
        let (plain, merged) = both(&mut grid, &blocks, &volumes, &AoTuning::default());
        let tops = (0..merged.quad_count()).filter(|&q| merged.normals[4 * q] == [0.0, 0.0, 1.0] && merged.positions[4 * q][2] == 1.0).count();
        assert_eq!(tops, 1, "dark along the wall, light away from it, one quad");
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

    /// Points where a vertex lies strictly inside an outer edge of the triangles, edges
    /// counted by where they are, so the two triangles of a quad (or a fan) share theirs.
    fn t_junctions(mesh: &MeshData) -> Vec<[f32; 3]> {
        let key = |p: [f32; 3]| p.map(f32::to_bits);
        let mut edges: HashMap<([u32; 3], [u32; 3]), usize> = HashMap::new();
        for tri in mesh.indices.chunks(3) {
            for k in 0..3 {
                let (a, b) = (key(mesh.positions[tri[k] as usize]), key(mesh.positions[tri[(k + 1) % 3] as usize]));
                *edges.entry(if a < b { (a, b) } else { (b, a) }).or_default() += 1;
            }
        }
        let points: std::collections::HashSet<[u32; 3]> = mesh.positions.iter().map(|&p| key(p)).collect();
        let mut found = Vec::new();
        for (&(a, b), &count) in &edges {
            if count > 1 {
                continue;
            }
            let (a, b) = (a.map(f32::from_bits), b.map(f32::from_bits));
            for p in points.iter().map(|p| p.map(f32::from_bits)) {
                let d: [f32; 3] = std::array::from_fn(|k| b[k] - a[k]);
                let e: [f32; 3] = std::array::from_fn(|k| p[k] - a[k]);
                let cross = [d[1] * e[2] - d[2] * e[1], d[2] * e[0] - d[0] * e[2], d[0] * e[1] - d[1] * e[0]];
                let along = (0..3).map(|k| d[k] * e[k]).sum::<f32>() / (0..3).map(|k| d[k] * d[k]).sum::<f32>();
                if cross.iter().all(|c| c.abs() < 1e-5) && along > 1e-5 && along < 1.0 - 1e-5 {
                    found.push(p);
                }
            }
        }
        found
    }

    /// The area the triangles cover.
    fn triangle_area(mesh: &MeshData) -> f64 {
        mesh.indices
            .chunks(3)
            .map(|t| {
                let [a, b, c] = [0, 1, 2].map(|i| mesh.positions[t[i] as usize].map(f64::from));
                let (d, e) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
                let n = [d[1] * e[2] - d[2] * e[1], d[2] * e[0] - d[0] * e[2], d[0] * e[1] - d[1] * e[0]];
                (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() / 2.0
            })
            .sum()
    }

    #[test]
    fn no_corner_sits_partway_along_a_merged_edge() {
        let (blocks, volumes, [stone, slab, log]) = blocks();
        let mut grid = Grid::default();
        // Away from the chunk's borders: a row of stone whose tops merge, beside a row of
        // stone and logs taking turns, whose tops can't; and a row of slabs, whose half-high
        // sides end partway along their merged top's edge.
        for x in 4..12 {
            grid.set_block(BoxelPos::new(x, 4, 4), stone);
            grid.set_block(BoxelPos::new(x, 5, 4), if x % 2 == 0 { stone } else { log });
            grid.set_block(BoxelPos::new(x, 8, 4), slab);
        }
        grid.relight_all(&blocks);
        let plain = mesh_chunk(&grid, ChunkPos::new(0, 0, 0), &blocks, &volumes, &AoTuning::OFF);
        let quads = merge_quads(&plain);
        assert!(!t_junctions(&quads).is_empty(), "the test should have T-junctions to close");
        let closed = merge(&plain);
        assert_eq!(t_junctions(&closed), Vec::<[f32; 3]>::new());
        assert!((triangle_area(&closed) - triangle_area(&plain)).abs() < 1e-3);
        same_values(&quads, &closed);
    }

    /// Every vertex of `closed` carries what a quad of `quads` it lies on has there (on an
    /// edge between two, either one's).
    fn same_values(quads: &MeshData, closed: &MeshData) {
        for v in 0..closed.positions.len() {
            let point = closed.positions[v];
            let matches = |q: usize, s: f32, t: f32| {
                let i = 4 * q;
                let w = [(1.0 - s) * (1.0 - t), s * (1.0 - t), s * t, (1.0 - s) * t];
                let mix = |f: &dyn Fn(usize) -> f32| (0..4).map(|k| f(i + k) * w[k]).sum::<f32>();
                (0..4).all(|c| {
                    (mix(&|k| quads.solid[k][c]) - closed.solid[v][c]).abs() < 1e-4
                        && (mix(&|k| quads.colors[k][c]) - closed.colors[v][c]).abs() < 1e-5
                        && (mix(&|k| quads.face[k][c] as f32) - closed.face[v][c] as f32).abs() <= 0.5
                        && quads.light.get(i).is_none_or(|_| (mix(&|k| quads.light[k][c] as f32) - closed.light[v][c] as f32).abs() <= 0.5)
                }) && closed.edges[v] == quads.edges[i]
                    && quads.ao.get(i) == closed.ao.get(v)
            };
            let found = (0..quads.quad_count())
                .filter(|&q| quads.normals[4 * q] == closed.normals[v])
                .filter_map(|q| on_quad(quads, q, point).map(|(s, t)| (q, s, t)))
                .any(|(q, s, t)| matches(q, s, t));
            assert!(found, "{point:?} facing {:?} matches no quad under it", closed.normals[v]);
        }
    }

    #[test]
    fn edges_on_the_chunks_border_split_at_every_boxel() {
        let (blocks, volumes, [stone, ..]) = blocks();
        let mut grid = Grid::default();
        // The next chunk's corners along the border aren't known, so the floor's top splits at
        // every boxel along its four sides: one point at each boxel, eight triangles a boxel.
        floor(&mut grid, stone, 32);
        let (plain, quads) = both(&mut grid, &blocks, &volumes, &AoTuning::default());
        let closed = close_t_junctions(&quads);
        let top = |m: &MeshData| m.indices.chunks(3).filter(|t| m.normals[t[0] as usize] == [0.0, 0.0, 1.0]).count();
        assert_eq!(top(&quads), 2);
        assert_eq!(top(&closed), 4 * 32);
        assert!((triangle_area(&closed) - triangle_area(&plain)).abs() < 1e-3);
        same_values(&quads, &closed);
    }

    #[test]
    fn the_shader_finds_each_texels_place_in_the_world() {
        // `solid.wgsl` works out a texel's world position as the boxel's corner, which is the
        // fragment's position less its place in the look (rounded), times 16, plus the look
        // position floored. Across every quad of looks that don't turn, merged or not, that
        // must be the texel the point lies in, a half texel behind the face.
        let (blocks, volumes, [stone, slab, log]) = blocks();
        let mut grid = Grid::default();
        floor(&mut grid, stone, 6);
        for x in 0..4 {
            grid.set_block(BoxelPos::new(x, 2, 1), slab);
            // Slabs stood on end, a turned shape whose look stays with the world.
            grid.set_cell(BoxelPos::new(x, 4, 1), &[Occupant::turned(slab, Orientation::z_along(0))]);
        }
        grid.set_block(BoxelPos::new(5, 5, 1), stone);
        grid.set_cell(BoxelPos::new(0, 0, 1), &[Occupant::turned(log, Orientation::z_along(0))]);
        let log_volume = volumes.of(log).unwrap() as f32;
        let (plain, merged) = both(&mut grid, &blocks, &volumes, &AoTuning::OFF);
        let mut checked = 0;
        for mesh in [&plain, &merged] {
            for q in 0..mesh.quad_count() {
                let i = 4 * q;
                if mesh.solid[i][3] == log_volume {
                    continue;
                }
                let n = mesh.normals[i];
                for (s, t) in [(0.1, 0.1), (0.5, 0.5), (0.93, 0.27), (0.4, 0.97)] {
                    let mix = |f: &dyn Fn(usize) -> f32| {
                        f(i) * (1.0 - s) * (1.0 - t) + f(i + 1) * s * (1.0 - t) + f(i + 2) * s * t + f(i + 3) * (1.0 - s) * t
                    };
                    let pos: [f32; 3] = std::array::from_fn(|c| mix(&|k| mesh.positions[k][c]));
                    let solid: [f32; 3] = std::array::from_fn(|c| mix(&|k| mesh.solid[k][c]));
                    let shader: [i64; 3] =
                        std::array::from_fn(|c| (pos[c] - solid[c] / 16.0).round() as i64 * 16 + solid[c].floor() as i64);
                    let behind: [f32; 3] = std::array::from_fn(|c| pos[c] * 16.0 - n[c] * 0.5);
                    // Right on a texel's edge either side is fine; float error picks one.
                    if behind.iter().any(|b| (b - b.round()).abs() < 1e-3) {
                        continue;
                    }
                    let truth = behind.map(|b| b.floor() as i64);
                    assert_eq!(shader, truth, "quad {q} at {pos:?}, look {solid:?}");
                    checked += 1;
                }
            }
        }
        assert!(checked > 100, "{checked}");
    }

    #[test]
    fn the_shader_knows_the_face_scale() {
        let shader = include_str!("solid.wgsl");
        assert!(shader.contains(&format!("const FACE_SCALE: f32 = {FACE_SCALE:.1};")), "solid.wgsl's FACE_SCALE should be {FACE_SCALE}");
        let levels = crate::volumes::LEVELS;
        assert!(shader.contains(&format!("const LEVELS: i32 = {levels};")), "solid.wgsl's LEVELS should be {levels}");
        let variants = crate::volumes::VARIANTS;
        assert!(shader.contains(&format!("const VARIANTS: i32 = {variants};")), "solid.wgsl's VARIANTS should be {variants}");
        // `variant_of` hashes as `client::volumes::variant_of` does.
        for k in ["0x8da6b343u", "0xd8163841u", "0xcb1ab31fu", "0x7feb352du", "0x846ca68bu"] {
            assert!(shader.contains(k), "solid.wgsl's variant_of should use {k}");
        }
    }
}
