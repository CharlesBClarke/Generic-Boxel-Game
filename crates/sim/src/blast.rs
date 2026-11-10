//! Crystal blasts (see [`api::Property::Fuse`] and [`api::Property::Lens`]).
//!
//! **Setting off.** A shown fuse piece touching a shown lens piece face to face, texel by
//! texel, in one boxel or across a boxel's face, sets both off: both are used up, and the lens
//! fires one beam for each direction it's touched in, the way from the fuse into the lens.
//! Only boxels that just changed are looked at (see [`reactions`]), so a pair built at load
//! time waits until something near it changes.
//!
//! **The beam.** It starts in the boxel past the lens's and runs straight, one boxel wide,
//! breaking every shown piece in its way until its strength (in full blocks' worth) is spent or
//! it reaches its range (see [`beam`]). Hidden pieces are passed through. A beam reaching a lens
//! piece stops there and sets that lens off, firing on the same way at full strength. Bodies
//! in its path are hit by the simulation.

use std::collections::HashSet;

use api::{BlastDef, BoxelPos, I64Vec3, Property, ShapeGrid, Vec3, WorldPos};

use crate::signal::Piece;
use crate::{BlockId, BlockRegistry, FULL_VOLUME, Grid, Occupant};

/// Texels across a boxel.
const N: i32 = 16;

const FACES: [I64Vec3; 6] = [I64Vec3::X, I64Vec3::NEG_X, I64Vec3::Y, I64Vec3::NEG_Y, I64Vec3::Z, I64Vec3::NEG_Z];

/// The most beams one tick fires, chains included.
pub const MAX_BEAMS: usize = 256;

/// A beam fired, for drawing and hearing it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blast {
    pub tick: u64,
    /// Where it leaves the lens's boxel: the middle of the face it goes out through.
    pub from: WorldPos,
    /// Its way, a unit vector along one axis.
    pub dir: Vec3,
    /// How far it ran, in meters.
    pub length: f32,
    /// Pieces it broke.
    pub broke: u32,
    /// The lens that fired it, as it was.
    pub lens: BlockId,
}

/// A lens firing: the boxel it was in, and the way its beam goes, a unit step along one axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Firing {
    pub lens: BoxelPos,
    pub dir: I64Vec3,
    /// What the lens was, for how it sounds.
    pub block: BlockId,
}

/// What touching crystals set off: the pieces used up, and the beams fired.
#[derive(Debug, Default, PartialEq)]
pub struct Reaction {
    pub used: Vec<Piece>,
    pub firings: Vec<Firing>,
}

impl Reaction {
    pub fn is_empty(&self) -> bool {
        self.used.is_empty() && self.firings.is_empty()
    }
}

fn has(blocks: &BlockRegistry, o: Occupant, property: Property) -> bool {
    !blocks.is_hidden(o.block) && blocks.get(o.block).is_some_and(|d| d.has(property))
}

/// The texels a piece fills in its boxel.
fn texels(blocks: &BlockRegistry, o: Occupant) -> ShapeGrid {
    blocks.origin(o).1
}

/// The directions `a` (in the boxel at `a_pos`) touches `b` (at `b_pos`) face to face: each
/// unit step that moves one of `a`'s texels onto one of `b`'s.
pub fn touch_directions(a_pos: BoxelPos, a: &ShapeGrid, b_pos: BoxelPos, b: &ShapeGrid) -> Vec<I64Vec3> {
    let off = (b_pos.0 - a_pos.0) * N as i64;
    let mut out = Vec::new();
    for d in FACES {
        let mut found = false;
        'texels: for z in 0..N {
            for y in 0..N {
                for x in 0..N {
                    if !a.get(x as usize, y as usize, z as usize) {
                        continue;
                    }
                    let q = [x as i64 + d.x - off.x, y as i64 + d.y - off.y, z as i64 + d.z - off.z];
                    if q.iter().all(|&c| (0..N as i64).contains(&c)) && b.get(q[0] as usize, q[1] as usize, q[2] as usize) {
                        found = true;
                        break 'texels;
                    }
                }
            }
        }
        if found {
            out.push(d);
        }
    }
    out
}

/// Every fuse and lens touching in or beside the boxels `near`: which pieces are used up and
/// which beams fire. A lens fires once for each direction any fuse touches it in.
pub fn reactions(grid: &Grid, blocks: &BlockRegistry, near: &[BoxelPos]) -> Reaction {
    let mut boxels: Vec<BoxelPos> = Vec::new();
    let mut seen = HashSet::new();
    for &p in near {
        for q in std::iter::once(p).chain(FACES.iter().map(|&d| p.offset(d))) {
            if seen.insert(q) {
                boxels.push(q);
            }
        }
    }
    let mut out = Reaction::default();
    let mut used = HashSet::new();
    let mut fired = HashSet::new();
    for &p in &boxels {
        for &fuse in grid.cell(p) {
            if !has(blocks, fuse, Property::Fuse) {
                continue;
            }
            let a = texels(blocks, fuse);
            for q in std::iter::once(p).chain(FACES.iter().map(|&d| p.offset(d))) {
                for &lens in grid.cell(q) {
                    if !has(blocks, lens, Property::Lens) {
                        continue;
                    }
                    let dirs = touch_directions(p, &a, q, &texels(blocks, lens));
                    if dirs.is_empty() {
                        continue;
                    }
                    for piece in [Piece { pos: p, occupant: fuse }, Piece { pos: q, occupant: lens }] {
                        if used.insert(piece) {
                            out.used.push(piece);
                        }
                    }
                    for dir in dirs {
                        if fired.insert((q, lens, dir)) {
                            out.firings.push(Firing { lens: q, dir, block: lens.block });
                        }
                    }
                }
            }
        }
    }
    out
}

/// Where a beam went and what it does.
#[derive(Debug, Default, PartialEq)]
pub struct Beam {
    /// Boxels it ran through past the lens's, the last one where it stopped included.
    pub length: u32,
    /// The pieces it breaks, in the order it met them.
    pub broken: Vec<Piece>,
    /// The lens piece it reached, if it stopped at one: it sets off and fires on.
    pub chained: Option<Piece>,
}

impl Firing {
    /// Where its beam leaves the lens's boxel, and its way as a vector.
    pub fn start(self) -> (WorldPos, Vec3) {
        let dir = self.dir.as_vec3();
        (WorldPos::at_boxel(self.lens).translate(Vec3::splat(0.5) + dir * 0.5), dir)
    }
}

/// The beam `firing` sends, as `def` says: what it breaks, where it stops. Changes nothing.
pub fn beam(grid: &Grid, blocks: &BlockRegistry, firing: Firing, def: &BlastDef) -> Beam {
    let range = def.range.round().max(0.0) as u32;
    let mut strength = def.strength;
    let mut out = Beam::default();
    for k in 1..=range {
        let pos = firing.lens.offset(firing.dir * k as i64);
        out.length = k;
        for &o in grid.cell(pos) {
            if blocks.is_hidden(o.block) {
                continue;
            }
            if has(blocks, o, Property::Lens) {
                out.chained = Some(Piece { pos, occupant: o });
                return out;
            }
            if strength <= 0.0 {
                return out;
            }
            strength -= texels(blocks, o).volume() as f32 / FULL_VOLUME as f32;
            out.broken.push(Piece { pos, occupant: o });
        }
        if strength <= 0.0 {
            return out;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{BlockBox, BlockDef, Orientation};

    struct World {
        grid: Grid,
        blocks: BlockRegistry,
        stone: BlockId,
        fuse: BlockId,
        lens: BlockId,
        /// Half a fuse: the west half of the boxel.
        fuse_half: BlockId,
        lens_half: BlockId,
    }

    fn world() -> World {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(BlockDef::new("test:stone", true, [1; 3])).unwrap();
        let fuse = blocks.register(BlockDef::new("test:fuse", true, [2; 3]).with_properties(vec![Property::Fuse])).unwrap();
        let lens = blocks.register(BlockDef::new("test:lens", true, [3; 3]).with_properties(vec![Property::Lens])).unwrap();
        let west = vec![BlockBox::new([0, 0, 0], [8, 16, 16])];
        let fuse_half = blocks
            .register(BlockDef::new("test:fuse_half", true, [2; 3]).with_properties(vec![Property::Fuse]).with_shape(west.clone()))
            .unwrap();
        let lens_half = blocks
            .register(BlockDef::new("test:lens_half", true, [3; 3]).with_properties(vec![Property::Lens]).with_shape(west))
            .unwrap();
        World { grid: Grid::default(), blocks, stone, fuse, lens, fuse_half, lens_half }
    }

    fn at(x: i64, y: i64, z: i64) -> BoxelPos {
        BoxelPos(I64Vec3::new(x, y, z))
    }

    impl World {
        fn put(&mut self, pos: BoxelPos, block: BlockId) {
            let mut cell = self.grid.cell(pos).to_vec();
            cell.push(Occupant { block, orientation: Orientation::IDENTITY });
            self.grid.set_cell(pos, &cell);
        }
    }

    #[test]
    fn touching_crystals_fire_from_fuse_to_lens() {
        let mut w = world();
        w.put(at(0, 0, 0), w.fuse);
        w.put(at(1, 0, 0), w.lens);
        let r = reactions(&w.grid, &w.blocks, &[at(0, 0, 0)]);
        assert_eq!(ways(&r), [(at(1, 0, 0), I64Vec3::X)]);
        assert_eq!(r.used.len(), 2, "both used up");
        // Looked at from the lens's side too, the same.
        assert_eq!(ways(&reactions(&w.grid, &w.blocks, &[at(1, 0, 0)])), ways(&r));
        // Below instead: it fires up.
        let mut w = world();
        w.put(at(0, 0, 0), w.fuse);
        w.put(at(0, 0, 1), w.lens);
        assert_eq!(ways(&reactions(&w.grid, &w.blocks, &[at(0, 0, 1)])), [(at(0, 0, 1), I64Vec3::Z)]);
    }

    #[test]
    fn crystals_a_texel_apart_or_corner_to_corner_do_nothing() {
        let mut w = world();
        // A west-half fuse leaves the boxel's east half empty, so a lens east of it is 8 texels off.
        w.put(at(0, 0, 0), w.fuse_half);
        w.put(at(1, 0, 0), w.lens);
        assert!(reactions(&w.grid, &w.blocks, &[at(0, 0, 0)]).is_empty());
        // Corner to corner isn't face to face.
        let mut w = world();
        w.put(at(0, 0, 0), w.fuse);
        w.put(at(1, 1, 0), w.lens);
        assert!(reactions(&w.grid, &w.blocks, &[at(0, 0, 0)]).is_empty());
        // Nothing near what changed: nothing looked at.
        let mut w = world();
        w.put(at(0, 0, 0), w.fuse);
        w.put(at(1, 0, 0), w.lens);
        assert!(reactions(&w.grid, &w.blocks, &[at(9, 9, 9)]).is_empty());
    }

    #[test]
    fn pieces_sharing_a_boxel_fire_along_the_face_they_share() {
        let mut w = world();
        // West half a fuse, east half a lens: a lens half turned half a turn about Z.
        w.put(at(0, 0, 0), w.fuse_half);
        let mut cell = w.grid.cell(at(0, 0, 0)).to_vec();
        let east = Orientation::all().into_iter().find(|&o| {
            let s = w.blocks.shape(w.lens_half).turned(o);
            s.get(15, 0, 0) && s.get(15, 15, 15) && !s.get(0, 0, 0)
        });
        cell.push(Occupant { block: w.lens_half, orientation: east.expect("an east half") });
        w.grid.set_cell(at(0, 0, 0), &cell);
        assert_eq!(ways(&reactions(&w.grid, &w.blocks, &[at(0, 0, 0)])), [(at(0, 0, 0), I64Vec3::X)]);
    }

    #[test]
    fn a_lens_touched_on_two_sides_fires_two_ways() {
        let mut w = world();
        w.put(at(1, 0, 0), w.lens);
        w.put(at(0, 0, 0), w.fuse);
        w.put(at(1, -1, 0), w.fuse);
        let r = reactions(&w.grid, &w.blocks, &[at(1, 0, 0)]);
        let mut dirs: Vec<_> = r.firings.iter().map(|f| f.dir).collect();
        dirs.sort_by_key(|d| d.to_array());
        assert_eq!(dirs, [I64Vec3::Y, I64Vec3::X]);
        assert_eq!(r.used.len(), 3);
    }

    #[test]
    fn hidden_crystals_do_nothing() {
        let mut w = world();
        w.put(at(0, 0, 0), w.fuse);
        let hidden = w.blocks.derive(Occupant { block: w.lens, orientation: Orientation::IDENTITY }, &ShapeGrid::FULL, true);
        let mut cell = w.grid.cell(at(1, 0, 0)).to_vec();
        cell.push(Occupant { block: hidden.unwrap(), orientation: Orientation::IDENTITY });
        w.grid.set_cell(at(1, 0, 0), &cell);
        assert!(reactions(&w.grid, &w.blocks, &[at(0, 0, 0)]).is_empty());
    }

    /// Where and which way each firing goes.
    fn ways(r: &Reaction) -> Vec<(BoxelPos, I64Vec3)> {
        r.firings.iter().map(|f| (f.lens, f.dir)).collect()
    }

    fn fire(lens: BoxelPos, dir: I64Vec3) -> Firing {
        Firing { lens, dir, block: BlockId::EMPTY }
    }

    fn def(range: f32, strength: f32) -> BlastDef {
        BlastDef { range, strength, radius: 0.5, damage: 1.0, player_damage: 1.0, push: 1.0 }
    }

    #[test]
    fn a_beam_breaks_until_its_strength_is_spent() {
        let mut w = world();
        for x in 2..10 {
            w.put(at(x, 0, 0), w.stone);
        }
        let b = beam(&w.grid, &w.blocks, fire(at(0, 0, 0), I64Vec3::X), &def(16.0, 3.0));
        assert_eq!(b.broken.iter().map(|p| p.pos).collect::<Vec<_>>(), [at(2, 0, 0), at(3, 0, 0), at(4, 0, 0)]);
        assert_eq!(b.length, 4, "stops where the last block broke");
        assert_eq!(b.chained, None);
        // Half blocks use up half.
        let mut w = world();
        for x in 1..10 {
            w.put(at(x, 0, 0), w.fuse_half);
        }
        let b = beam(&w.grid, &w.blocks, fire(at(0, 0, 0), I64Vec3::X), &def(16.0, 3.0));
        assert_eq!(b.broken.len(), 6);
    }

    #[test]
    fn a_beam_stops_at_its_range_through_air() {
        let w = world();
        let b = beam(&w.grid, &w.blocks, fire(at(0, 0, 0), I64Vec3::NEG_Z), &def(5.0, 3.0));
        assert_eq!((b.length, b.broken.len()), (5, 0));
        let mut w = world();
        w.put(at(0, 0, -6), w.stone);
        let b = beam(&w.grid, &w.blocks, fire(at(0, 0, 0), I64Vec3::NEG_Z), &def(5.0, 3.0));
        assert!(b.broken.is_empty(), "out of range");
    }

    #[test]
    fn a_beam_reaching_a_lens_chains() {
        let mut w = world();
        w.put(at(2, 0, 0), w.stone);
        w.put(at(4, 0, 0), w.lens);
        w.put(at(5, 0, 0), w.stone);
        let b = beam(&w.grid, &w.blocks, fire(at(0, 0, 0), I64Vec3::X), &def(16.0, 3.0));
        assert_eq!(b.broken.len(), 1);
        assert_eq!(b.chained.map(|p| p.pos), Some(at(4, 0, 0)));
        assert_eq!(b.length, 4);
        // A fuse in the way just breaks.
        let mut w = world();
        w.put(at(2, 0, 0), w.fuse);
        let b = beam(&w.grid, &w.blocks, fire(at(0, 0, 0), I64Vec3::X), &def(16.0, 3.0));
        assert_eq!((b.broken.len(), b.chained), (1, None));
    }
}
