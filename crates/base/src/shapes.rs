//! The base game's shapes, crafted with the same rule players use at a crafting table (see
//! `api::crafting`), so the test area is built from shapes players can make.

use api::crafting::{GRID, pattern};
use api::{Cell, ShapeGrid, TableFace, craft_shape};

fn craft(face: TableFace, cells: &[[Cell; GRID]; GRID]) -> ShapeGrid {
    craft_shape(face, cells).expect("the base game's patterns make shapes")
}

/// A bottom slab: the lower half, drawn at the front.
pub fn slab() -> ShapeGrid {
    craft(TableFace::Front, &pattern(["....", "....", "####", "####"]))
}

/// Stairs rising away from the front, drawn at the right side.
pub fn stairs() -> ShapeGrid {
    craft(TableFace::Right, &pattern(["..##", "..##", "####", "####"]))
}

/// A post a quarter of a boxel across (4 sixteenths), standing in the middle. It straddles
/// the middle cells rather than filling one, so it takes two crafts: four parts, each a quarter of the grid in one
/// corner, drawn on top, then those parts shrunk into the four middle cells so their corners
/// meet in the middle.
pub fn post() -> ShapeGrid {
    let corner = |rows| Cell::Shape(Box::new(craft(TableFace::Top, &pattern(rows))));
    // Seen from the top, standing at the front: up the grid is +Y, right is +X.
    let north_east = corner(["..##", "..##", "....", "...."]);
    let north_west = corner(["##..", "##..", "....", "...."]);
    let south_east = corner(["....", "....", "..##", "..##"]);
    let south_west = corner(["....", "....", "##..", "##.."]);
    let mut cells: [[Cell; GRID]; GRID] = Default::default();
    // Each middle cell keeps the corner of its part nearest the middle of the grid.
    cells[1][1] = south_east;
    cells[1][2] = south_west;
    cells[2][1] = north_east;
    cells[2][2] = north_west;
    craft(TableFace::Top, &cells)
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
