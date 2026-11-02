//! Crafting shapes: the rule that turns stacks built on a crafting table into a shape. Plain
//! functions over plain data, so any side of a boundary can run them.
//!
//! **Building up.** Shapes are built on the table's top, seen from its front: the grid's right
//! is +X, up the grid (away from you) is +Y, and stacks rise along +Z. Each of the 4×4 cells
//! holds a stack of up to 4 layers, bottom first; a layer is a quarter of the boxel each way.
//! A material layer is a solid quarter cube. A shape layer is that shape shrunk to a quarter,
//! turned as it is given, so a shape with detail finer than quarters can't go in a layer.
//!
//! **Nothing floats.** Every vertical line of sixteenths in the result must be filled from the
//! bottom with no gaps: what's on the table rests on the table, and each layer on what's below.
//! So every shape grows out of one flat side, its bottom, which is what placing puts against a
//! face.
//!
//! **As built.** The shape keeps the frame it was built in: placed, its far side (+Y) points the
//! way the player looks, as it pointed away from someone at the table's front. Flipping turns it
//! upside down about the Y axis, for things easier built top first.

use crate::shape::ShapeGrid;
use crate::Orientation;

/// Cells along each side of the crafting grid, and layers a stack can hold. Four, so a shape
/// shrunk to a cell lands exactly on sixteenths.
pub const GRID: usize = 4;

/// What makes a crafting table, registered by a mod.
#[derive(Clone, Debug, PartialEq)]
pub struct CraftingDef {
    /// The block players craft at, such as `"base:crafting_table"`.
    pub table: String,
}

/// One layer of a stack.
#[derive(Clone, Debug, PartialEq)]
pub enum Layer {
    /// A material: a solid quarter cube.
    Solid,
    /// A shape, shrunk to a quarter.
    Shape(Box<ShapeGrid>),
}

/// One cell's stack, bottom first. An empty layer under a full one leaves the one above
/// floating.
pub type Stack = [Option<Layer>; GRID];

/// The grid's stacks, by row from the far side (the top of the grid as drawn) and column from
/// the left.
pub type Stacks = [[Stack; GRID]; GRID];

/// Why stacks make nothing. Rows count from the far side, columns from the left, layers from
/// the bottom, all from 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CraftError {
    /// Nothing on the table.
    Empty,
    /// The layer holds a shape with detail finer than quarters, which can't shrink into it.
    TooFine { row: usize, col: usize, layer: usize },
    /// Something in this cell's stack rests on nothing.
    Floating { row: usize, col: usize },
}

impl std::fmt::Display for CraftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CraftError::Empty => write!(f, "the table is empty"),
            CraftError::TooFine { row, col, layer } => write!(
                f,
                "the shape in row {}, column {}, layer {} is too detailed to shrink into a layer",
                row + 1,
                col + 1,
                layer + 1
            ),
            CraftError::Floating { row, col } => {
                write!(f, "something in row {}, column {} rests on nothing", row + 1, col + 1)
            }
        }
    }
}

/// The shape `stacks` build, as built: before any flip.
pub fn stack_shape(stacks: &Stacks) -> Result<ShapeGrid, CraftError> {
    let mut out = ShapeGrid::EMPTY;
    let mut any = false;
    for (row, line) in stacks.iter().enumerate() {
        for (col, stack) in line.iter().enumerate() {
            // Row 0 is the far side, +Y.
            let (qx, qy) = (col, GRID - 1 - row);
            for (layer, content) in stack.iter().enumerate() {
                let quarters = match content {
                    None => continue,
                    Some(Layer::Solid) => u64::MAX,
                    Some(Layer::Shape(shape)) => shape.quarters().ok_or(CraftError::TooFine { row, col, layer })?,
                };
                any = true;
                for bit in (0..64).filter(|b| quarters & (1 << b) != 0) {
                    let s = [bit % 4, bit / 4 % 4, bit / 16];
                    out.set(4 * qx + s[0], 4 * qy + s[1], 4 * layer + s[2], true);
                }
            }
        }
    }
    if !any {
        return Err(CraftError::Empty);
    }
    if let Some((x, y)) = floating(&out) {
        return Err(CraftError::Floating { row: GRID - 1 - y / 4, col: x / 4 });
    }
    Ok(out)
}

/// The first vertical line of sixteenths with a gap under something filled, as `(x, y)`.
pub fn floating(shape: &ShapeGrid) -> Option<(usize, usize)> {
    let n = 16;
    for y in 0..n {
        for x in 0..n {
            let mut gap = false;
            for z in 0..n {
                match shape.get(x, y, z) {
                    false => gap = true,
                    true if gap => return Some((x, y)),
                    true => {}
                }
            }
        }
    }
    None
}

/// The shape upside down, turned half a turn about the Y axis so its far side stays far.
pub fn flipped(shape: &ShapeGrid) -> ShapeGrid {
    shape.turned(Orientation::quarter_turns(1, 2))
}

/// The shape `stacks` build, flipped if `flip`.
pub fn craft_shape(stacks: &Stacks, flip: bool) -> Result<ShapeGrid, CraftError> {
    let shape = stack_shape(stacks)?;
    Ok(if flip { flipped(&shape) } else { shape })
}

/// Stacks from text rows, far side first: each character is a cell's height, `0` to `4`, in
/// material layers (`.` for 0). For tests and world building; panics on anything else.
pub fn heights(rows: [&str; GRID]) -> Stacks {
    rows.map(|row| {
        let cells: Vec<Stack> = row
            .chars()
            .map(|c| {
                let h = match c {
                    '.' => 0,
                    '0'..='4' => c as usize - '0' as usize,
                    _ => panic!("heights are 0 to 4 or ., got {c:?}"),
                };
                std::array::from_fn(|layer| (layer < h).then_some(Layer::Solid))
            })
            .collect();
        cells.try_into().unwrap_or_else(|_| panic!("rows are {GRID} cells wide"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BlockBox;

    fn shape(boxes: &[([u8; 3], [u8; 3])]) -> ShapeGrid {
        ShapeGrid::from_boxes(&boxes.iter().map(|&(a, b)| BlockBox::new(a, b)).collect::<Vec<_>>())
    }

    fn craft(rows: [&str; 4]) -> Result<ShapeGrid, CraftError> {
        craft_shape(&heights(rows), false)
    }

    #[test]
    fn full_stacks_are_a_full_block() {
        assert_eq!(craft(["4444"; 4]), Ok(ShapeGrid::FULL));
        assert_eq!(craft(["...."; 4]), Err(CraftError::Empty));
    }

    #[test]
    fn half_height_everywhere_is_a_slab() {
        assert_eq!(craft(["2222"; 4]), Ok(shape(&[([0, 0, 0], [16, 16, 8])])));
        assert_eq!(craft(["1111"; 4]), Ok(shape(&[([0, 0, 0], [16, 16, 4])])), "a quarter slab");
    }

    #[test]
    fn taller_far_rows_are_stairs_rising_away() {
        let stairs = shape(&[([0, 0, 0], [16, 16, 8]), ([0, 8, 8], [16, 16, 16])]);
        assert_eq!(craft(["4444", "4444", "2222", "2222"]), Ok(stairs));
    }

    #[test]
    fn a_thin_stair_fills_only_some_rows() {
        // Rising left to right, in the far half only.
        let thin = craft(["1234", "1234", "....", "...."]).unwrap();
        let expected = shape(&[
            ([0, 8, 0], [16, 16, 4]),
            ([4, 8, 4], [16, 16, 8]),
            ([8, 8, 8], [16, 16, 12]),
            ([12, 8, 12], [16, 16, 16]),
        ]);
        assert_eq!(thin, expected);
    }

    #[test]
    fn corner_stairs_and_buckets_take_one_craft() {
        let outer = craft(["2244", "2244", "2222", "2222"]).unwrap();
        assert_eq!(outer, shape(&[([0, 0, 0], [16, 16, 8]), ([8, 8, 8], [16, 16, 16])]));
        let inner = craft(["4444", "4444", "2244", "2244"]).unwrap();
        assert_eq!(inner.volume(), 4096 * 7 / 8);
        let bucket = craft(["4444", "4114", "4114", "4444"]).unwrap();
        assert_eq!(bucket.volume(), 4096 * 12 / 16 + 4 * 64, "12 full columns and 4 one layer high");
        assert!(!bucket.get(8, 8, 15) && bucket.get(8, 8, 0), "open at the top, with a floor");
        let pipe = craft(["4444", "4..4", "4..4", "4444"]).unwrap();
        assert!((0..16).all(|z| !pipe.get(8, 8, z)), "a hole all the way up");
    }

    #[test]
    fn a_gap_in_a_stack_floats() {
        let mut stacks = heights(["....", "....", "....", "...."]);
        stacks[1][2][1] = Some(Layer::Solid);
        assert_eq!(craft_shape(&stacks, false), Err(CraftError::Floating { row: 1, col: 2 }));
        stacks[1][2][0] = Some(Layer::Solid);
        assert!(craft_shape(&stacks, false).is_ok(), "resting on the one below");
    }

    /// A quarter slab made of materials, shrunk: one sixteenth thick in a layer.
    fn quarter_slab() -> ShapeGrid {
        craft(["1111"; 4]).unwrap()
    }

    #[test]
    fn a_shape_layer_shrinks_into_its_quarter() {
        let mut stacks = heights(["....", "....", "....", "4..."]);
        stacks[3][1][0] = Some(Layer::Shape(Box::new(quarter_slab())));
        let made = craft_shape(&stacks, false).unwrap();
        // A full column at the near left, and one sixteenth of floor in the cell beside it.
        assert_eq!(made.volume(), 4 * 4 * 16 + 4 * 4);
        assert!(made.get(4, 0, 0) && !made.get(4, 0, 1));
        assert!(made.quarters().is_none(), "finer than quarters now");
    }

    #[test]
    fn a_shape_layer_must_rest_on_what_is_below() {
        // A thin floor in a layer, with a solid layer on it: the solid one sits on the floor's
        // top quarter... which isn't filled, so it floats.
        let mut stacks = heights(["....", "....", "....", "...."]);
        stacks[0][0][0] = Some(Layer::Shape(Box::new(quarter_slab())));
        stacks[0][0][1] = Some(Layer::Solid);
        assert_eq!(craft_shape(&stacks, false), Err(CraftError::Floating { row: 0, col: 0 }));
        // A post standing on full layers is fine: four stacked quarter posts.
        let post = craft(["....", ".4..", "....", "...."]).unwrap();
        let mut stacks = heights(["....", "....", "....", "...."]);
        stacks[0][0] = std::array::from_fn(|_| Some(Layer::Shape(Box::new(post.clone()))));
        assert!(craft_shape(&stacks, false).is_ok());
    }

    #[test]
    fn shapes_too_fine_for_a_layer_are_refused() {
        let mut stacks = heights(["....", "....", "....", "...."]);
        stacks[3][0][0] = Some(Layer::Shape(Box::new(quarter_slab())));
        let finer = craft_shape(&stacks, false).unwrap();
        let mut again = heights(["....", "....", "....", "...."]);
        again[1][2][0] = Some(Layer::Shape(Box::new(finer)));
        assert_eq!(craft_shape(&again, false), Err(CraftError::TooFine { row: 1, col: 2, layer: 0 }));
    }

    #[test]
    fn flipping_turns_it_upside_down_keeping_the_far_side_far() {
        let stairs = craft_shape(&heights(["4444", "4444", "2222", "2222"]), true).unwrap();
        assert_eq!(stairs, shape(&[([0, 0, 8], [16, 16, 16]), ([0, 8, 0], [16, 16, 8])]));
        // A flipped shape may hang: only the stacks have to rest.
        assert!(floating(&stairs).is_some());
    }
}
