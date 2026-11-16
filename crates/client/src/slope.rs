//! The slope of the ground around each face of far terrain (see [`crate::far`]): which way
//! the surface faces there, worked out from the solid cells near the face and packed into
//! the face's stored light, where far terrain keeps nothing else. `solid.wgsl` draws the
//! face as the staircase of full-detail faces such a slope is, so a coarse step's top and
//! side come out alike and a hillside shades as one surface.
//!
//! A far chunk is meshed with the far chunks of its level beside it, so the cells near a face
//! are known out to [`REACH`] past its sides where those are loaded. Where they aren't, the
//! ground is taken to carry on evenly: an unknown cell is the opposite of the known one
//! straight across the face from it, which is exact wherever the surface is a plane through
//! the face.

use api::{CHUNK_SIZE, ChunkPos, I64Vec3, LocalPos};
use sim::Grid;

use crate::mesher::MeshData;

/// How many cells from a face count toward its slope, each way.
pub const REACH: i32 = 3;

/// Steps each way a packed slope's two numbers take: fewer lets more faces merge (see
/// [`crate::greedy`]), more shades more evenly. Divides 124.
const STEPS: f32 = 62.0;

const SIZE: i32 = CHUNK_SIZE as i32;

/// Which cells around one far chunk are solid, as far as its grid says: the far chunk and
/// [`REACH`] cells past it every way.
pub struct Cells {
    /// The lowest corner of the box kept, in cells from the far chunk's lowest corner.
    lo: [i32; 3],
    /// Cells across the box, each way.
    size: [i32; 3],
    /// By X, then Y, then Z: 0 empty, 1 solid, 2 not known.
    cells: Vec<u8>,
}

impl Cells {
    /// The cells around the far chunk at `pos` in `grid`, which holds it and the far chunks of
    /// its level beside it that are loaded: cells in a chunk `grid` doesn't have aren't known.
    pub fn of(grid: &Grid, pos: ChunkPos) -> Self {
        let lo = [-REACH; 3];
        let side = SIZE + 2 * REACH;
        // The 27 chunks the box reaches into, looked up once.
        let near: Vec<Option<&sim::Chunk>> = (0..27).map(|i| grid.chunk(ChunkPos(pos.0 + I64Vec3::new(i % 3 - 1, i / 3 % 3 - 1, i / 9 - 1)))).collect();
        Self::from_fn(lo, [side; 3], |x, y, z| {
            let (cx, cy, cz) = (x.div_euclid(SIZE), y.div_euclid(SIZE), z.div_euclid(SIZE));
            let chunk = near[((cx + 1) + 3 * (cy + 1) + 9 * (cz + 1)) as usize]?;
            let local = LocalPos::new((x - cx * SIZE) as u8, (y - cy * SIZE) as u8, (z - cz * SIZE) as u8);
            Some(!chunk.cells.get(local).is_empty())
        })
    }

    /// A box of cells `size` across from `lo`, solid, empty or not known as `solid` says.
    fn from_fn(lo: [i32; 3], size: [i32; 3], solid: impl Fn(i32, i32, i32) -> Option<bool>) -> Self {
        let mut cells = Vec::with_capacity((size[0] * size[1] * size[2]) as usize);
        for z in lo[2]..lo[2] + size[2] {
            for y in lo[1]..lo[1] + size[1] {
                for x in lo[0]..lo[0] + size[0] {
                    cells.push(solid(x, y, z).map_or(2, |s| s as u8));
                }
            }
        }
        Cells { lo, size, cells }
    }

    /// Whether the cell is solid, if it's known.
    #[inline]
    pub(crate) fn at(&self, x: i32, y: i32, z: i32) -> Option<bool> {
        let (x, y, z) = (x - self.lo[0], y - self.lo[1], z - self.lo[2]);
        if x < 0 || y < 0 || z < 0 || x >= self.size[0] || y >= self.size[1] || z >= self.size[2] {
            return None;
        }
        match self.cells[(x + self.size[0] * (y + self.size[1] * z)) as usize] {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    /// Which way the surface faces around a face at `center` (in cells from the far chunk's
    /// lowest corner) facing along `axis`, forward if `sign` is 1 and backward if -1: the way
    /// the cells within [`REACH`] thin out, weighted toward the near ones. The face's own
    /// direction where that says nothing.
    fn slope(&self, center: [f32; 3], axis: usize, sign: f32) -> [f32; 3] {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        // The cell just ahead of the face along its axis; the one just behind is one back.
        let mut ahead = center.map(|c| c.floor() as i32);
        ahead[axis] = (center[axis] + 0.5).floor() as i32;
        let cell = |a: i32, du: i32, dv: i32| {
            let mut c = ahead;
            c[axis] += a;
            c[u] += du;
            c[v] += dv;
            self.at(c[0], c[1], c[2])
        };
        // Toward the solid: every cell and the one straight across the face from it.
        let mut toward = [0.0f32; 3];
        for &(a, du, dv, weight) in pairs() {
            let fill = |s: bool| s as i32 as f32;
            let more = match (cell(a, du, dv), cell(-1 - a, -du, -dv)) {
                (Some(ahead), Some(behind)) => fill(ahead) - fill(behind),
                (Some(ahead), None) => 2.0 * fill(ahead) - 1.0,
                (None, Some(behind)) => 1.0 - 2.0 * fill(behind),
                (None, None) => continue,
            };
            let more = more * weight;
            toward[axis] += more * (a as f32 + 0.5);
            toward[u] += more * du as f32;
            toward[v] += more * dv as f32;
        }
        let mut own = [0.0; 3];
        own[axis] = sign;
        let length = (toward[0] * toward[0] + toward[1] * toward[1] + toward[2] * toward[2]).sqrt();
        // The surface faces away from the solid, and never behind the face itself.
        if length < 1e-4 || -toward[axis] * sign <= 0.0 {
            return own;
        }
        toward.map(|c| -c / length)
    }
}

/// The cells that count toward a slope, on one side of the face: how many cells out along
/// the face's axis, where across it, and how much each counts, less the farther off.
fn pairs() -> &'static [(i32, i32, i32, f32)] {
    static PAIRS: std::sync::OnceLock<Vec<(i32, i32, i32, f32)>> = std::sync::OnceLock::new();
    PAIRS.get_or_init(|| {
        let reach = ((REACH + 1) * (REACH + 1)) as f32;
        let mut pairs = Vec::new();
        for a in 0..REACH {
            for du in -REACH..=REACH {
                for dv in -REACH..=REACH {
                    let out = a as f32 + 0.5;
                    let weight = 1.0 - (out * out + (du * du + dv * dv) as f32) / reach;
                    if weight > 0.0 {
                        pairs.push((a, du, dv, weight));
                    }
                }
            }
        }
        pairs
    })
}

/// Packs a direction into two bytes: its place on an octahedron unfolded to a square, each
/// way in [`STEPS`] steps, with straight up exactly in the middle.
pub fn pack(n: [f32; 3]) -> [u8; 2] {
    let sum = n[0].abs() + n[1].abs() + n[2].abs();
    if sum <= 0.0 {
        return [128, 128];
    }
    let (x, y) = (n[0] / sum, n[1] / sum);
    let side = |c: f32| if c >= 0.0 { 1.0 } else { -1.0 };
    let (x, y) = if n[2] < 0.0 { ((1.0 - y.abs()) * side(x), (1.0 - x.abs()) * side(y)) } else { (x, y) };
    let byte = |c: f32| (128.0 + (c * STEPS).round() * (124.0 / STEPS)) as u8;
    [byte(x), byte(y)]
}

/// The direction [`pack`] packed, as `solid.wgsl` reads it.
pub fn unpack(packed: [u8; 2]) -> [f32; 3] {
    let x = (packed[0] as f32 - 128.0) / 124.0;
    let y = (packed[1] as f32 - 128.0) / 124.0;
    let z = 1.0 - x.abs() - y.abs();
    let fold = (-z).max(0.0);
    let x = x + if x >= 0.0 { -fold } else { fold };
    let y = y + if y >= 0.0 { -fold } else { fold };
    let length = (x * x + y * y + z * z).sqrt();
    [x / length, y / length, z / length]
}

/// Gives every quad of a far chunk's mesh its slope from `cells`, the cells around it (see
/// the module), packed into its stored light's red and green. A face that isn't open to the
/// air as far as `cells` knows keeps its own direction.
pub fn set_slopes(mesh: &mut MeshData, cells: &Cells) {
    if mesh.light.len() != mesh.positions.len() {
        mesh.light = vec![crate::mesher::OPEN_SKY; mesh.positions.len()];
    }
    for q in 0..mesh.positions.len() / 4 {
        let i = 4 * q;
        let normal = mesh.normals[i];
        let mut center = [0.0f32; 3];
        for p in &mesh.positions[i..i + 4] {
            for k in 0..3 {
                center[k] += p[k] / 4.0;
            }
        }
        let facing = (0..3).find(|&k| normal[k].abs() == 1.0 && normal[(k + 1) % 3] == 0.0 && normal[(k + 2) % 3] == 0.0);
        let slope = match facing {
            Some(axis) => {
                let sign = normal[axis];
                let mut front = center;
                front[axis] += 0.5 * sign;
                let front = front.map(|c| c.floor() as i32);
                if cells.at(front[0], front[1], front[2]) == Some(false) { cells.slope(center, axis, sign) } else { normal }
            }
            None => normal,
        };
        let [r, g] = pack(slope);
        for light in &mut mesh.light[i..i + 4] {
            light[0] = r;
            light[1] = g;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn angle(a: [f32; 3], b: [f32; 3]) -> f32 {
        let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let len = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        (dot / (len(a) * len(b))).clamp(-1.0, 1.0).acos().to_degrees()
    }

    /// Ground rising `rise` cells every `run` cells east, 16 high at the west side, known
    /// only over the far chunk (96 cells tall here), not past its sides.
    fn hillside(rise: i32, run: i32) -> Cells {
        Cells::from_fn([0, 0, -REACH], [SIZE, SIZE, 96 + 2 * REACH], move |x, _, z| Some(z < 16 + x.div_euclid(run) * rise))
    }

    /// The far chunk's own cells, `height` tall, solid where `solid` says; nothing past it known.
    fn alone(height: i32, solid: impl Fn(i32, i32, i32) -> bool) -> Cells {
        Cells::from_fn([0; 3], [SIZE, SIZE, height], move |x, y, z| Some(solid(x, y, z)))
    }

    /// The top face of the ground's highest cell at `x`, and the riser facing west there if
    /// the ground steps up to it.
    fn faces(cells: &Cells, x: i32, y: i32) -> ([f32; 3], Option<[f32; 3]>) {
        let top = |x: i32| (0..96).rev().find(|&z| cells.at(x, y, z) == Some(true)).unwrap();
        let z = top(x);
        let tread = cells.slope([x as f32 + 0.5, y as f32 + 0.5, z as f32 + 1.0], 2, 1.0);
        let riser = (x > 0 && top(x - 1) < z).then(|| cells.slope([x as f32, y as f32 + 0.5, z as f32 + 0.5], 0, -1.0));
        (tread, riser)
    }

    #[test]
    fn flat_ground_faces_straight_up() {
        let cells = alone(32, |_, _, z| z < 10);
        for (x, y) in [(16, 16), (0, 0), (31, 7)] {
            assert_eq!(cells.slope([x as f32 + 0.5, y as f32 + 0.5, 10.0], 2, 1.0), [0.0, 0.0, 1.0]);
        }
        assert_eq!(pack([0.0, 0.0, 1.0]), [128, 128]);
    }

    #[test]
    fn a_staircase_s_tops_and_sides_face_the_way_the_hillside_does() {
        // Rise over run, and how far off the hillside's own direction a face may be: the
        // gentler, the wider its steps against the reach.
        for (rise, run, off) in [(1, 1, 4.0), (2, 1, 6.0), (1, 2, 8.0), (1, 3, 12.0)] {
            let cells = hillside(rise, run);
            let expected = [-(rise as f32), 0.0, run as f32];
            for x in 12..20 {
                let (tread, riser) = faces(&cells, x, 16);
                assert!(angle(tread, expected) < off, "{rise}/{run} top at {x}: {tread:?}, {}", angle(tread, expected));
                if let Some(riser) = riser {
                    assert!(angle(riser, expected) < off, "{rise}/{run} side at {x}: {riser:?}, {}", angle(riser, expected));
                }
            }
        }
    }

    #[test]
    fn the_ground_carries_on_past_what_s_known() {
        // The same hillside, read at the far chunk's west and east sides and in its middle.
        for (rise, run) in [(1, 1), (1, 2)] {
            let cells = hillside(rise, run);
            let expected = [-(rise as f32), 0.0, run as f32];
            for x in [0, 1, 30, 31] {
                let (tread, _) = faces(&cells, x, 16);
                assert!(angle(tread, expected) < 10.0, "{rise}/{run} at {x}: {tread:?}, {}", angle(tread, expected));
            }
            // Along the north and south sides nothing changes.
            for y in [0, 31] {
                let (tread, _) = faces(&cells, 16, y);
                assert!(angle(tread, expected) < 10.0, "{rise}/{run} at y {y}: {tread:?}");
            }
        }
    }

    #[test]
    fn an_overhang_s_underside_faces_down() {
        let cells = alone(64, |_, _, z| z >= 40);
        assert_eq!(cells.slope([16.5, 16.5, 40.0], 2, -1.0), [0.0, 0.0, -1.0]);
    }

    #[test]
    fn cells_past_the_sides_come_from_the_far_chunks_beside_it() {
        let mut blocks = sim::BlockRegistry::default();
        let stone = blocks.register(api::BlockDef::new("t:stone", true, [0; 3])).unwrap();
        // A far chunk with a slab on its floor, and the one over it solid all through.
        let chunk = |z: i64, solid: &dyn Fn(LocalPos) -> bool| {
            let mut chunk = sim::ChunkData {
                pos: ChunkPos(I64Vec3::new(0, 0, z)),
                lists: vec![Vec::new(), vec![sim::Occupant::new(stone)]],
                cells: sim::Layer::Uniform(0),
                light: sim::Layer::Uniform(sim::Light::SKY),
                records: Vec::new(),
            };
            for i in 0..api::CHUNK_VOLUME {
                if solid(LocalPos::from_index(i)) {
                    chunk.cells.set_at(i, 1);
                }
            }
            chunk
        };
        let mut grid = Grid::default();
        grid.import_chunk(&chunk(0, &|p| p.z < 2));
        grid.import_chunk(&chunk(1, &|_| true));
        let cells = Cells::of(&grid, ChunkPos(I64Vec3::ZERO));
        assert_eq!(cells.at(5, 5, 0), Some(true));
        assert_eq!(cells.at(5, 5, 2), Some(false));
        assert_eq!(cells.at(5, 5, 32), Some(true), "from the one over it");
        assert_eq!(cells.at(5, 5, 32 + REACH), None, "past what's kept");
        assert_eq!(cells.at(-1, 5, 1), None, "no far chunk west of it");
        assert_eq!(cells.at(5, 5, -1), None, "nor under it");
    }

    #[test]
    fn packing_keeps_a_direction_within_two_degrees() {
        let mut worst = 0.0f32;
        for i in 0..2000 {
            let (a, b) = (i as f32 * 0.37, i as f32 * 0.011 - 1.0);
            let r = (1.0 - b * b).max(0.0).sqrt();
            let n = [r * a.cos(), r * a.sin(), b];
            worst = worst.max(angle(unpack(pack(n)), n));
        }
        assert!(worst < 2.0, "{worst}");
    }

    #[test]
    fn slopes_go_in_the_light_and_side_walls_keep_their_own() {
        let cells = hillside(1, 1);
        let quad = |corners: [[f32; 3]; 4], normal: [f32; 3], mesh: &mut MeshData| {
            mesh.positions.extend(corners);
            mesh.normals.extend([normal; 4]);
            mesh.light.extend([[9, 9, 9, 200]; 4]);
        };
        let mut mesh = MeshData::default();
        // The top at x 16 (the ground there is 32 high), then the far chunk's west wall.
        quad([[16.0, 16.0, 32.0], [17.0, 16.0, 32.0], [17.0, 17.0, 32.0], [16.0, 17.0, 32.0]], [0.0, 0.0, 1.0], &mut mesh);
        quad([[0.0, 16.0, 3.0], [0.0, 17.0, 3.0], [0.0, 17.0, 4.0], [0.0, 16.0, 4.0]], [-1.0, 0.0, 0.0], &mut mesh);
        set_slopes(&mut mesh, &cells);
        let top = mesh.light[0];
        assert!(mesh.light[..4].iter().all(|l| *l == top));
        assert!(angle(unpack([top[0], top[1]]), [-1.0, 0.0, 1.0]) < 5.0);
        // Blue and the sky's light stay.
        assert_eq!(&top[2..], [9, 200]);
        assert_eq!(unpack([mesh.light[4][0], mesh.light[4][1]]), [-1.0, 0.0, 0.0]);
    }
}
