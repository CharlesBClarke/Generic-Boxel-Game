//! Block orientations: the 24 ways to turn a cube so it stays lined up with the grid.

use crate::block::BlockBox;

/// One of the 24 grid-aligned rotations of a cube. Turns happen about the boxel's center.
/// Plain data: a rotation matrix whose entries are -1, 0 or 1, one non-zero per row and column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Orientation {
    /// Row by row: `rows[i][j]` is how much input axis `j` adds to output axis `i`.
    rows: [[i8; 3]; 3],
}

impl Default for Orientation {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Orientation {
    /// Not turned at all.
    pub const IDENTITY: Orientation = Orientation { rows: [[1, 0, 0], [0, 1, 0], [0, 0, 1]] };

    /// A turn of `quarters` quarter turns about `axis` (0 = X, 1 = Y, 2 = Z), counterclockwise
    /// seen from the positive end of the axis.
    pub fn quarter_turns(axis: usize, quarters: i32) -> Orientation {
        // One counterclockwise quarter turn maps the next axis (a) onto the one after (b), and
        // b onto minus a.
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        let mut one = [[0i8; 3]; 3];
        one[axis][axis] = 1;
        one[b][a] = 1;
        one[a][b] = -1;
        let one = Orientation { rows: one };
        (0..quarters.rem_euclid(4)).fold(Self::IDENTITY, |acc, _| acc.then(one))
    }

    /// The turn that points +Z along `axis` (0 = X, 1 = Y, 2 = Z), such as a log lying on
    /// its side.
    pub fn z_along(axis: usize) -> Orientation {
        match axis {
            0 => Self::quarter_turns(1, 1),
            1 => Self::quarter_turns(0, -1),
            _ => Self::IDENTITY,
        }
    }

    /// The turn that points +Z along `up` and +Y along `forward`, two different axis directions
    /// such as `[0, 0, -1]`. `None` if they aren't unit axes at right angles.
    pub fn from_axes(up: [i32; 3], forward: [i32; 3]) -> Option<Orientation> {
        let unit = |v: [i32; 3]| v.iter().map(|c| c.abs()).sum::<i32>() == 1 && v.iter().all(|c| c.abs() <= 1);
        let dot: i32 = (0..3).map(|i| up[i] * forward[i]).sum();
        if !unit(up) || !unit(forward) || dot != 0 {
            return None;
        }
        // +X goes along forward × up, keeping it right-handed.
        let x = [
            forward[1] * up[2] - forward[2] * up[1],
            forward[2] * up[0] - forward[0] * up[2],
            forward[0] * up[1] - forward[1] * up[0],
        ];
        let columns = [x, forward, up];
        let rows = [0, 1, 2].map(|i| [0, 1, 2].map(|j| columns[j][i] as i8));
        Some(Orientation { rows })
    }

    /// All 24, identity first, in a fixed order.
    pub fn all() -> Vec<Orientation> {
        const PERMUTATIONS: [[usize; 3]; 6] = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
        let mut all = Vec::with_capacity(24);
        for p in PERMUTATIONS {
            for signs in 0..8 {
                let mut rows = [[0i8; 3]; 3];
                for (i, &col) in p.iter().enumerate() {
                    rows[i][col] = if signs & (1 << i) == 0 { 1 } else { -1 };
                }
                let o = Orientation { rows };
                if o.determinant() == 1 {
                    all.push(o);
                }
            }
        }
        all
    }

    fn determinant(&self) -> i32 {
        let m = self.rows.map(|r| r.map(i32::from));
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// This turn followed by `next`.
    pub fn then(self, next: Orientation) -> Orientation {
        let mut rows = [[0i8; 3]; 3];
        for (i, row) in rows.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| next.rows[i][k] * self.rows[k][j]).sum();
            }
        }
        Orientation { rows }
    }

    /// Turns a direction or offset.
    pub fn apply(&self, v: [i32; 3]) -> [i32; 3] {
        self.rows.map(|r| r[0] as i32 * v[0] + r[1] as i32 * v[1] + r[2] as i32 * v[2])
    }

    /// Turns a box about the boxel's center.
    pub fn apply_box(&self, b: BlockBox) -> BlockBox {
        let half = BlockBox::SIZE as i32 / 2;
        let turn = |p: [u8; 3]| self.apply(p.map(|c| c as i32 - half)).map(|c| (c + half) as u8);
        let (p, q) = (turn(b.min), turn(b.max));
        BlockBox { min: [0, 1, 2].map(|i| p[i].min(q[i])), max: [0, 1, 2].map(|i| p[i].max(q[i])) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_24_distinct_turns() {
        let all = Orientation::all();
        assert_eq!(all.len(), 24);
        assert_eq!(all[0], Orientation::IDENTITY);
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), 24);
    }

    #[test]
    fn turns_compose_and_close() {
        let all = Orientation::all();
        for &a in &all {
            for &b in &all {
                assert!(all.contains(&a.then(b)));
            }
            assert_eq!(a.then(Orientation::IDENTITY), a);
        }
        // Four quarter turns about any axis are no turn.
        for axis in 0..3 {
            assert_eq!(Orientation::quarter_turns(axis, 4), Orientation::IDENTITY);
            assert_eq!(Orientation::quarter_turns(axis, -1), Orientation::quarter_turns(axis, 3));
        }
    }

    #[test]
    fn quarter_turns_go_counterclockwise() {
        // About +Z, east turns to north.
        assert_eq!(Orientation::quarter_turns(2, 1).apply([1, 0, 0]), [0, 1, 0]);
        assert_eq!(Orientation::quarter_turns(0, 1).apply([0, 1, 0]), [0, 0, 1]);
        assert_eq!(Orientation::quarter_turns(1, 1).apply([0, 0, 1]), [1, 0, 0]);
        // `then` applies the first turn first.
        let a = Orientation::quarter_turns(2, 1);
        let b = Orientation::quarter_turns(0, 1);
        assert_eq!(a.then(b).apply([1, 0, 0]), b.apply(a.apply([1, 0, 0])));
    }

    #[test]
    fn z_along_points_up_along_each_axis() {
        assert_eq!(Orientation::z_along(0).apply([0, 0, 1]), [1, 0, 0]);
        assert_eq!(Orientation::z_along(1).apply([0, 0, 1]), [0, 1, 0]);
        assert_eq!(Orientation::z_along(2).apply([0, 0, 1]), [0, 0, 1]);
    }

    #[test]
    fn from_axes_points_up_and_forward() {
        assert_eq!(Orientation::from_axes([0, 0, 1], [0, 1, 0]), Some(Orientation::IDENTITY));
        for o in Orientation::all() {
            let (up, forward) = (o.apply([0, 0, 1]), o.apply([0, 1, 0]));
            assert_eq!(Orientation::from_axes(up, forward), Some(o));
        }
        assert_eq!(Orientation::from_axes([0, 0, 1], [0, 0, -1]), None);
        assert_eq!(Orientation::from_axes([0, 1, 1], [1, 0, 0]), None);
    }

    #[test]
    fn boxes_turn_about_the_center() {
        let slab = BlockBox::new([0, 0, 0], [16, 16, 8]);
        let flipped = Orientation::quarter_turns(0, 2).apply_box(slab);
        assert_eq!(flipped, BlockBox::new([0, 0, 8], [16, 16, 16]));
        let wall = Orientation::z_along(0).apply_box(slab);
        // The slab's bottom turns to face west.
        assert_eq!(wall, BlockBox::new([0, 0, 0], [8, 16, 16]));
        for o in Orientation::all() {
            assert_eq!(o.apply_box(BlockBox::FULL), BlockBox::FULL);
        }
    }
}
