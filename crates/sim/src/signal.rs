//! Signals and what answers them: properties that come from materials (see [`api::Property`]).
//!
//! **Reaching.** Pressing interact on a trigger piece signals its boxel; the signal goes on
//! into every face-touching boxel with a shown conductor piece, and on from those, and every
//! boxel reached is signaled (see [`reached`]).
//!
//! **Phase groups.** A phase piece's group is every piece of its material joined to it face to
//! face, across boxels too (see [`group`]). A signal flips each group with a piece in a
//! signaled boxel: a group with anything shown hides, otherwise it shows. Shows go first, so a
//! door's closed panel and its open panel, of two phase materials, swap on one signal, and
//! what's hidden is always what's within reach of a hidden group.
//!
//! **Reach.** Hiding a group takes everything else within reach of it too, sixteenth by
//! sixteenth, edges and corners counting, other phase pieces excepted: a piece partly within
//! reach is cut, its kept part and its hidden part each a derived kind remembering the piece
//! (see [`crate::Derived`]). Showing a group brings back everything hidden within reach of it,
//! and the parts of one piece join up again into exactly that piece. A hidden piece stays
//! hidden while something solid or a body is in its place.

use std::collections::{HashMap, HashSet, VecDeque};

use api::{BoxelPos, I64Vec3, Property, ShapeGrid};

use crate::{BlockRegistry, Grid, MAX_OCCUPANTS, Occupant};

/// Sixteenths across a boxel.
const N: i32 = 16;

/// The most pieces one phase group holds.
const MAX_GROUP: usize = 4096;

const FACES: [I64Vec3; 6] = [I64Vec3::X, I64Vec3::NEG_X, I64Vec3::Y, I64Vec3::NEG_Y, I64Vec3::Z, I64Vec3::NEG_Z];

/// The boxels a signal starting at `start` reaches: `start`, then every boxel joined to it
/// through face-touching boxels with a shown conductor piece, at most `limit` in all.
pub fn reached(grid: &Grid, blocks: &BlockRegistry, start: BoxelPos, limit: u32) -> Vec<BoxelPos> {
    let mut out = vec![start];
    let mut seen = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(p) = queue.pop_front() {
        for d in FACES {
            if out.len() >= limit as usize {
                return out;
            }
            let q = p.offset(d);
            if seen.insert(q) && has(grid, blocks, q, Property::Conductor) {
                out.push(q);
                queue.push_back(q);
            }
        }
    }
    out
}

/// Whether a shown piece in the boxel at `pos` has `property`.
pub fn has(grid: &Grid, blocks: &BlockRegistry, pos: BoxelPos, property: Property) -> bool {
    grid.cell(pos).iter().any(|&o| !blocks.is_hidden(o.block) && has_property(blocks, o, property))
}

fn has_property(blocks: &BlockRegistry, o: Occupant, property: Property) -> bool {
    blocks.get(o.block).is_some_and(|d| d.has(property))
}

/// One occupant of one boxel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Piece {
    pub pos: BoxelPos,
    pub occupant: Occupant,
}

/// The sixteenths a piece fills in its boxel.
fn texels(blocks: &BlockRegistry, o: Occupant) -> ShapeGrid {
    blocks.origin(o).1
}

/// The phase group of `start`: every phase piece of the same material, shown or hidden, joined
/// to it through pieces touching face to face, across boxels too. `start` first.
pub fn group(grid: &Grid, blocks: &BlockRegistry, start: Piece) -> Vec<Piece> {
    let material = blocks.made_of(start.occupant.block);
    let mut out = vec![start];
    let mut seen = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(p) = queue.pop_front() {
        let a = texels(blocks, p.occupant);
        let sides = std::iter::once((p.pos, None)).chain(FACES.iter().map(|&d| (p.pos.offset(d), Some(d))));
        for (pos, side) in sides {
            for &o in grid.cell(pos) {
                let q = Piece { pos, occupant: o };
                if out.len() >= MAX_GROUP || seen.contains(&q) {
                    continue;
                }
                if !has_property(blocks, o, Property::Phase) || blocks.made_of(o.block) != material {
                    continue;
                }
                if touches(&a, &texels(blocks, o), side) {
                    seen.insert(q);
                    out.push(q);
                    queue.push_back(q);
                }
            }
        }
    }
    out
}

/// Whether shape `a` touches shape `b` face to face (or overlaps it): in one boxel, or with `b`
/// in the boxel one step `side` from `a`'s.
fn touches(a: &ShapeGrid, b: &ShapeGrid, side: Option<I64Vec3>) -> bool {
    let Some(side) = side else {
        return a.overlaps(b) || shifted_faces(a).overlaps(b);
    };
    let axis = (0..3).find(|&i| side[i] != 0).expect("a face direction");
    let (from, to) = if side[axis] > 0 { (N - 1, 0) } else { (0, N - 1) };
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    for i in 0..N {
        for j in 0..N {
            let mut p = [0; 3];
            p[u] = i;
            p[v] = j;
            let (mut pa, mut pb) = (p, p);
            pa[axis] = from;
            pb[axis] = to;
            if get(a, pa) && get(b, pb) {
                return true;
            }
        }
    }
    false
}

/// Every sixteenth face-touching a filled one of `shape`, within its boxel.
fn shifted_faces(shape: &ShapeGrid) -> ShapeGrid {
    let mut out = ShapeGrid::EMPTY;
    for_each(shape, |p| {
        for axis in 0..3 {
            for d in [-1, 1] {
                let mut q = p;
                q[axis] += d;
                if (0..N).contains(&q[axis]) {
                    set(&mut out, q);
                }
            }
        }
    });
    out
}

fn get(shape: &ShapeGrid, p: [i32; 3]) -> bool {
    shape.get(p[0] as usize, p[1] as usize, p[2] as usize)
}

fn set(shape: &mut ShapeGrid, p: [i32; 3]) {
    shape.set(p[0] as usize, p[1] as usize, p[2] as usize, true);
}

/// Calls `f` with every filled sixteenth of `shape`.
fn for_each(shape: &ShapeGrid, mut f: impl FnMut([i32; 3])) {
    for z in 0..N {
        for y in 0..N {
            for x in 0..N {
                if get(shape, [x, y, z]) {
                    f([x, y, z]);
                }
            }
        }
    }
}

/// Sixteenths spread over boxels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Texels(HashMap<BoxelPos, ShapeGrid>);

impl Texels {
    /// Adds `shape`'s sixteenths in the boxel at `pos`.
    pub fn add(&mut self, pos: BoxelPos, shape: &ShapeGrid) {
        let entry = self.0.entry(pos).or_default();
        *entry = entry.union(shape);
    }

    /// The sixteenths in the boxel at `pos`.
    pub fn get(&self, pos: BoxelPos) -> ShapeGrid {
        self.0.get(&pos).cloned().unwrap_or_default()
    }

    /// Every boxel with any sixteenths, and them.
    pub fn iter(&self) -> impl Iterator<Item = (BoxelPos, &ShapeGrid)> {
        self.0.iter().filter(|(_, s)| !s.is_empty()).map(|(p, s)| (*p, s))
    }

    /// Every sixteenth within `r` of one of these, edges and corners counting: grown by `r`
    /// along each axis in turn, so a cube around each.
    pub fn grown(&self, r: i32) -> Texels {
        let mut out = self.clone();
        for axis in 0..3 {
            out = out.grown_along(axis, r);
        }
        out
    }

    fn grown_along(&self, axis: usize, r: i32) -> Texels {
        if r <= 0 {
            return self.clone();
        }
        let mut out = Texels::default();
        for (&pos, shape) in &self.0 {
            for_each(shape, |p| {
                for k in -r..=r {
                    let mut q = p;
                    q[axis] += k;
                    let mut step = [0; 3];
                    step[axis] = q[axis].div_euclid(N) as i64;
                    q[axis] = q[axis].rem_euclid(N);
                    let at = pos.offset(I64Vec3::from_array(step));
                    set(out.0.entry(at).or_default(), q);
                }
            });
        }
        out
    }
}

/// Answers a signal to the boxels in `reached`: flips every phase group with a piece in one,
/// the showing ones first (see the module). `reach` is in sixteenths; `clear` says whether
/// bodies leave room for solid sixteenths `shape` in the boxel at a position. Returns the
/// boxels that changed.
pub fn signal(
    grid: &mut Grid,
    blocks: &mut BlockRegistry,
    reached: &[BoxelPos],
    reach: i32,
    clear: &dyn Fn(BoxelPos, &ShapeGrid) -> bool,
) -> Vec<BoxelPos> {
    let mut groups: Vec<Vec<Piece>> = Vec::new();
    let mut taken: HashSet<Piece> = HashSet::new();
    for &pos in reached {
        for &o in grid.cell(pos) {
            let piece = Piece { pos, occupant: o };
            if has_property(blocks, o, Property::Phase) && !taken.contains(&piece) {
                let g = group(grid, blocks, piece);
                taken.extend(g.iter().copied());
                groups.push(g);
            }
        }
    }
    let (hides, shows): (Vec<_>, Vec<_>) =
        groups.into_iter().partition(|g| g.iter().any(|p| !blocks.is_hidden(p.occupant.block)));
    let mut changed = Vec::new();
    for g in &shows {
        changed.extend(show(grid, blocks, g, reach, clear));
    }
    for g in &hides {
        changed.extend(hide(grid, blocks, g, reach));
    }
    changed.sort_by_key(|p| p.0.to_array());
    changed.dedup();
    changed
}

/// Hides the shown ones of `pieces`, whole, and every other shown piece's sixteenths within
/// `reach` of them, phase pieces excepted. Returns the boxels that changed.
pub fn hide(grid: &mut Grid, blocks: &mut BlockRegistry, pieces: &[Piece], reach: i32) -> Vec<BoxelPos> {
    let mut area = Texels::default();
    let mut changed = Vec::new();
    let shown: Vec<Piece> = pieces.iter().copied().filter(|p| !blocks.is_hidden(p.occupant.block)).collect();
    for p in shown {
        let shape = texels(blocks, p.occupant);
        area.add(p.pos, &shape);
        if hide_part(grid, blocks, p.pos, p.occupant, &shape) {
            changed.push(p.pos);
        }
    }
    let within: Vec<(BoxelPos, ShapeGrid)> = area.grown(reach).iter().map(|(p, s)| (p, s.clone())).collect();
    for (pos, mask) in within {
        let shown: Vec<Occupant> = grid
            .cell(pos)
            .iter()
            .copied()
            .filter(|&o| !blocks.is_hidden(o.block) && !has_property(blocks, o, Property::Phase))
            .collect();
        for o in shown {
            if hide_part(grid, blocks, pos, o, &mask) {
                changed.push(pos);
            }
        }
    }
    changed
}

/// Shows the hidden ones of `pieces`, whole, and every other hidden piece's sixteenths within
/// `reach` of them, phase pieces excepted, each only where `clear` and the boxel's shown solid
/// pieces leave room. Returns the boxels that changed.
pub fn show(
    grid: &mut Grid,
    blocks: &mut BlockRegistry,
    pieces: &[Piece],
    reach: i32,
    clear: &dyn Fn(BoxelPos, &ShapeGrid) -> bool,
) -> Vec<BoxelPos> {
    let mut area = Texels::default();
    let mut changed = Vec::new();
    for p in pieces {
        area.add(p.pos, &texels(blocks, p.occupant));
    }
    let hidden: Vec<Piece> = pieces.iter().copied().filter(|p| blocks.is_hidden(p.occupant.block)).collect();
    for p in hidden {
        if show_part(grid, blocks, p.pos, p.occupant, &ShapeGrid::FULL, clear) {
            changed.push(p.pos);
        }
    }
    let within: Vec<(BoxelPos, ShapeGrid)> = area.grown(reach).iter().map(|(p, s)| (p, s.clone())).collect();
    for (pos, mask) in within {
        let hidden: Vec<Occupant> = grid
            .cell(pos)
            .iter()
            .copied()
            .filter(|&o| blocks.is_hidden(o.block) && !has_property(blocks, o, Property::Phase))
            .filter(|&o| texels(blocks, o).overlaps(&mask))
            .collect();
        for o in hidden {
            if show_part(grid, blocks, pos, o, &mask, clear) {
                changed.push(pos);
            }
        }
    }
    changed
}

/// Hides the sixteenths of shown occupant `o` at `pos` inside `mask`: the piece becomes a
/// hidden part and, if any is left outside, a kept part. False if `o` isn't there, nothing of
/// it is inside, or the boxel has no room for both parts.
fn hide_part(grid: &mut Grid, blocks: &mut BlockRegistry, pos: BoxelPos, o: Occupant, mask: &ShapeGrid) -> bool {
    let cell = grid.cell(pos).to_vec();
    let Some(i) = cell.iter().position(|&c| c == o) else {
        return false;
    };
    let (from, shape) = blocks.origin(o);
    let hidden = shape.intersection(mask);
    if hidden.is_empty() {
        return false;
    }
    let kept = shape.minus(mask);
    let mut new = cell;
    new.remove(i);
    let Some(h) = part(blocks, from, &hidden, true) else {
        return false;
    };
    new.insert(i, h);
    if !kept.is_empty() {
        let Some(k) = part(blocks, from, &kept, false) else {
            return false;
        };
        new.insert(i, k);
    }
    join(&mut new, blocks);
    if new.len() > MAX_OCCUPANTS {
        return false;
    }
    grid.set_cell(pos, &new);
    true
}

/// Shows the sixteenths of hidden occupant `o` at `pos` inside `mask`, joining them up with
/// the rest of their piece; any outside stay hidden. False if `o` isn't there, nothing of it
/// is inside, the boxel has no room, or it's solid and a shown solid piece or, by `clear`, a
/// body is in its place.
fn show_part(
    grid: &mut Grid,
    blocks: &mut BlockRegistry,
    pos: BoxelPos,
    o: Occupant,
    mask: &ShapeGrid,
    clear: &dyn Fn(BoxelPos, &ShapeGrid) -> bool,
) -> bool {
    let cell = grid.cell(pos).to_vec();
    let Some(i) = cell.iter().position(|&c| c == o) else {
        return false;
    };
    let Some(d) = blocks.derived(o.block).cloned() else {
        return false;
    };
    let back = d.shape.intersection(mask);
    if back.is_empty() {
        return false;
    }
    if blocks.is_solid(d.from.block) {
        let blocked = cell.iter().any(|&c| c != o && blocks.is_solid(c.block) && texels(blocks, c).overlaps(&back));
        if blocked || !clear(pos, &back) {
            return false;
        }
    }
    let Some(shown) = part(blocks, d.from, &back, false) else {
        return false;
    };
    let mut new = cell;
    new[i] = shown;
    let still = d.shape.minus(mask);
    if !still.is_empty() {
        let Some(h) = part(blocks, d.from, &still, true) else {
            return false;
        };
        new.insert(i + 1, h);
    }
    join(&mut new, blocks);
    if new.len() > MAX_OCCUPANTS {
        return false;
    }
    grid.set_cell(pos, &new);
    true
}

/// The occupant for the part of `from` filling `shape`: `from` itself when shown and whole.
fn part(blocks: &mut BlockRegistry, from: Occupant, shape: &ShapeGrid, hidden: bool) -> Option<Occupant> {
    if !hidden && *shape == blocks.shape(from.block).turned(from.orientation) {
        return Some(from);
    }
    blocks.derive(from, shape, hidden).ok().map(Occupant::new)
}

/// Joins the parts of one piece in a boxel: shown parts of the same piece become one (the
/// piece itself once whole), and hidden parts likewise, each where the first of them was.
fn join(cell: &mut Vec<Occupant>, blocks: &mut BlockRegistry) {
    let mut out: Vec<Occupant> = Vec::with_capacity(cell.len());
    for o in cell.drain(..) {
        let Some(d) = blocks.derived(o.block).cloned() else {
            out.push(o);
            continue;
        };
        let same = out.iter().position(|e| blocks.derived(e.block).is_some_and(|e| e.from == d.from && e.hidden == d.hidden));
        match same {
            Some(j) => {
                let joined = blocks.derived(out[j].block).expect("derived").shape.union(&d.shape);
                out[j] = part(blocks, d.from, &joined, d.hidden).unwrap_or(out[j]);
            }
            None => out.push(o),
        }
    }
    *cell = out;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BlockId;
    use api::{BlockBox, BlockDef};

    fn grid(boxes: &[([u8; 3], [u8; 3])]) -> ShapeGrid {
        ShapeGrid::from_boxes(&boxes.iter().map(|&(a, b)| BlockBox::new(a, b)).collect::<Vec<_>>())
    }

    /// Phasite, antiphasite, copper, brass and brick.
    struct Kit {
        blocks: BlockRegistry,
        phasite: BlockId,
        antiphasite: BlockId,
        copper: BlockId,
        brick: BlockId,
    }

    fn kit() -> Kit {
        let mut blocks = BlockRegistry::default();
        let mut def = |n: &str, p: Vec<Property>| {
            blocks.register(BlockDef::new(format!("t:{n}"), true, [0; 3]).with_material(format!("t:{n}")).with_properties(p)).unwrap()
        };
        let phasite = def("phasite", vec![Property::Phase]);
        let antiphasite = def("antiphasite", vec![Property::Phase, Property::StartsHidden]);
        let copper = def("copper", vec![Property::Conductor]);
        def("brass", vec![Property::Trigger]);
        let brick = def("brick", vec![]);
        Kit { blocks, phasite, antiphasite, copper, brick }
    }

    impl Kit {
        fn shaped(&mut self, block: BlockId, shape: &ShapeGrid) -> Occupant {
            let m = self.blocks.made_of(block).unwrap();
            Occupant::new(self.blocks.shaped(m, shape).unwrap())
        }
    }

    fn at(x: i64, y: i64, z: i64) -> BoxelPos {
        BoxelPos::new(x, y, z)
    }

    fn always(_: BoxelPos, _: &ShapeGrid) -> bool {
        true
    }

    #[test]
    fn a_signal_runs_through_touching_copper_only() {
        let mut k = kit();
        let wire = k.shaped(k.copper, &grid(&[([6, 6, 0], [10, 10, 16])]));
        let mut g = Grid::default();
        for z in 1..4 {
            g.set_cell(at(0, 0, z), &[wire]);
        }
        g.set_cell(at(1, 1, 1), &[wire]); // only a diagonal
        g.set_cell(at(0, 1, 0), &[Occupant::new(k.brick)]);
        let r = reached(&g, &k.blocks, at(0, 0, 0), 100);
        assert_eq!(r, [at(0, 0, 0), at(0, 0, 1), at(0, 0, 2), at(0, 0, 3)]);
        assert_eq!(reached(&g, &k.blocks, at(0, 0, 0), 2).len(), 2, "capped");
    }

    #[test]
    fn growing_reaches_across_boxels_edges_and_corners_included() {
        let mut t = Texels::default();
        t.add(at(0, 0, 0), &grid(&[([15, 15, 15], [16, 16, 16])]));
        let g = t.grown(1);
        assert!(g.get(at(1, 1, 1)).get(0, 0, 0), "the corner across");
        assert!(g.get(at(0, 0, 0)).get(14, 14, 14));
        assert_eq!(g.get(at(0, 0, 0)).volume(), 8);
        assert_eq!(t.grown(0), t);
    }

    #[test]
    fn a_group_runs_through_face_touching_pieces_of_one_material() {
        let mut k = kit();
        let panel = k.shaped(k.phasite, &grid(&[([0, 12, 0], [16, 16, 16])]));
        let other = k.shaped(k.antiphasite, &grid(&[([0, 0, 0], [4, 12, 16])]));
        let mut g = Grid::default();
        g.set_cell(at(0, 0, 0), &[panel, other]);
        g.set_cell(at(0, 0, 1), &[panel]);
        g.set_cell(at(0, 0, 3), &[panel]); // a gap of a boxel
        let found = group(&g, &k.blocks, Piece { pos: at(0, 0, 0), occupant: panel });
        assert_eq!(found.len(), 2, "both halves, not the antiphasite touching it nor the far one");
    }

    #[test]
    fn hiding_takes_what_touches_and_showing_puts_it_back_exactly() {
        let mut k = kit();
        // A thin panel standing in the middle of a boxel, a brick on each side.
        let panel = k.shaped(k.phasite, &grid(&[([0, 7, 0], [16, 9, 16])]));
        let mut g = Grid::default();
        g.set_cell(at(0, 0, 0), &[panel]);
        g.set_cell(at(-1, 0, 0), &[Occupant::new(k.brick)]);
        g.set_cell(at(0, 0, -1), &[Occupant::new(k.brick)]);
        let cells = |g: &Grid| [at(0, 0, 0), at(-1, 0, 0), at(0, 0, -1)].map(|p| g.cell(p).to_vec());
        let before = cells(&g);

        let changed = signal(&mut g, &mut k.blocks, &[at(0, 0, 0)], 1, &always);
        assert_eq!(changed.len(), 3);
        assert!(g.cell(at(0, 0, 0)).iter().all(|o| k.blocks.is_hidden(o.block)), "the panel hid");
        // The west brick lost a strip one sixteenth deep, beside the panel and its corners.
        let west_now = g.cell(at(-1, 0, 0)).to_vec();
        assert_eq!(west_now.len(), 2, "a kept part and a hidden part");
        let hidden = west_now.iter().find(|o| k.blocks.is_hidden(o.block)).unwrap();
        assert_eq!(k.blocks.derived(hidden.block).unwrap().shape, grid(&[([15, 6, 0], [16, 10, 16])]));
        assert!(west_now.iter().any(|o| k.blocks.is_solid(o.block)), "the rest stays solid");

        signal(&mut g, &mut k.blocks, &[at(0, 0, 0)], 1, &always);
        assert_eq!(cells(&g), before, "exactly as before");
    }

    #[test]
    fn a_door_of_two_phase_materials_swaps_panels() {
        let mut k = kit();
        let closed = k.shaped(k.phasite, &grid(&[([0, 12, 0], [16, 16, 16])]));
        let open = k.shaped(k.antiphasite, &grid(&[([0, 0, 0], [4, 12, 16])]));
        let mut g = Grid::default();
        g.set_cell(at(0, 0, 0), &[closed, open]);
        hide(&mut g, &mut k.blocks, &[Piece { pos: at(0, 0, 0), occupant: open }], 1);
        let shut = g.cell(at(0, 0, 0)).to_vec();
        assert_eq!(shut[0], closed);
        assert!(k.blocks.is_hidden(shut[1].block), "placed hidden");

        signal(&mut g, &mut k.blocks, &[at(0, 0, 0)], 1, &always);
        let swung = g.cell(at(0, 0, 0)).to_vec();
        assert!(k.blocks.is_hidden(swung[0].block));
        assert_eq!(swung[1], open);

        signal(&mut g, &mut k.blocks, &[at(0, 0, 0)], 1, &always);
        assert_eq!(g.cell(at(0, 0, 0)), shut, "and back");
    }

    #[test]
    fn a_piece_stays_hidden_while_something_is_in_its_place() {
        let mut k = kit();
        let panel = k.shaped(k.phasite, &grid(&[([0, 12, 0], [16, 16, 16])]));
        let mut g = Grid::default();
        g.set_cell(at(0, 0, 0), &[panel]);
        signal(&mut g, &mut k.blocks, &[at(0, 0, 0)], 1, &always);
        signal(&mut g, &mut k.blocks, &[at(0, 0, 0)], 1, &|_, _| false);
        assert!(k.blocks.is_hidden(g.cell(at(0, 0, 0))[0].block), "a body in the way");
        // A slab put where it was, while it was hidden.
        let slab = k.shaped(k.brick, &grid(&[([0, 8, 0], [16, 16, 8])]));
        g.set_cell(at(0, 0, 0), &[g.cell(at(0, 0, 0))[0], slab]);
        signal(&mut g, &mut k.blocks, &[at(0, 0, 0)], 1, &always);
        assert!(k.blocks.is_hidden(g.cell(at(0, 0, 0))[0].block), "a solid in the way");
    }

    #[test]
    fn two_doors_on_one_wire_flip_together() {
        let mut k = kit();
        let panel = k.shaped(k.phasite, &grid(&[([0, 12, 0], [16, 16, 16])]));
        let wire = k.shaped(k.copper, &grid(&[([0, 0, 0], [16, 4, 4])]));
        let mut g = Grid::default();
        g.set_cell(at(0, 0, 0), &[panel, wire]);
        for x in 1..5 {
            g.set_cell(at(x, 0, 0), &[wire]);
        }
        g.set_cell(at(5, 0, 0), &[panel, wire]);
        let r = reached(&g, &k.blocks, at(-1, 0, 0), 100);
        signal(&mut g, &mut k.blocks, &r, 0, &always);
        assert!(k.blocks.is_hidden(g.cell(at(0, 0, 0))[0].block));
        assert!(k.blocks.is_hidden(g.cell(at(5, 0, 0))[0].block), "the far door too");
    }

    #[test]
    fn reach_zero_takes_nothing_else() {
        let mut k = kit();
        let panel = k.shaped(k.phasite, &grid(&[([0, 12, 0], [16, 16, 16])]));
        let mut g = Grid::default();
        g.set_cell(at(0, 0, 0), &[panel]);
        g.set_cell(at(0, 1, 0), &[Occupant::new(k.brick)]);
        assert_eq!(signal(&mut g, &mut k.blocks, &[at(0, 0, 0)], 0, &always), [at(0, 0, 0)]);
    }

    #[test]
    fn phase_pieces_are_never_taken_by_reach() {
        let mut k = kit();
        let a = k.shaped(k.phasite, &grid(&[([0, 0, 0], [16, 4, 16])]));
        let b = k.shaped(k.antiphasite, &grid(&[([0, 4, 0], [16, 8, 16])]));
        let mut g = Grid::default();
        g.set_cell(at(0, 0, 0), &[a, b]);
        hide(&mut g, &mut k.blocks, &[Piece { pos: at(0, 0, 0), occupant: a }], 2);
        assert_eq!(g.cell(at(0, 0, 0))[1], b);
    }
}
