//! The base game's shapes, built with the same rule players use at a crafting table (see
//! `api::crafting`), so the test area is built from shapes players can make.

use api::crafting::heights;
use api::{Layer, Orientation, ShapeGrid, Stacks, craft_shape};

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
}
