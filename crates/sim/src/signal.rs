//! Signals and what answers them: properties that come from materials (see [`api::Property`]).
//! A boxel's occupants act together as one block. Pressing interact on a trigger piece signals
//! its boxel and every touching boxel with a listener in it; a signaled boxel with a hinge
//! piece swings, with the rest of its hinge line.
//!
//! **Swinging.** A hinge piece turns about its long axis (up when it has none longer): its
//! boxel and every boxel along the same line whose hinge shares the axis swing together, a
//! quarter turn about that line. Of the two ways, the one leaving less outside the boxel wins
//! (counterclockwise on a tie), and the result slides back inside. Swung pieces become open
//! kinds that remember what they were (see [`BlockRegistry::opened`]), so the next signal puts
//! back exactly what was there: if anything along the line is open, the line closes.

use api::{BoxelPos, I64Vec3, Property, ShapeGrid};

use crate::{BlockRegistry, Grid, Occupant};

/// Sixteenths across a boxel.
const N: i32 = 16;

/// The boxels a trigger at `pos` signals: its own, then each touching boxel with a listener.
pub fn reached(grid: &Grid, blocks: &BlockRegistry, pos: BoxelPos) -> Vec<BoxelPos> {
    let mut out = vec![pos];
    for axis in 0..3 {
        for side in [-1, 1] {
            let mut step = [0; 3];
            step[axis] = side;
            let next = pos.offset(I64Vec3::from_array(step));
            if has(grid, blocks, next, Property::Listener) {
                out.push(next);
            }
        }
    }
    out
}

/// Whether anything in the boxel at `pos` has `property`.
pub fn has(grid: &Grid, blocks: &BlockRegistry, pos: BoxelPos, property: Property) -> bool {
    grid.cell(pos).iter().any(|o| blocks.get(o.block).is_some_and(|d| d.has(property)))
}

/// What one occupant becomes when its line swings.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// Swings open into this shape, unturned: it becomes the open kind of the occupant.
    Open { closed: Occupant, shape: Box<ShapeGrid> },
    /// An open kind going back to what it was.
    Close(Occupant),
    /// Stays as it is: something placed into an open door's boxel.
    Keep(Occupant),
}

/// What a swing does: each boxel along the hinge line, with what each of its occupants becomes,
/// in the boxel's order.
pub type Swing = Vec<(BoxelPos, Vec<Change>)>;

/// The swing a signal to `pos` starts, or `None` if nothing there swings: no hinge, a full
/// block along the line, or a hinge whose middle doesn't fall on a sixteenth's corner or middle
/// so a quarter turn would leave the grid.
pub fn swing(grid: &Grid, blocks: &BlockRegistry, pos: BoxelPos) -> Option<Swing> {
    let line = hinge_line(grid, blocks, pos)?;
    let cells: Vec<(BoxelPos, Vec<Occupant>)> = line.boxels.iter().map(|&p| (p, grid.cell(p).to_vec())).collect();
    let open = cells.iter().flat_map(|(_, c)| c).any(|o| blocks.closed_of(o.block).is_some());
    if open {
        let close = |o: &Occupant| blocks.closed_of(o.block).map_or(Change::Keep(*o), Change::Close);
        return Some(cells.iter().map(|(p, c)| (*p, c.iter().map(close).collect())).collect());
    }
    if cells.iter().flat_map(|(_, c)| c).any(|o| blocks.get(o.block).is_none_or(|d| d.is_full())) {
        return None;
    }
    let shapes: Vec<Vec<ShapeGrid>> =
        cells.iter().map(|(_, c)| c.iter().map(|o| blocks.shape(o.block).turned(o.orientation)).collect()).collect();
    let turn = line.turn(shapes.iter().flatten())?;
    let swing = cells
        .iter()
        .zip(&shapes)
        .map(|((p, c), s)| {
            let changes = c.iter().zip(s).map(|(o, shape)| Change::Open { closed: *o, shape: Box::new(turn.apply(shape)) }).collect();
            (*p, changes)
        })
        .collect();
    Some(swing)
}

/// A hinge line: the axis it runs along, where it crosses the boxel (in doubled sixteenths
/// along the other two axes, so a middle between sixteenths is whole), and its boxels.
#[derive(Clone, Debug, PartialEq)]
pub struct HingeLine {
    pub axis: usize,
    pub center: [i32; 2],
    pub boxels: Vec<BoxelPos>,
}

/// The hinge line through `pos`, if a hinge piece is there: `pos`, then the boxels on from it
/// both ways along the axis whose hinge lies on the same line.
pub fn hinge_line(grid: &Grid, blocks: &BlockRegistry, pos: BoxelPos) -> Option<HingeLine> {
    let (axis, center) = hinge_at(grid, blocks, pos)?;
    let mut boxels = vec![pos];
    for side in [-1, 1] {
        let mut step = [0; 3];
        step[axis] = side;
        let mut at = pos;
        loop {
            at = at.offset(I64Vec3::from_array(step));
            if hinge_at(grid, blocks, at) != Some((axis, center)) || boxels.len() > 64 {
                break;
            }
            boxels.push(at);
        }
    }
    Some(HingeLine { axis, center, boxels })
}

/// The axis and crossing of the first hinge piece in the boxel at `pos`.
fn hinge_at(grid: &Grid, blocks: &BlockRegistry, pos: BoxelPos) -> Option<(usize, [i32; 2])> {
    let hinge = grid.cell(pos).iter().copied().find(|o| blocks.get(o.block).is_some_and(|d| d.has(Property::Hinge)))?;
    let (min, max) = bounds(&blocks.shape(hinge.block).turned(hinge.orientation))?;
    let size = [0, 1, 2].map(|i| max[i] - min[i]);
    let longest = *size.iter().max().expect("three axes");
    // Up when it's among the longest, like a door's hinge or a cube.
    let axis = if size[2] == longest { 2 } else { size.iter().position(|&s| s == longest).expect("one is longest") };
    let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
    Some((axis, [min[b] + max[b], min[c] + max[c]]))
}

/// The smallest box around a shape's filled sixteenths, as minimum and maximum (one past the
/// last), or `None` for an empty shape.
fn bounds(shape: &ShapeGrid) -> Option<([i32; 3], [i32; 3])> {
    let mut out: Option<([i32; 3], [i32; 3])> = None;
    for z in 0..N {
        for y in 0..N {
            for x in 0..N {
                if shape.get(x as usize, y as usize, z as usize) {
                    let p = [x, y, z];
                    let (min, max) = out.get_or_insert((p, p.map(|c| c + 1)));
                    for i in 0..3 {
                        min[i] = min[i].min(p[i]);
                        max[i] = max[i].max(p[i] + 1);
                    }
                }
            }
        }
    }
    out
}

/// A quarter turn about a hinge line, then a slide back inside the boxel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Turn {
    axis: usize,
    center: [i32; 2],
    /// 1 for counterclockwise seen from the axis's positive end, -1 for clockwise.
    way: i32,
    /// Sixteenths to slide along the two other axes after turning.
    slide: [i32; 2],
}

impl HingeLine {
    /// The turn for these shapes: the way that leaves less outside the boxel, counterclockwise
    /// on a tie, with the slide that brings it back in. `None` if a quarter turn about the
    /// line would leave the grid of sixteenths.
    fn turn<'a>(&self, shapes: impl Iterator<Item = &'a ShapeGrid> + Clone) -> Option<Turn> {
        if (self.center[0] + self.center[1]) % 2 != 0 {
            return None;
        }
        let mut best: Option<(i32, Turn)> = None;
        for way in [1, -1] {
            let mut turn = Turn { axis: self.axis, center: self.center, way, slide: [0, 0] };
            let mut range: Option<([i32; 2], [i32; 2])> = None;
            for shape in shapes.clone() {
                for_each(shape, |p| {
                    let q = turn.rotate(p);
                    let (b, c) = ((self.axis + 1) % 3, (self.axis + 2) % 3);
                    let (lo, hi) = range.get_or_insert(([q[b], q[c]], [q[b], q[c]]));
                    for (i, v) in [q[b], q[c]].into_iter().enumerate() {
                        lo[i] = lo[i].min(v);
                        hi[i] = hi[i].max(v);
                    }
                });
            }
            let (lo, hi) = range?;
            turn.slide = [0, 1].map(|i| if lo[i] < 0 { -lo[i] } else if hi[i] >= N { N - 1 - hi[i] } else { 0 });
            let out = turn.slide[0].abs() + turn.slide[1].abs();
            if best.as_ref().is_none_or(|(o, _)| out < *o) {
                best = Some((out, turn));
            }
        }
        best.map(|(_, t)| t)
    }
}

impl Turn {
    /// Where sixteenth `p` goes, before sliding.
    fn rotate(&self, p: [i32; 3]) -> [i32; 3] {
        let (b, c) = ((self.axis + 1) % 3, (self.axis + 2) % 3);
        // Doubled offsets from the line, so the line between sixteenths is whole.
        let (pb, pc) = (2 * p[b] + 1 - self.center[0], 2 * p[c] + 1 - self.center[1]);
        let (qb, qc) = if self.way > 0 { (-pc, pb) } else { (pc, -pb) };
        let mut out = p;
        out[b] = (self.center[0] + qb - 1).div_euclid(2);
        out[c] = (self.center[1] + qc - 1).div_euclid(2);
        out
    }

    /// The shape turned and slid back inside.
    pub fn apply(&self, shape: &ShapeGrid) -> ShapeGrid {
        let (b, c) = ((self.axis + 1) % 3, (self.axis + 2) % 3);
        let mut out = ShapeGrid::EMPTY;
        for_each(shape, |p| {
            let mut q = self.rotate(p);
            q[b] += self.slide[0];
            q[c] += self.slide[1];
            if q.iter().all(|v| (0..N).contains(v)) {
                out.set(q[0] as usize, q[1] as usize, q[2] as usize, true);
            }
        });
        out
    }
}

/// Calls `f` with every filled sixteenth of `shape`.
fn for_each(shape: &ShapeGrid, mut f: impl FnMut([i32; 3])) {
    for z in 0..N {
        for y in 0..N {
            for x in 0..N {
                if shape.get(x as usize, y as usize, z as usize) {
                    f([x, y, z]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MaterialId;
    use api::{BlockBox, BlockDef};

    fn grid(boxes: &[([u8; 3], [u8; 3])]) -> ShapeGrid {
        ShapeGrid::from_boxes(&boxes.iter().map(|&(a, b)| BlockBox::new(a, b)).collect::<Vec<_>>())
    }

    /// A door's lower half seen from above: a panel along the north side, the hinge its west
    /// quarter, a knob sticking out south at the east end.
    fn door() -> (ShapeGrid, ShapeGrid, ShapeGrid) {
        let panel = grid(&[([4, 12, 0], [16, 16, 16])]);
        let hinge = grid(&[([0, 12, 0], [4, 16, 16])]);
        let knob = grid(&[([12, 8, 8], [16, 12, 12])]);
        (panel, hinge, knob)
    }

    fn line(hinge: &ShapeGrid) -> HingeLine {
        let (min, max) = bounds(hinge).unwrap();
        HingeLine { axis: 2, center: [min[0] + max[0], min[1] + max[1]], boxels: vec![] }
    }

    #[test]
    fn a_door_swings_toward_its_knob_and_stays_inside() {
        let (panel, hinge, knob) = door();
        let all = [panel.clone(), hinge.clone(), knob.clone()];
        let turn = line(&hinge).turn(all.iter()).unwrap();
        assert_eq!(turn.way, -1, "clockwise: the free end swings south, toward the knob");
        let (p, h, k) = (turn.apply(&panel), turn.apply(&hinge), turn.apply(&knob));
        assert_eq!(p.volume() + h.volume() + k.volume(), panel.volume() + hinge.volume() + knob.volume(), "nothing lost");
        // Open, the door runs along the west side, slid in by the knob's depth.
        assert_eq!(h, grid(&[([4, 12, 0], [8, 16, 16])]));
        assert_eq!(p, grid(&[([4, 0, 0], [8, 12, 16])]));
        assert_eq!(k, grid(&[([0, 0, 8], [4, 4, 12])]), "the knob on the panel's west face");
    }

    #[test]
    fn a_plain_door_turns_about_its_hinge_without_sliding() {
        let (panel, hinge, _) = door();
        let turn = line(&hinge).turn([panel.clone(), hinge.clone()].iter()).unwrap();
        assert_eq!(turn.slide, [0, 0]);
        assert_eq!(turn.apply(&hinge), hinge, "the hinge stays put");
        assert_eq!(turn.apply(&panel), grid(&[([0, 0, 0], [4, 12, 16])]));
    }

    #[test]
    fn a_trapdoor_hinge_lying_along_x_turns_about_x() {
        let mut blocks = BlockRegistry::default();
        blocks.register(BlockDef::new("t:iron", true, [0; 3]).with_material("t:iron").with_properties(vec![Property::Hinge])).unwrap();
        let iron = blocks.material("t:iron").unwrap();
        // A strip along the north edge of the floor.
        let strip = blocks.shaped(iron, &grid(&[([0, 12, 0], [16, 16, 4])])).unwrap();
        let mut g = Grid::default();
        g.set_cell(BoxelPos::new(0, 0, 0), &[Occupant::new(strip)]);
        let line = hinge_line(&g, &blocks, BoxelPos::new(0, 0, 0)).unwrap();
        assert_eq!((line.axis, line.center), (0, [28, 4]));
    }

    /// A registry with an iron hinge material, a copper listener, and wood.
    fn metals() -> (BlockRegistry, MaterialId, MaterialId, MaterialId) {
        let mut blocks = BlockRegistry::default();
        let def = |n: &str, p: Vec<Property>| BlockDef::new(format!("t:{n}"), true, [0; 3]).with_material(format!("t:{n}")).with_properties(p);
        blocks.register(def("iron", vec![Property::Hinge])).unwrap();
        blocks.register(def("copper", vec![Property::Listener])).unwrap();
        blocks.register(def("wood", vec![])).unwrap();
        let m = |n: &str| blocks.material(&format!("t:{n}")).unwrap();
        let (iron, copper, wood) = (m("iron"), m("copper"), m("wood"));
        (blocks, iron, copper, wood)
    }

    #[test]
    fn a_hinge_line_runs_through_hinges_on_the_same_line_only() {
        let (mut blocks, iron, _, wood) = metals();
        let (panel, hinge, _) = door();
        let (panel, hinge) = (blocks.shaped(wood, &panel).unwrap(), blocks.shaped(iron, &hinge).unwrap());
        // The other corner: a door hinged on the east.
        let east = blocks.shaped(iron, &grid(&[([12, 12, 0], [16, 16, 16])])).unwrap();
        let mut g = Grid::default();
        let door_at = |g: &mut Grid, p: BoxelPos, h| g.set_cell(p, &[Occupant::new(panel), Occupant::new(h)]);
        door_at(&mut g, BoxelPos::new(0, 0, 0), hinge);
        door_at(&mut g, BoxelPos::new(0, 0, 1), hinge);
        door_at(&mut g, BoxelPos::new(0, 0, 2), east);
        // A double door beside it, its hinge the mirror of this one's.
        door_at(&mut g, BoxelPos::new(1, 0, 0), east);
        let line = hinge_line(&g, &blocks, BoxelPos::new(0, 0, 1)).unwrap();
        assert_eq!(line.boxels, [BoxelPos::new(0, 0, 1), BoxelPos::new(0, 0, 0)], "two high, not the third");
        let swing = swing(&g, &blocks, BoxelPos::new(0, 0, 0)).unwrap();
        assert_eq!(swing.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [BoxelPos::new(0, 0, 0), BoxelPos::new(0, 0, 1)]);
        assert!(swing.iter().all(|(_, c)| c.iter().all(|c| matches!(c, Change::Open { .. }))));
    }

    #[test]
    fn a_trigger_reaches_its_boxel_and_touching_listeners() {
        let (mut blocks, _, copper, wood) = metals();
        let plate = blocks.shaped(copper, &grid(&[([0, 0, 0], [4, 4, 4])])).unwrap();
        let plank = blocks.shaped(wood, &grid(&[([0, 0, 0], [4, 4, 4])])).unwrap();
        let mut g = Grid::default();
        let at = BoxelPos::new(5, 5, 5);
        g.set_cell(BoxelPos::new(6, 5, 5), &[Occupant::new(plate)]);
        g.set_cell(BoxelPos::new(5, 4, 5), &[Occupant::new(plank)]);
        g.set_cell(BoxelPos::new(6, 6, 5), &[Occupant::new(plate)]);
        assert_eq!(reached(&g, &blocks, at), [at, BoxelPos::new(6, 5, 5)], "not the wood, nor a diagonal");
    }

    #[test]
    fn a_hinge_off_the_sixteenth_grid_does_not_swing() {
        let odd = HingeLine { axis: 2, center: [3, 28], boxels: vec![] };
        assert_eq!(odd.turn([ShapeGrid::FULL].iter()), None);
    }
}
