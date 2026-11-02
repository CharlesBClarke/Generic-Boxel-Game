//! Crafting shapes: the rule that turns a 4×4 pattern on one face of a crafting table into a
//! shape. Plain functions over plain data, so any side of a boundary can run them.
//!
//! **The table's frame.** A crafting table has a front, like any turned block: its frame is
//! the block's own, unturned frame, +X right, +Y away from someone standing at the front, +Z
//! up. Shapes are made in that frame, and a made shape is placed as if by someone standing at
//! the table's front: its +Y the way the player looks.
//!
//! **Faces.** Each usable face shows its grid as seen standing there, looking in: the front and
//! back look along Y, the sides along X, the top down Z. So the back is the front mirrored, and
//! one side is the other mirrored. The bottom can't be used.
//!
//! **The rule.** Each cell of the 4×4 grid is a quarter of the face, and holds a 3D copy of what
//! was put there at a quarter of its size (4×4×4 sixteenths): a material is a solid little
//! cube, a shape is that shape shrunk, kept turned as it is in the table's frame. The copy is
//! repeated 4 times along the face's axis to fill the boxel's depth. So a pipe made at the
//! front, used at a side, gives 4 small pipes per cell with their holes running across. A shape
//! with detail finer than quarters can't be shrunk into a cell.

use crate::shape::ShapeGrid;

/// Cells along each side of the crafting grid. Four, so a cell of a shape shrunk to a cell
/// lands exactly on sixteenths.
pub const GRID: usize = 4;

/// What makes a crafting table, registered by a mod.
#[derive(Clone, Debug, PartialEq)]
pub struct CraftingDef {
    /// The block players craft at, such as `"base:crafting_table"`.
    pub table: String,
}

/// A usable face of a crafting table, in the table's frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableFace {
    Top,
    /// Facing -Y: standing here, the table's +Y runs away from you.
    Front,
    Back,
    /// Facing -X.
    Left,
    /// Facing +X.
    Right,
}

impl TableFace {
    pub const ALL: [TableFace; 5] = [TableFace::Top, TableFace::Front, TableFace::Back, TableFace::Left, TableFace::Right];

    /// The face whose outward normal, in the table's frame, is `normal`. `None` for the bottom
    /// or anything that isn't a unit axis.
    pub fn from_normal(normal: [i32; 3]) -> Option<TableFace> {
        match normal {
            [0, 0, 1] => Some(TableFace::Top),
            [0, -1, 0] => Some(TableFace::Front),
            [0, 1, 0] => Some(TableFace::Back),
            [-1, 0, 0] => Some(TableFace::Left),
            [1, 0, 0] => Some(TableFace::Right),
            _ => None,
        }
    }

    /// The outward normal, in the table's frame.
    pub fn normal(self) -> [i32; 3] {
        match self {
            TableFace::Top => [0, 0, 1],
            TableFace::Front => [0, -1, 0],
            TableFace::Back => [0, 1, 0],
            TableFace::Left => [-1, 0, 0],
            TableFace::Right => [1, 0, 0],
        }
    }

    /// The grid's right and up as seen standing at this face, in the table's frame. On the
    /// top, you stand at the front looking down, so up on the grid is away from you.
    pub fn right_up(self) -> ([i32; 3], [i32; 3]) {
        match self {
            TableFace::Top => ([1, 0, 0], [0, 1, 0]),
            TableFace::Front => ([1, 0, 0], [0, 0, 1]),
            TableFace::Back => ([-1, 0, 0], [0, 0, 1]),
            TableFace::Left => ([0, -1, 0], [0, 0, 1]),
            TableFace::Right => ([0, 1, 0], [0, 0, 1]),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TableFace::Top => "top",
            TableFace::Front => "front",
            TableFace::Back => "back",
            TableFace::Left => "left side",
            TableFace::Right => "right side",
        }
    }

    /// Which way the made shape runs through, in words.
    pub fn pushes(self) -> &'static str {
        match self {
            TableFace::Top => "up and down",
            TableFace::Front | TableFace::Back => "away from the front",
            TableFace::Left | TableFace::Right => "side to side",
        }
    }
}

/// What one cell of the grid holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Cell {
    #[default]
    Empty,
    /// A material: a solid little cube.
    Solid,
    /// A shape, shrunk to the cell.
    Shape(Box<ShapeGrid>),
}

/// Why a pattern makes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CraftError {
    /// Nothing in the grid.
    Empty,
    /// The cell at this row (from the top) and column (from the left) holds a shape with
    /// detail finer than quarters, which can't shrink into a cell.
    TooFine { row: usize, col: usize },
}

impl std::fmt::Display for CraftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CraftError::Empty => write!(f, "the grid is empty"),
            CraftError::TooFine { row, col } => {
                write!(f, "the shape in row {}, column {} is too detailed to shrink into a cell", row + 1, col + 1)
            }
        }
    }
}

/// The shape a pattern makes on `face`. `cells[row][col]`, rows from the top of the grid as seen
/// at that face, columns from the left.
pub fn craft_shape(face: TableFace, cells: &[[Cell; GRID]; GRID]) -> Result<ShapeGrid, CraftError> {
    let (right, up) = face.right_up();
    let into = face.normal().map(|c| -c);
    let axis = |v: [i32; 3]| v.iter().position(|&c| c != 0).expect("a unit axis");
    // A position along a direction, 0 to 3 counted along it, as a quarter index along its axis.
    let along = |v: [i32; 3], i: usize| if v[axis(v)] > 0 { i } else { GRID - 1 - i };
    let mut out = ShapeGrid::EMPTY;
    let mut any = false;
    for (row, line) in cells.iter().enumerate() {
        for (col, cell) in line.iter().enumerate() {
            let quarters = match cell {
                Cell::Empty => continue,
                Cell::Solid => u64::MAX,
                Cell::Shape(shape) => shape.quarters().ok_or(CraftError::TooFine { row, col })?,
            };
            any = true;
            for depth in 0..GRID {
                let mut q = [0usize; 3];
                q[axis(right)] = along(right, col);
                q[axis(up)] = along(up, GRID - 1 - row);
                q[axis(into)] = along(into, depth);
                for bit in (0..64).filter(|b| quarters & (1 << b) != 0) {
                    let s = [bit % 4, bit / 4 % 4, bit / 16];
                    out.set(4 * q[0] + s[0], 4 * q[1] + s[1], 4 * q[2] + s[2], true);
                }
            }
        }
    }
    if !any {
        return Err(CraftError::Empty);
    }
    Ok(out)
}

/// A pattern from text rows, top first: `#` a material, `.` empty. For tests and world
/// building; panics on other characters or the wrong size.
pub fn pattern(rows: [&str; GRID]) -> [[Cell; GRID]; GRID] {
    rows.map(|row| {
        let cells: Vec<Cell> = row
            .chars()
            .map(|c| match c {
                '#' => Cell::Solid,
                '.' => Cell::Empty,
                _ => panic!("pattern cells are # or ., got {c:?}"),
            })
            .collect();
        cells.try_into().unwrap_or_else(|_| panic!("pattern rows are {GRID} cells wide"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BlockBox;

    fn shape(boxes: &[([u8; 3], [u8; 3])]) -> ShapeGrid {
        ShapeGrid::from_boxes(&boxes.iter().map(|&(a, b)| BlockBox::new(a, b)).collect::<Vec<_>>())
    }

    const SIDE_STAIRS: [&str; 4] = ["..##", "..##", "####", "####"];

    #[test]
    fn a_full_grid_is_a_full_block() {
        for face in TableFace::ALL {
            assert_eq!(craft_shape(face, &pattern(["####"; 4])), Ok(ShapeGrid::FULL));
        }
        assert_eq!(craft_shape(TableFace::Top, &pattern(["...."; 4])), Err(CraftError::Empty));
    }

    #[test]
    fn the_bottom_half_is_a_slab() {
        let slab = shape(&[([0, 0, 0], [16, 16, 8])]);
        let bottom = pattern(["....", "....", "####", "####"]);
        for face in [TableFace::Front, TableFace::Back, TableFace::Left, TableFace::Right] {
            assert_eq!(craft_shape(face, &bottom), Ok(slab.clone()), "{face:?}");
        }
        // On the top, the bottom rows are the near half: a slab standing at the front.
        assert_eq!(craft_shape(TableFace::Top, &bottom), Ok(shape(&[([0, 0, 0], [16, 8, 16])])));
    }

    #[test]
    fn stairs_drawn_at_each_face() {
        let cells = pattern(SIDE_STAIRS);
        // Seen from the right side, the grid's right is +Y: stairs rising away from the front,
        // which is how the old registered stairs stood.
        let stairs = shape(&[([0, 0, 0], [16, 16, 8]), ([0, 8, 8], [16, 16, 16])]);
        assert_eq!(craft_shape(TableFace::Right, &cells), Ok(stairs));
        // From the left side it's mirrored: rising toward the front.
        let toward = shape(&[([0, 0, 0], [16, 16, 8]), ([0, 0, 8], [16, 8, 16])]);
        assert_eq!(craft_shape(TableFace::Left, &cells), Ok(toward));
        // From the front the high step is on the right, running front to back.
        let right = shape(&[([0, 0, 0], [16, 16, 8]), ([8, 0, 8], [16, 16, 16])]);
        assert_eq!(craft_shape(TableFace::Front, &cells), Ok(right));
        // From the back, the right as seen there is the table's left.
        let left = shape(&[([0, 0, 0], [16, 16, 8]), ([0, 0, 8], [8, 16, 16])]);
        assert_eq!(craft_shape(TableFace::Back, &cells), Ok(left));
    }

    #[test]
    fn fine_stairs_rise_by_quarters() {
        let cells = pattern(["...#", "..##", ".###", "####"]);
        let fine = shape(&[
            ([0, 0, 0], [16, 16, 4]),
            ([0, 4, 4], [16, 16, 8]),
            ([0, 8, 8], [16, 16, 12]),
            ([0, 12, 12], [16, 16, 16]),
        ]);
        assert_eq!(craft_shape(TableFace::Right, &cells), Ok(fine));
    }

    /// A pipe made at the front: a square ring, its hole running front to back.
    fn pipe() -> ShapeGrid {
        craft_shape(TableFace::Front, &pattern(["####", "#..#", "#..#", "####"])).unwrap()
    }

    #[test]
    fn a_pipe_is_a_ring_pushed_through() {
        let pipe = pipe();
        assert_eq!(pipe.volume(), 4096 * 12 / 16);
        for y in 0..16 {
            assert!(!pipe.get(8, y, 8), "the hole runs all the way through");
            assert!(pipe.get(0, y, 0));
        }
    }

    #[test]
    fn materials_alone_make_the_plain_prism() {
        // A shape that's all one quarter cube per cell is the same as a material there.
        let mut cells = pattern(SIDE_STAIRS);
        cells[0][3] = Cell::Shape(Box::new(ShapeGrid::FULL));
        assert_eq!(craft_shape(TableFace::Right, &cells), craft_shape(TableFace::Right, &pattern(SIDE_STAIRS)));
    }

    #[test]
    fn a_pipe_on_its_own_axis_makes_a_finer_prism() {
        let mut cells = pattern(["....", "....", "....", "...."]);
        cells[3][0] = Cell::Shape(Box::new(pipe()));
        let made = craft_shape(TableFace::Front, &cells).unwrap();
        // The cell at the bottom left is the quarter column x 0..4, z 0..4, all along Y: a
        // small ring, pushed through, so a small hole runs front to back.
        assert_eq!(made.volume(), 16 * 12);
        for y in 0..16 {
            assert!(made.get(0, y, 0) && made.get(3, y, 3));
            assert!(!made.get(1, y, 1) && !made.get(2, y, 2), "the little hole runs through");
        }
        assert!(made.quarters().is_none(), "finer than quarters now");
    }

    #[test]
    fn a_pipe_on_another_face_gives_four_holes_across() {
        let mut cells = pattern(["....", "....", "....", "...."]);
        cells[3][0] = Cell::Shape(Box::new(pipe()));
        let made = craft_shape(TableFace::Right, &cells).unwrap();
        // From the right side the grid's left is the front (low Y) and the push runs along X.
        // The cell holds a small pipe whose hole still runs along Y, copied 4 times along X:
        // four holes, each through the 4 sixteenths at y 0..4.
        assert_eq!(made.volume(), 4 * 4 * 4 * 12 / 16 * 4);
        for copy in 0..4 {
            let x = 4 * copy + 1;
            for y in 0..4 {
                assert!(!made.get(x, y, 1) && !made.get(x + 1, y, 2), "hole {copy} runs along Y");
            }
            assert!(made.get(x - 1, 0, 0) && made.get(x + 2, 3, 3), "the ring around hole {copy}");
        }
        // Nothing outside that quarter column.
        assert!(!made.get(0, 4, 0) && !made.get(0, 0, 4));
    }

    #[test]
    fn shapes_too_fine_for_a_cell_are_refused() {
        let mut cells = pattern(["....", "....", "....", "...."]);
        cells[3][0] = Cell::Shape(Box::new(pipe()));
        let finer = craft_shape(TableFace::Front, &cells).unwrap();
        let mut again = pattern(["....", "....", "....", "...."]);
        again[1][2] = Cell::Shape(Box::new(finer));
        assert_eq!(craft_shape(TableFace::Front, &again), Err(CraftError::TooFine { row: 1, col: 2 }));
    }

    #[test]
    fn faces_come_from_normals() {
        for face in TableFace::ALL {
            assert_eq!(TableFace::from_normal(face.normal()), Some(face));
            let (right, up) = face.right_up();
            // Right, up and looking in make a right-handed view: right × up points out at you.
            let out = [
                right[1] * up[2] - right[2] * up[1],
                right[2] * up[0] - right[0] * up[2],
                right[0] * up[1] - right[1] * up[0],
            ];
            assert_eq!(out, face.normal(), "{face:?}");
        }
        assert_eq!(TableFace::from_normal([0, 0, -1]), None);
    }
}
