//! The base game's shapes, built with the same rule players use at a crafting table (see
//! `api::crafting`), so the test area is built from shapes players can make.

use api::crafting::{GRID, heights, stack_shape};
use api::{Layer, Orientation, ShapeGrid, Stacks, craft_shape, finished, stack_fill};

use crate::{BRASS, COPPER, IRON, WOOD};

fn craft(stacks: &Stacks) -> ShapeGrid {
    craft_shape(stacks, false).expect("the base game's stacks make shapes")
}

/// A bottom slab: every cell two layers high.
pub fn slab() -> ShapeGrid {
    craft(&heights(["2222"; 4]))
}

/// Stairs rising away from you: the far half four high, the near half two.
pub fn stairs() -> ShapeGrid {
    craft(&heights(["4444", "4444", "2222", "2222"]))
}

/// A post a quarter of a boxel across (4 sixteenths), standing in the middle. It straddles
/// the middle cells rather than filling one, so it takes two crafts: a column in one corner of
/// the table, then that column shrunk and stacked four high in each middle cell, turned so the
/// four meet in the middle.
pub fn post() -> ShapeGrid {
    // The column stands at the far right, in the quarter of the table nearest +X and +Y.
    let column = craft(&heights(["..44", "..44", "....", "...."]));
    let mut stacks = heights(["...."; 4]);
    // Quarter turns counterclockwise from above that bring the far right corner to each
    // middle cell's corner nearest the middle.
    let turns = [((1, 1), 3), ((1, 2), 2), ((2, 1), 0), ((2, 2), 1)];
    for ((row, col), quarters) in turns {
        let piece = column.turned(Orientation::quarter_turns(2, quarters));
        stacks[row][col] = std::array::from_fn(|_| Some(Layer::Shape(Box::new(piece.clone()))));
    }
    craft(&stacks)
}

/// The sixteenths of the wall a material fills: (row from the top, column from the left, layer
/// out from the wall).
type Cells = Vec<(usize, usize, usize)>;

/// A design of several materials built on the table's side, as a player would: each material
/// with the cells it fills, as (row from the top of the wall, column from the left, layer out
/// from the wall). Gives each material's piece, ready to place: the wall is the far side.
fn on_the_side(materials: &[(&'static str, Cells)]) -> Vec<(&'static str, ShapeGrid)> {
    let stacks = |cells: &[(usize, usize, usize)]| {
        let mut stacks: Stacks = Default::default();
        for &(row, col, layer) in cells {
            stacks[row][col][layer] = Some(Layer::Solid);
        }
        stacks
    };
    let all: Vec<_> = materials.iter().flat_map(|(_, cells)| cells.iter().copied()).collect();
    stack_shape(&stacks(&all)).expect("the base game's designs rest on the wall");
    materials
        .iter()
        .map(|(m, cells)| (*m, finished(&stack_fill(&stacks(cells)).expect("whole layers"), true, false)))
        .collect()
}

/// Every cell of the wall in columns `cols`, rows `rows`, on layer `layer`.
fn wall(rows: std::ops::Range<usize>, cols: std::ops::Range<usize>, layer: usize) -> Cells {
    rows.flat_map(|r| cols.clone().map(move |c| (r, c, layer))).collect()
}

/// A door's lower half, a quarter of a block thick against the far side: an iron hinge strip
/// up its left edge, wood across, a copper plate low on the right so a lever beside it reaches
/// it, and a brass knob sticking out toward you near the top on the right.
pub fn door_lower() -> Vec<(&'static str, ShapeGrid)> {
    let mut wood = wall(0..GRID, 1..GRID, 0);
    wood.retain(|&c| c != (3, 3, 0));
    on_the_side(&[(WOOD, wood), (IRON, wall(0..GRID, 0..1, 0)), (BRASS, vec![(1, 3, 1)]), (COPPER, vec![(3, 3, 0)])])
}

/// A door's upper half: the hinge strip and wood, lined up with the lower half's.
pub fn door_upper() -> Vec<(&'static str, ShapeGrid)> {
    on_the_side(&[(WOOD, wall(0..GRID, 1..GRID, 0)), (IRON, wall(0..GRID, 0..1, 0))])
}

/// A lever set in a wall: a block of `material` three quarters thick against the far side,
/// with a brass handle sticking out of the middle toward you.
pub fn lever(material: &'static str) -> Vec<(&'static str, ShapeGrid)> {
    let back: Vec<_> = (0..3).flat_map(|layer| wall(0..GRID, 0..GRID, layer)).collect();
    on_the_side(&[(material, back), (BRASS, vec![(1, 1, 3)])])
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::BlockBox;

    #[test]
    fn the_shapes_are_the_ones_the_test_area_always_had() {
        assert_eq!(slab(), ShapeGrid::from_boxes(&[BlockBox::new([0, 0, 0], [16, 16, 8])]));
        let stairs_boxes = [BlockBox::new([0, 0, 0], [16, 16, 8]), BlockBox::new([0, 8, 8], [16, 16, 16])];
        assert_eq!(stairs(), ShapeGrid::from_boxes(&stairs_boxes));
        assert_eq!(post(), ShapeGrid::from_boxes(&[BlockBox::new([6, 6, 0], [10, 10, 16])]));
    }

    #[test]
    fn the_door_is_a_panel_on_the_far_side_with_a_knob_toward_you() {
        let lower = door_lower();
        let get = |m: &str| lower.iter().find(|(n, _)| *n == m).map(|(_, s)| s.clone()).unwrap();
        assert_eq!(get(IRON), ShapeGrid::from_boxes(&[BlockBox::new([0, 12, 0], [4, 16, 16])]), "the hinge up the left");
        assert_eq!(get(BRASS), ShapeGrid::from_boxes(&[BlockBox::new([12, 8, 8], [16, 12, 12])]), "the knob");
        assert_eq!(get(COPPER), ShapeGrid::from_boxes(&[BlockBox::new([12, 12, 0], [16, 16, 4])]));
        assert_eq!(get(WOOD).volume() + get(COPPER).volume() + get(IRON).volume(), 16 * 16 * 4);
        assert_eq!(door_upper().len(), 2);
        assert_eq!(lever(crate::BRICK)[1].1.volume(), 64);
    }
}
