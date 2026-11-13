//! The slope of the ground around each face of far terrain (see [`crate::far`]): which way
//! the surface faces there, worked out from the solid cells near the face and packed into
//! the face's stored light, where far terrain keeps nothing else. `solid.wgsl` draws the
//! face as the staircase of full-detail faces such a slope is, so a coarse step's top and
//! side come out alike and a hillside shades as one surface.
//!
//! A far column is meshed alone, so cells past its sides aren't known. There the ground is
//! taken to carry on evenly: an unknown cell is the opposite of the known one straight across
//! the face from it, which is exact wherever the surface is a plane through the face. Up and
//! down everything is known: the server leaves out only chunks that are empty, or solid all
//! through under solid (see [`sim::far`]).

use api::{CHUNK_SIZE, CHUNK_VOLUME, ChunkPos};
use sim::{Grid, Layer};

use crate::mesher::MeshData;

/// How many cells from a face count toward its slope, each way.
pub const REACH: i32 = 3;

/// Steps each way a packed slope's two numbers take: fewer lets more faces merge (see
/// [`crate::greedy`]), more shades more evenly. Divides 124.
const STEPS: f32 = 62.0;

const SIZE: i32 = CHUNK_SIZE as i32;

/// Which cells of one far column are solid, as far as its chunks say.
pub struct Cells {
    /// Cells up the column, from the floor of its lowest chunk.
    height: i32,
    /// By X, then Y, then Z: 0 empty, 1 solid.
    cells: Vec<u8>,
    /// The lowest chunk's Z.
    bottom: i64,
    /// Whether it's solid under the lowest chunk.
    under: bool,
}

impl Cells {
    /// The cells of the column whose chunks are at `positions` in `grid`.
    pub fn of(grid: &Grid, positions: &[ChunkPos]) -> Self {
        let bottom = positions.iter().map(|p| p.0.z).min().unwrap_or(0);
        let top = positions.iter().map(|p| p.0.z).max().unwrap_or(0);
        let height = (top - bottom + 1) as i32 * SIZE;
        let mut cells = vec![0u8; (SIZE * SIZE * height) as usize];
        // From the top down: a chunk left out is solid if the one over it is solid along its
        // floor, as only a buried one is left out full, and empty otherwise.
        let mut covered = false;
        for z in (bottom..=top).rev() {
            let from = (z - bottom) as usize * CHUNK_VOLUME;
            let out = &mut cells[from..from + CHUNK_VOLUME];
            let chunk = positions.iter().find(|p| p.0.z == z).and_then(|&p| grid.chunk(p));
            match chunk.map(|c| &c.cells) {
                None => out.fill(covered as u8),
                Some(Layer::Uniform(id)) => out.fill(!id.is_empty() as u8),
                Some(Layer::Dense(ids)) => {
                    for (cell, id) in out.iter_mut().zip(ids.iter()) {
                        *cell = !id.is_empty() as u8;
                    }
                }
            }
            covered = out[..(SIZE * SIZE) as usize].iter().all(|&c| c == 1);
        }
        Cells { height, cells, bottom, under: covered }
    }

    /// A column `height` cells tall, solid where `solid` says.
    #[cfg(test)]
    fn from_fn(height: i32, solid: impl Fn(i32, i32, i32) -> bool) -> Self {
        let mut cells = Vec::new();
        for z in 0..height {
            for y in 0..SIZE {
                for x in 0..SIZE {
                    cells.push(solid(x, y, z) as u8);
                }
            }
        }
        let under = cells[..(SIZE * SIZE) as usize].iter().all(|&c| c == 1);
        Cells { height, cells, bottom: 0, under }
    }

    /// Whether the cell is solid, if it's known: not past the column's sides.
    #[inline]
    fn at(&self, x: i32, y: i32, z: i32) -> Option<bool> {
        if x < 0 || y < 0 || x >= SIZE || y >= SIZE {
            return None;
        }
        if z < 0 {
            return Some(self.under);
        }
        if z >= self.height {
            return Some(false);
        }
        Some(self.cells[(x + SIZE * (y + SIZE * z)) as usize] == 1)
    }

    /// Which way the surface faces around a face at `center` (in cells from the column's
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

/// Gives every quad of the far chunk at `pos`, meshed from `cells`' column, its slope (see the
/// module), packed into its stored light's red and green. A face that isn't open to the air
/// as far as the column knows, as on its sides, keeps its own direction.
pub fn set_slopes(mesh: &mut MeshData, cells: &Cells, pos: ChunkPos) {
    if mesh.light.len() != mesh.positions.len() {
        mesh.light = vec![crate::mesher::OPEN_SKY; mesh.positions.len()];
    }
    let lift = (pos.0.z - cells.bottom) as f32 * SIZE as f32;
    for q in 0..mesh.positions.len() / 4 {
        let i = 4 * q;
        let normal = mesh.normals[i];
        let mut center = [0.0f32; 3];
        for p in &mesh.positions[i..i + 4] {
            for k in 0..3 {
                center[k] += p[k] / 4.0;
            }
        }
        center[2] += lift;
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

    /// Ground rising `rise` cells every `run` cells east, 16 high at the west side.
    fn hillside(rise: i32, run: i32) -> Cells {
        Cells::from_fn(96, move |x, _, z| z < 16 + x.div_euclid(run) * rise)
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
        let cells = Cells::from_fn(32, |_, _, z| z < 10);
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
    fn the_ground_carries_on_past_the_column_s_sides() {
        // The same hillside, read at the column's west and east sides and in its middle.
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
        let cells = Cells::from_fn(64, |_, _, z| z >= 40);
        assert_eq!(cells.slope([16.5, 16.5, 40.0], 2, -1.0), [0.0, 0.0, -1.0]);
    }

    #[test]
    fn chunks_left_out_are_solid_under_a_solid_floor_and_empty_otherwise() {
        let mut blocks = sim::BlockRegistry::default();
        let stone = blocks.register(api::BlockDef::new("t:stone", true, [0; 3])).unwrap();
        // A chunk at Z 2 with a slab on its floor, and one at Z 0 with a slab halfway up.
        let slab = |z: i64, from: u8| {
            let mut chunk = sim::ChunkData {
                pos: ChunkPos(api::I64Vec3::new(0, 0, z)),
                lists: vec![Vec::new(), vec![sim::Occupant::new(stone)]],
                cells: Layer::Uniform(0),
                light: Layer::Uniform(sim::Light::SKY),
                records: Vec::new(),
            };
            for i in 0..CHUNK_VOLUME {
                if (from..from + 2).contains(&api::LocalPos::from_index(i).z) {
                    chunk.cells.set_at(i, 1);
                }
            }
            chunk
        };
        let mut grid = Grid::default();
        let chunks = [slab(2, 0), slab(0, 16)];
        for chunk in &chunks {
            grid.import_chunk(chunk);
        }
        let cells = Cells::of(&grid, &[chunks[0].pos, chunks[1].pos]);
        // Over the top chunk: empty. The chunk left out between the two, under a solid floor:
        // solid. Under the lowest, whose floor is empty: empty.
        assert_eq!(cells.at(5, 5, 96), Some(false));
        assert_eq!(cells.at(5, 5, 64), Some(true));
        assert_eq!(cells.at(5, 5, 66), Some(false));
        assert_eq!(cells.at(5, 5, 40), Some(true));
        assert_eq!(cells.at(5, 5, 17), Some(true));
        assert_eq!(cells.at(5, 5, 3), Some(false));
        assert_eq!(cells.at(5, 5, -1), Some(false));
        // Past its sides: not known.
        assert_eq!(cells.at(-1, 5, 17), None);
        assert_eq!(cells.at(5, 32, 17), None);
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
        // The top at x 16 (the ground there is 32 high), then the column's west wall.
        quad([[16.0, 16.0, 32.0], [17.0, 16.0, 32.0], [17.0, 17.0, 32.0], [16.0, 17.0, 32.0]], [0.0, 0.0, 1.0], &mut mesh);
        quad([[0.0, 16.0, 3.0], [0.0, 17.0, 3.0], [0.0, 17.0, 4.0], [0.0, 16.0, 4.0]], [-1.0, 0.0, 0.0], &mut mesh);
        set_slopes(&mut mesh, &cells, ChunkPos(api::I64Vec3::ZERO));
        let top = mesh.light[0];
        assert!(mesh.light[..4].iter().all(|l| *l == top));
        assert!(angle(unpack([top[0], top[1]]), [-1.0, 0.0, 1.0]) < 5.0);
        // Blue and the sky's light stay.
        assert_eq!(&top[2..], [9, 200]);
        assert_eq!(unpack([mesh.light[4][0], mesh.light[4][1]]), [-1.0, 0.0, 0.0]);
    }
}
