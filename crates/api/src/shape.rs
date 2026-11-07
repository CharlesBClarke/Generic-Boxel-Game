//! Shapes as grids of filled texels: the form crafting works in. A block's shape is a list
//! of boxes ([`BlockBox`]); a [`ShapeGrid`] says, for each of the 16×16×16 texels of a
//! boxel, whether it's filled, and converts to and from boxes.

use std::fmt;

use crate::{BlockBox, Orientation};

/// Texels along each side of a boxel.
const N: usize = BlockBox::SIZE as usize;

/// Which texels of a boxel a shape fills. Plain data: one row of 16 bits per `(y, z)`, bit
/// `x` set where filled.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ShapeGrid {
    rows: [u16; N * N],
}

impl Default for ShapeGrid {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl fmt::Debug for ShapeGrid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ShapeGrid({} of 4096, {:016x})", self.volume(), self.hash64())
    }
}

impl ShapeGrid {
    /// Nothing filled.
    pub const EMPTY: ShapeGrid = ShapeGrid { rows: [0; N * N] };
    /// The whole boxel.
    pub const FULL: ShapeGrid = ShapeGrid { rows: [u16::MAX; N * N] };

    /// The raw rows, for storing a shape: one per `(y, z)`, `y + 16 * z`, bit `x` set where filled.
    pub fn rows(&self) -> &[u16; N * N] {
        &self.rows
    }

    /// A shape from rows laid out as [`ShapeGrid::rows`] gives them.
    pub fn from_rows(rows: [u16; N * N]) -> ShapeGrid {
        ShapeGrid { rows }
    }

    fn row(y: usize, z: usize) -> usize {
        y + N * z
    }

    /// Whether texel `(x, y, z)` is filled, each below 16.
    pub fn get(&self, x: usize, y: usize, z: usize) -> bool {
        self.rows[Self::row(y, z)] & (1 << x) != 0
    }

    pub fn set(&mut self, x: usize, y: usize, z: usize, filled: bool) {
        let row = &mut self.rows[Self::row(y, z)];
        if filled {
            *row |= 1 << x;
        } else {
            *row &= !(1 << x);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rows.iter().all(|&r| r == 0)
    }

    pub fn is_full(&self) -> bool {
        self.rows.iter().all(|&r| r == u16::MAX)
    }

    /// How many texels are filled: the shape's volume in 1/4096ths of a boxel.
    pub fn volume(&self) -> u32 {
        self.rows.iter().map(|r| r.count_ones()).sum()
    }

    /// Every texel inside any of `boxes`. Boxes outside the boxel are cut to it.
    pub fn from_boxes(boxes: &[BlockBox]) -> ShapeGrid {
        let mut grid = Self::EMPTY;
        for b in boxes {
            let [x0, y0, z0] = b.min.map(|c| (c as usize).min(N));
            let [x1, y1, z1] = b.max.map(|c| (c as usize).min(N));
            if x0 >= x1 {
                continue;
            }
            let bits = (((1u32 << (x1 - x0)) - 1) << x0) as u16;
            for z in z0..z1 {
                for y in y0..y1 {
                    grid.rows[Self::row(y, z)] |= bits;
                }
            }
        }
        grid
    }

    /// The shape as boxes that cover every filled texel exactly once, merged greedily: each
    /// box grows along X, then Y, then Z as far as the shape allows.
    pub fn to_boxes(&self) -> Vec<BlockBox> {
        let mut left = self.rows;
        let mut boxes = Vec::new();
        for z in 0..N {
            for y in 0..N {
                while left[Self::row(y, z)] != 0 {
                    let row = left[Self::row(y, z)];
                    let x0 = row.trailing_zeros() as usize;
                    let run = (row >> x0).trailing_ones() as usize;
                    let bits = (((1u32 << run) - 1) << x0) as u16;
                    let fits = |left: &[u16; N * N], y: usize, z: usize| left[Self::row(y, z)] & bits == bits;
                    let mut y1 = y + 1;
                    while y1 < N && fits(&left, y1, z) {
                        y1 += 1;
                    }
                    let mut z1 = z + 1;
                    while z1 < N && (y..y1).all(|yy| fits(&left, yy, z1)) {
                        z1 += 1;
                    }
                    for zz in z..z1 {
                        for yy in y..y1 {
                            left[Self::row(yy, zz)] &= !bits;
                        }
                    }
                    boxes.push(BlockBox::new([x0 as u8, y as u8, z as u8], [(x0 + run) as u8, y1 as u8, z1 as u8]));
                }
            }
        }
        boxes
    }

    /// Every texel filled in either.
    pub fn union(&self, other: &ShapeGrid) -> ShapeGrid {
        let mut out = self.clone();
        for (a, b) in out.rows.iter_mut().zip(&other.rows) {
            *a |= b;
        }
        out
    }

    /// Every texel filled in both.
    pub fn intersection(&self, other: &ShapeGrid) -> ShapeGrid {
        let mut out = self.clone();
        for (a, b) in out.rows.iter_mut().zip(&other.rows) {
            *a &= b;
        }
        out
    }

    /// Every texel filled in this one but not in `other`.
    pub fn minus(&self, other: &ShapeGrid) -> ShapeGrid {
        let mut out = self.clone();
        for (a, b) in out.rows.iter_mut().zip(&other.rows) {
            *a &= !b;
        }
        out
    }

    /// Whether any texel is filled in both.
    pub fn overlaps(&self, other: &ShapeGrid) -> bool {
        self.rows.iter().zip(&other.rows).any(|(a, b)| a & b != 0)
    }

    /// The shape turned by `orientation` about the boxel's center.
    pub fn turned(&self, orientation: Orientation) -> ShapeGrid {
        let mut out = Self::EMPTY;
        for z in 0..N {
            for y in 0..N {
                for x in 0..N {
                    if self.get(x, y, z) {
                        // A texel's center, doubled so it's a whole number about the middle.
                        let p = [x, y, z].map(|c| 2 * c as i32 - (N as i32 - 1));
                        let [a, b, c] = orientation.apply(p).map(|c| ((c + N as i32 - 1) / 2) as usize);
                        out.set(a, b, c, true);
                    }
                }
            }
        }
        out
    }

    /// The shape at quarter detail, if it has no finer detail: bit `qx + 4 qy + 16 qz` set where
    /// the 4×4×4 texels of quarter `(qx, qy, qz)` are all filled. `None` if any quarter is
    /// only partly filled.
    pub fn quarters(&self) -> Option<u64> {
        let mut out = 0u64;
        for qz in 0..4 {
            for qy in 0..4 {
                for qx in 0..4 {
                    let bits = 0xfu16 << (4 * qx);
                    let mut filled = 0;
                    for z in 4 * qz..4 * qz + 4 {
                        for y in 4 * qy..4 * qy + 4 {
                            match self.rows[Self::row(y, z)] & bits {
                                0 => {}
                                b if b == bits => filled += 1,
                                _ => return None,
                            }
                        }
                    }
                    match filled {
                        0 => {}
                        16 => out |= 1 << (qx + 4 * qy + 16 * qz),
                        _ => return None,
                    }
                }
            }
        }
        Some(out)
    }

    /// A hash of the filled texels, the same on every machine (64-bit FNV-1a), for naming
    /// shapes.
    pub fn hash64(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for r in self.rows {
            for byte in r.to_le_bytes() {
                h ^= byte as u64;
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLAB: BlockBox = BlockBox::new([0, 0, 0], [16, 16, 8]);
    const STAIRS: [BlockBox; 2] = [SLAB, BlockBox::new([0, 8, 8], [16, 16, 16])];

    /// Every texel `boxes` cover, counting overlaps.
    fn coverage(boxes: &[BlockBox]) -> Vec<u8> {
        let mut count = vec![0u8; N * N * N];
        for b in boxes {
            for z in b.min[2]..b.max[2] {
                for y in b.min[1]..b.max[1] {
                    for x in b.min[0]..b.max[0] {
                        count[x as usize + N * (y as usize + N * z as usize)] += 1;
                    }
                }
            }
        }
        count
    }

    fn check_boxes(grid: &ShapeGrid) {
        let boxes = grid.to_boxes();
        let count = coverage(&boxes);
        for z in 0..N {
            for y in 0..N {
                for x in 0..N {
                    let expected = grid.get(x, y, z) as u8;
                    assert_eq!(count[x + N * (y + N * z)], expected, "({x}, {y}, {z}) covered once if filled");
                }
            }
        }
        assert!(boxes.iter().all(BlockBox::is_valid));
    }

    #[test]
    fn boxes_go_in_and_come_back() {
        assert_eq!(ShapeGrid::from_boxes(&[BlockBox::FULL]), ShapeGrid::FULL);
        assert_eq!(ShapeGrid::FULL.to_boxes(), [BlockBox::FULL]);
        assert_eq!(ShapeGrid::EMPTY.to_boxes(), []);
        assert_eq!(ShapeGrid::from_boxes(&[SLAB]).to_boxes(), [SLAB]);
        let stairs = ShapeGrid::from_boxes(&STAIRS);
        assert_eq!(stairs.volume(), 4096 * 3 / 4);
        assert_eq!(ShapeGrid::from_boxes(&stairs.to_boxes()), stairs);
        assert_eq!(stairs.to_boxes().len(), 2);
        check_boxes(&stairs);
    }

    #[test]
    fn merged_boxes_cover_every_texel_once() {
        // A scatter that leaves little to merge, and a hollow pipe.
        let mut scatter = ShapeGrid::EMPTY;
        let mut n = 12345u32;
        for z in 0..N {
            for y in 0..N {
                for x in 0..N {
                    n = n.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    scatter.set(x, y, z, n >> 16 & 1 == 1);
                }
            }
        }
        check_boxes(&scatter);
        let pipe = ShapeGrid::from_boxes(&[
            BlockBox::new([0, 0, 0], [16, 16, 4]),
            BlockBox::new([0, 0, 12], [16, 16, 16]),
            BlockBox::new([0, 0, 4], [4, 16, 12]),
            BlockBox::new([12, 0, 4], [16, 16, 12]),
        ]);
        check_boxes(&pipe);
        assert_eq!(pipe.to_boxes().len(), 4);
    }

    #[test]
    fn turning_matches_turning_boxes() {
        let stairs = ShapeGrid::from_boxes(&STAIRS);
        for o in Orientation::all() {
            let boxes: Vec<_> = STAIRS.iter().map(|&b| o.apply_box(b)).collect();
            assert_eq!(stairs.turned(o), ShapeGrid::from_boxes(&boxes));
        }
    }

    #[test]
    fn quarters_only_without_finer_detail() {
        assert_eq!(ShapeGrid::FULL.quarters(), Some(u64::MAX));
        assert_eq!(ShapeGrid::EMPTY.quarters(), Some(0));
        // A slab is the bottom two layers of quarters.
        assert_eq!(ShapeGrid::from_boxes(&[SLAB]).quarters(), Some(0xffff_ffff));
        let corner = ShapeGrid::from_boxes(&[BlockBox::new([0, 0, 0], [4, 4, 4])]);
        assert_eq!(corner.quarters(), Some(1));
        assert_eq!(ShapeGrid::from_boxes(&[BlockBox::new([0, 0, 0], [2, 4, 4])]).quarters(), None);
        assert_eq!(ShapeGrid::from_boxes(&[BlockBox::new([0, 0, 0], [16, 16, 7])]).quarters(), None);
    }

    #[test]
    fn hashes_tell_shapes_apart() {
        let slab = ShapeGrid::from_boxes(&[SLAB]);
        assert_eq!(slab.hash64(), ShapeGrid::from_boxes(&[SLAB]).hash64());
        assert_ne!(slab.hash64(), ShapeGrid::from_boxes(&STAIRS).hash64());
        assert_ne!(slab.hash64(), slab.turned(Orientation::quarter_turns(0, 2)).hash64());
    }
}
