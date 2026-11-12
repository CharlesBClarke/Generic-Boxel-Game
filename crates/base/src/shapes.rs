//! The base game's shapes, built with the same rule players use at a crafting table (see
//! `api::crafting`), so the test area is built from shapes players can make.

use api::crafting::heights;
use api::{BlockBox, Layer, Orientation, ShapeGrid, Stacks, craft_shape};

use crate::{ANTIPHASITE, BLUE_CRYSTAL, BRASS, CACHITE, COPPER, MENDITE, PHASITE};

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

/// A post a quarter of a boxel across (4 texels), standing in the middle. It straddles
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

/// The texels from `min` to `max`.
fn cube(min: [u8; 3], max: [u8; 3]) -> ShapeGrid {
    ShapeGrid::from_boxes(&[BlockBox::new(min, max)])
}

/// A door's lower half, facing -Y with its wall side +Y, like a Minecraft door: a phasite panel
/// a quarter thick against the wall side, and the same panel swung open toward you about its
/// left edge, in antiphasite, so it starts hidden and the two swap on a signal. A copper block
/// a quarter across sits in the near right corner of the floor, clear of both panels, so a
/// signal beside or in front of the door reaches it.
pub fn door_lower() -> Vec<(&'static str, ShapeGrid)> {
    let mut pieces = door_upper();
    pieces.push((COPPER, cube([12, 0, 0], [16, 4, 4])));
    pieces
}

/// A door's upper half: the two panels, lined up with the lower half's.
pub fn door_upper() -> Vec<(&'static str, ShapeGrid)> {
    vec![(PHASITE, cube([0, 12, 0], [16, 16, 16])), (ANTIPHASITE, cube([0, 0, 0], [4, 12, 16]))]
}

/// A lever set in a wall, facing -Y: a block of `material` half thick through the middle, with
/// a brass handle sticking out of the middle of each side.
pub fn lever(material: &'static str) -> Vec<(&'static str, ShapeGrid)> {
    let handles = cube([6, 0, 6], [10, 4, 10]).union(&cube([6, 12, 6], [10, 16, 10]));
    vec![(material, cube([0, 4, 0], [16, 12, 16])), (BRASS, handles)]
}

/// A shutter at the mouth of a niche, facing -Y: a phasite panel a quarter thick across the
/// near side, and a copper block a quarter across in the far corner of the floor, clear of the
/// panel's grab, so a lever beside it reaches it. A signal hides the panel and shows what's
/// behind it.
pub fn shutter() -> Vec<(&'static str, ShapeGrid)> {
    vec![(PHASITE, cube([0, 0, 0], [16, 4, 16])), (COPPER, cube([12, 12, 0], [16, 16, 4]))]
}

/// A crystal gun's trigger, firing +X: a blue crystal one texel thick against the +X side,
/// hidden by an antiphasite slab against it (which starts hidden and takes what's within a
/// texel along), and a copper block a quarter across in the far corner for a lever's signal.
/// The signal shows the slab and brings the blue back, touching a green crystal east of it.
///
/// Not a shape the table can make: the table works in quarters, and a blue crystal a quarter
/// thick is only cut by a texel's grab, its kept part still touching the green.
pub fn crystal_trigger() -> Vec<(&'static str, ShapeGrid)> {
    vec![
        (COPPER, cube([0, 0, 0], [4, 4, 4])),
        (ANTIPHASITE, cube([12, 0, 0], [15, 16, 16])),
        (BLUE_CRYSTAL, cube([15, 0, 0], [16, 16, 16])),
    ]
}

/// A mendite block with a copper block a quarter across in its bottom north-west corner, so a
/// lever beside it reaches it.
pub fn mender_tap() -> Vec<(&'static str, ShapeGrid)> {
    let tap = cube([0, 12, 0], [4, 16, 4]);
    vec![(MENDITE, ShapeGrid::FULL.minus(&tap)), (COPPER, tap)]
}

/// A cachite block with a copper block a quarter across in its top north-west corner, under a
/// [`mender_tap`]'s, so the same signal reaches the bin.
pub fn bin_tap() -> Vec<(&'static str, ShapeGrid)> {
    let tap = cube([0, 12, 12], [4, 16, 16]);
    vec![(CACHITE, ShapeGrid::FULL.minus(&tap)), (COPPER, tap)]
}

/// A copper wire a quarter across, lying on the floor along X against the +Y side.
pub fn wire() -> ShapeGrid {
    craft(&heights(["1111", "....", "....", "...."]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shapes_are_the_ones_the_test_area_always_had() {
        assert_eq!(slab(), ShapeGrid::from_boxes(&[BlockBox::new([0, 0, 0], [16, 16, 8])]));
        let stairs_boxes = [BlockBox::new([0, 0, 0], [16, 16, 8]), BlockBox::new([0, 8, 8], [16, 16, 16])];
        assert_eq!(stairs(), ShapeGrid::from_boxes(&stairs_boxes));
        assert_eq!(post(), ShapeGrid::from_boxes(&[BlockBox::new([6, 6, 0], [10, 10, 16])]));
    }

    #[test]
    fn the_door_swaps_two_panels_and_the_wire_runs_along_the_wall_side() {
        let lower = door_lower();
        let get = |m: &str| lower.iter().find(|(n, _)| *n == m).map(|(_, s)| s.clone()).unwrap();
        let (closed, open) = (get(PHASITE), get(ANTIPHASITE));
        assert!(!closed.overlaps(&open) && closed.get(0, 12, 0) && open.get(0, 11, 0), "meeting at the hinge corner");
        assert!(!get(COPPER).overlaps(&get(PHASITE).union(&get(ANTIPHASITE))));
        assert_eq!(door_upper().len(), 2);
        assert_eq!(lever(crate::BRICK)[1].1.volume(), 2 * 64);
        assert_eq!(wire(), cube([0, 12, 0], [16, 16, 4]));
    }
}
