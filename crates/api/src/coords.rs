//! World addressing. +Z is up, right-handed, +X east, +Y north.
//!
//! Positions are never one large float: a [`WorldPos`] is an integer cell plus a small
//! offset inside it, so precision is the same everywhere in the world.

use glam::{I64Vec3, Vec3};

/// Boxels along each edge of a chunk.
pub const CHUNK_SIZE: i64 = 32;
/// Boxels in one chunk.
pub const CHUNK_VOLUME: usize = (CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE) as usize;

/// The integer address of one boxel in a grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BoxelPos(pub I64Vec3);

impl BoxelPos {
    pub const fn new(x: i64, y: i64, z: i64) -> Self {
        Self(I64Vec3::new(x, y, z))
    }

    /// The chunk containing this boxel. Rounds toward negative infinity, so -1 is in chunk -1.
    pub fn chunk(self) -> ChunkPos {
        ChunkPos(self.0.div_euclid(I64Vec3::splat(CHUNK_SIZE)))
    }

    /// This boxel's position inside its chunk.
    pub fn local(self) -> LocalPos {
        let r = self.0.rem_euclid(I64Vec3::splat(CHUNK_SIZE));
        LocalPos::new(r.x as u8, r.y as u8, r.z as u8)
    }

    pub fn split(self) -> (ChunkPos, LocalPos) {
        (self.chunk(), self.local())
    }

    pub fn offset(self, by: I64Vec3) -> Self {
        Self(self.0 + by)
    }
}

/// The integer address of one chunk in a grid, in chunk units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ChunkPos(pub I64Vec3);

impl ChunkPos {
    pub const fn new(x: i64, y: i64, z: i64) -> Self {
        Self(I64Vec3::new(x, y, z))
    }

    /// The boxel at this chunk's minimum corner.
    pub fn origin(self) -> BoxelPos {
        BoxelPos(self.0 * CHUNK_SIZE)
    }

    pub fn boxel(self, local: LocalPos) -> BoxelPos {
        self.origin().offset(local.to_ivec())
    }
}

/// A boxel's position inside its chunk, each axis in `0..CHUNK_SIZE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalPos {
    pub x: u8,
    pub y: u8,
    pub z: u8,
}

impl LocalPos {
    /// Panics if any axis is outside the chunk.
    pub fn new(x: u8, y: u8, z: u8) -> Self {
        let size = CHUNK_SIZE as u8;
        assert!(x < size && y < size && z < size, "local position out of chunk: {x}, {y}, {z}");
        Self { x, y, z }
    }

    /// Index into a chunk's flat storage, X fastest, then Y, then Z.
    pub fn index(self) -> usize {
        let size = CHUNK_SIZE as usize;
        self.x as usize + self.y as usize * size + self.z as usize * size * size
    }

    pub fn from_index(index: usize) -> Self {
        let size = CHUNK_SIZE as usize;
        assert!(index < CHUNK_VOLUME, "chunk index out of range: {index}");
        Self {
            x: (index % size) as u8,
            y: (index / size % size) as u8,
            z: (index / (size * size)) as u8,
        }
    }

    pub fn to_ivec(self) -> I64Vec3 {
        I64Vec3::new(self.x as i64, self.y as i64, self.z as i64)
    }
}

/// A continuous position: an integer cell plus an offset inside it, each axis in `[0, 1)`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldPos {
    cell: I64Vec3,
    offset: Vec3,
}

impl WorldPos {
    /// Builds a position from a cell and any offset; whole units of the offset carry into the cell.
    pub fn new(cell: I64Vec3, offset: Vec3) -> Self {
        let mut pos = Self { cell, offset: Vec3::ZERO };
        pos.offset = pos.carry(offset);
        pos
    }

    /// The minimum corner of a boxel.
    pub fn at_boxel(boxel: BoxelPos) -> Self {
        Self { cell: boxel.0, offset: Vec3::ZERO }
    }

    pub fn cell(self) -> I64Vec3 {
        self.cell
    }

    pub fn offset(self) -> Vec3 {
        self.offset
    }

    /// The boxel this position is inside.
    pub fn boxel(self) -> BoxelPos {
        BoxelPos(self.cell)
    }

    /// Moves by a small delta. The arithmetic happens in the offset's small range, so the
    /// result is equally precise at any distance from zero.
    pub fn translate(self, delta: Vec3) -> Self {
        let mut pos = self;
        pos.offset = pos.carry(self.offset + delta);
        pos
    }

    /// The vector from `self` to `other`. Only precise when the two are close, which is the
    /// only way it should be used: physics steps, interpolation and render-origin offsets.
    pub fn delta_to(self, other: Self) -> Vec3 {
        (other.cell - self.cell).as_vec3() + (other.offset - self.offset)
    }

    /// Moves whole units of `offset` into the cell and returns what remains, in `[0, 1)`.
    fn carry(&mut self, offset: Vec3) -> Vec3 {
        let mut out = offset.to_array();
        let mut cell = self.cell.to_array();
        for (o, c) in out.iter_mut().zip(cell.iter_mut()) {
            let whole = o.floor();
            *c += whole as i64;
            *o -= whole;
            // A tiny negative offset floors to -1 and then rounds up to exactly 1.0.
            if *o >= 1.0 {
                *o = 0.0;
                *c += 1;
            }
        }
        self.cell = I64Vec3::from_array(cell);
        Vec3::from_array(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxel_to_chunk_and_local_handles_negatives() {
        assert_eq!(BoxelPos::new(0, 0, 0).split(), (ChunkPos::new(0, 0, 0), LocalPos::new(0, 0, 0)));
        assert_eq!(BoxelPos::new(31, 32, 33).split(), (ChunkPos::new(0, 1, 1), LocalPos::new(31, 0, 1)));
        assert_eq!(BoxelPos::new(-1, -32, -33).split(), (ChunkPos::new(-1, -1, -2), LocalPos::new(31, 0, 31)));
    }

    #[test]
    fn chunk_and_local_round_trip() {
        for b in [
            BoxelPos::new(0, 0, 0),
            BoxelPos::new(-1, 5, -64),
            BoxelPos::new(3_000_017, -2_000_001, 1_000),
            BoxelPos::new(i64::MAX / 2, i64::MIN / 2, 7),
        ] {
            let (chunk, local) = b.split();
            assert_eq!(chunk.boxel(local), b);
        }
    }

    #[test]
    fn local_index_round_trips() {
        for i in [0, 1, 31, 32, 1023, 1024, CHUNK_VOLUME - 1] {
            assert_eq!(LocalPos::from_index(i).index(), i);
        }
        assert_eq!(LocalPos::new(1, 2, 3).index(), 1 + 2 * 32 + 3 * 1024);
    }

    #[test]
    #[should_panic]
    fn local_rejects_out_of_chunk() {
        LocalPos::new(32, 0, 0);
    }

    #[test]
    fn world_pos_carries_offsets() {
        let p = WorldPos::new(I64Vec3::new(10, 10, 10), Vec3::new(2.5, -0.25, 0.0));
        assert_eq!(p.cell(), I64Vec3::new(12, 9, 10));
        assert_eq!(p.offset(), Vec3::new(0.5, 0.75, 0.0));
    }

    #[test]
    fn world_pos_tiny_negative_stays_in_range() {
        let p = WorldPos::new(I64Vec3::ZERO, Vec3::new(-1e-9, 0.0, 0.0));
        assert!(p.offset().x >= 0.0 && p.offset().x < 1.0);
        assert!(p.cell().x == -1 || p.cell().x == 0);
    }

    #[test]
    fn world_pos_translate_and_delta_agree() {
        let a = WorldPos::new(I64Vec3::new(-5, 0, 3), Vec3::new(0.9, 0.1, 0.5));
        let b = a.translate(Vec3::new(0.3, -0.4, 2.0));
        assert_eq!(b.cell(), I64Vec3::new(-4, -1, 5));
        assert!((a.delta_to(b) - Vec3::new(0.3, -0.4, 2.0)).length() < 1e-6);
    }

    #[test]
    fn world_pos_is_equally_precise_in_the_millions() {
        let near = WorldPos::new(I64Vec3::ZERO, Vec3::splat(0.5));
        let far = WorldPos::new(I64Vec3::new(3_000_000, -2_000_000, 1_000_000), Vec3::splat(0.5));
        let step = Vec3::new(0.001, -0.0007, 0.0003);
        let (mut n, mut f) = (near, far);
        for _ in 0..10_000 {
            n = n.translate(step);
            f = f.translate(step);
        }
        assert_eq!(n.offset(), f.offset());
        assert_eq!(f.cell() - far.cell(), n.cell() - near.cell());
    }
}
