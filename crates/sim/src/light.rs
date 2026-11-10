//! Stored light: how much light from the sky and from blocks that give light reaches each
//! boxel, in four channels (red, green, blue and sky), each from 0 to [`Light::MAX`].
//!
//! The rule, channel by channel: a boxel's light is the brighter of what it gives off itself
//! and what its brightest neighbor's passes on. Block light keeps a share of itself each boxel
//! ([`Light::KEEP`]), as real light dims by a share with distance, so every channel of a lamp
//! fades together and its color holds all the way out. Sky light, one channel with no color to
//! keep, loses a fixed step ([`Light::SKY_STEP`]) instead, so shade deepens evenly under a roof.
//! Sky light also falls straight down through open space
//! without dimming, so open ground is lit fully and light coming in sideways fades. Light goes
//! from a boxel to the next through the face between them only if neither boxel's shown pieces
//! cover that face completely, texel by texel: a shutter against a face closes it, a slab is lit
//! from above but shades what's under it. Light a boxel gives off itself also leaves through
//! covered faces, so a lamp lights around it whatever its shape. A panel across the middle of a
//! boxel covers neither face, so light gets past it. A missing chunk is open sky.
//!
//! The rule has one settled state, so it isn't stepped tick by tick like the design doc's other
//! boxel rules: queues solve it straight there, all at once when terrain is made and around the
//! boxels that changed after edits, and it's settled again by the end of each tick. Stepping it
//! until it stopped changing would end in the same light.

use std::collections::{HashMap, HashSet, VecDeque};

use api::{BlockDef, BoxelPos, CHUNK_VOLUME, ChunkPos, I64Vec3, ShapeGrid};

use crate::{BlockRegistry, CellId, Chunk, Grid, Layer, Occupant};

/// Light in one boxel: red, green, blue and sky, a byte each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Light(u32);

impl Light {
    /// The brightest a channel gets.
    pub const MAX: u8 = BlockDef::MAX_LIGHT;
    /// The sky channel's index; red, green and blue are 0, 1 and 2.
    pub const SKY_CHANNEL: usize = 3;
    pub const DARK: Light = Light(0);
    /// Open sky: full sky light, no block light.
    pub const SKY: Light = Light((Self::MAX as u32) << 24);
    /// How much block light one boxel passes to the next, out of 256: about 70%, so full light
    /// reaches about 14 boxels before it's gone.
    pub const KEEP: u32 = 179;
    /// How much sky light drops each boxel it goes other than straight down: full sky is gone
    /// after 15.
    pub const SKY_STEP: u8 = 17;

    pub fn new(rgb: [u8; 3], sky: u8) -> Light {
        Light(rgb[0] as u32 | (rgb[1] as u32) << 8 | (rgb[2] as u32) << 16 | (sky as u32) << 24)
    }

    /// Channel `c`: 0 red, 1 green, 2 blue, 3 sky.
    #[inline]
    pub fn channel(self, c: usize) -> u8 {
        (self.0 >> (8 * c)) as u8
    }

    #[inline]
    fn with_channel(self, c: usize, v: u8) -> Light {
        Light(self.0 & !(0xff << (8 * c)) | (v as u32) << (8 * c))
    }

    /// What `level` in channel `c` is one boxel on, not straight down.
    #[inline]
    pub fn passed_on(c: usize, level: u8) -> u8 {
        if c == Self::SKY_CHANNEL { level.saturating_sub(Self::SKY_STEP) } else { ((level as u32 * Self::KEEP) >> 8) as u8 }
    }

    pub fn rgb(self) -> [u8; 3] {
        [self.channel(0), self.channel(1), self.channel(2)]
    }

    pub fn sky(self) -> u8 {
        self.channel(Self::SKY_CHANNEL)
    }
}

/// What a boxel's occupants do to light: which of its faces they cover, stopping light there,
/// and what they give off. Hidden pieces do neither.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct CellLight {
    /// Bit `d` set where the face toward [`DIRS`]`[d]` is covered.
    pub covered: u8,
    /// Red, green and blue given off; no sky.
    pub emits: Light,
}

/// Every face covered: a boxel no light gets into.
pub const ALL_FACES: u8 = 0b11_1111;

/// Which faces of a boxel holding `occupants` its shown pieces cover completely, stopping light
/// there: bit `d` for the face toward east, west, north, south, up and down, in that order.
pub fn covered_faces_of(occupants: &[Occupant], blocks: &BlockRegistry) -> u8 {
    CellLight::of(occupants, blocks).covered
}

impl CellLight {
    fn of(occupants: &[Occupant], blocks: &BlockRegistry) -> CellLight {
        let mut filled = ShapeGrid::EMPTY;
        let mut rgb = [0u8; 3];
        for o in occupants.iter().filter(|o| !blocks.is_hidden(o.block)) {
            let Some(def) = blocks.get(o.block) else { continue };
            filled = if def.is_full() { ShapeGrid::FULL } else { filled.union(&blocks.shape(o.block).turned(o.orientation)) };
            for (c, l) in rgb.iter_mut().zip(def.light) {
                *c = (*c).max(l);
            }
        }
        CellLight { covered: covered_faces(&filled), emits: Light::new(rgb, 0) }
    }

    /// Covers every face, so no light gets in.
    fn closed(self) -> bool {
        self.covered == ALL_FACES
    }

    fn covers(self, d: usize) -> bool {
        self.covered & (1 << d) != 0
    }
}

/// Which faces of its boxel `shape` covers completely, as bits by direction (see [`DIRS`]).
fn covered_faces(shape: &ShapeGrid) -> u8 {
    if shape.is_full() {
        return ALL_FACES;
    }
    const N: usize = api::BlockBox::SIZE as usize;
    let rows = shape.rows();
    let row = |y: usize, z: usize| rows[y + N * z];
    let full = |mut rows: Box<dyn Iterator<Item = u16> + '_>| rows.all(|r| r == u16::MAX);
    let faces = [
        rows.iter().all(|r| r & (1 << (N - 1)) != 0),
        rows.iter().all(|r| r & 1 != 0),
        full(Box::new((0..N).map(|z| row(N - 1, z)))),
        full(Box::new((0..N).map(|z| row(0, z)))),
        full(Box::new((0..N).map(|y| row(y, N - 1)))),
        full(Box::new((0..N).map(|y| row(y, 0)))),
    ];
    faces.iter().enumerate().fold(0, |bits, (d, &f)| bits | (f as u8) << d)
}

/// How bright channel `c` arrives in a boxel `to` from its neighbor `from` with `level`, one
/// step along direction `d`; 0 when the face between them stops it.
#[inline]
fn carried(from: CellLight, to: CellLight, d: usize, level: u8, c: usize) -> u8 {
    if to.covers(d ^ 1) {
        return 0;
    }
    if from.covers(d) {
        // Only its own light leaves through a covered face.
        return Light::passed_on(c, from.emits.channel(c));
    }
    if c == Light::SKY_CHANNEL && d == DOWN && level == Light::MAX {
        return level;
    }
    Light::passed_on(c, level)
}

const SIZE: usize = api::CHUNK_SIZE as usize;
/// The six directions, as offsets: east, west, north, south, up, down.
const DIRS: [[i64; 3]; 6] = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];
const UP: usize = 4;
const DOWN: usize = 5;
/// The sideways directions.
const SIDES: [usize; 4] = [0, 1, 2, 3];

/// A boxel in a pass: the slot of its chunk in the grid, and where in the chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Node {
    slot: u32,
    at: u16,
}

/// What's one step from a boxel.
enum Step {
    In(Node),
    /// A missing chunk: open sky, never written.
    Missing,
}

fn coords(at: usize) -> [usize; 3] {
    [at % SIZE, at / SIZE % SIZE, at / (SIZE * SIZE)]
}

fn index([x, y, z]: [usize; 3]) -> usize {
    x + SIZE * (y + SIZE * z)
}

/// The grid's chunks, borrowed for spreading light through them.
struct Pass<'a> {
    chunks: &'a mut [Chunk],
    positions: &'a [ChunkPos],
    slots: &'a HashMap<ChunkPos, usize>,
    info: &'a [CellLight],
    /// Every boxel written to, with its light before the first write, so only those that end
    /// up different count as changed.
    touched: Option<HashMap<Node, Light>>,
}

impl Pass<'_> {
    fn step(&self, n: Node, d: usize) -> Step {
        let at = n.at as usize;
        let mut c = coords(at);
        let axis = d / 2;
        let forward = d.is_multiple_of(2);
        let inside = if forward { c[axis] + 1 < SIZE } else { c[axis] > 0 };
        if inside {
            c[axis] = if forward { c[axis] + 1 } else { c[axis] - 1 };
            return Step::In(Node { slot: n.slot, at: index(c) as u16 });
        }
        c[axis] = if forward { 0 } else { SIZE - 1 };
        let pos = ChunkPos(self.positions[n.slot as usize].0 + I64Vec3::from_array(DIRS[d]));
        match self.slots.get(&pos) {
            Some(&slot) => Step::In(Node { slot: slot as u32, at: index(c) as u16 }),
            None => Step::Missing,
        }
    }

    #[inline]
    fn get(&self, n: Node) -> Light {
        self.chunks[n.slot as usize].light.at(n.at as usize)
    }

    #[inline]
    fn info(&self, n: Node) -> CellLight {
        self.info[self.chunks[n.slot as usize].cells.at(n.at as usize).index()]
    }

    #[inline]
    fn level(&self, n: Node, c: usize) -> u8 {
        self.get(n).channel(c)
    }

    fn set_level(&mut self, n: Node, c: usize, v: u8) {
        let old = self.get(n);
        self.chunks[n.slot as usize].light.set_at(n.at as usize, old.with_channel(c, v));
        if let Some(touched) = &mut self.touched {
            touched.entry(n).or_insert(old);
        }
    }

    /// The light a boxel gets from missing chunks beside it, in channel `c`.
    fn sky_from_missing(&self, n: Node, c: usize) -> u8 {
        if c != Light::SKY_CHANNEL {
            return 0;
        }
        let info = self.info(n);
        let mut best = 0;
        for d in 0..6 {
            if !info.covers(d)
                && let Step::Missing = self.step(n, d)
            {
                best = best.max(if d == UP { Light::MAX } else { Light::passed_on(c, Light::MAX) });
            }
        }
        best
    }

    /// Spreads channel `c` out from every boxel in `queue` until nothing brightens.
    fn spread(&mut self, mut queue: VecDeque<Node>, c: usize) {
        while let Some(n) = queue.pop_front() {
            let l = self.level(n, c);
            if l == 0 {
                continue;
            }
            let from = self.info(n);
            for d in 0..6 {
                let Step::In(m) = self.step(n, d) else { continue };
                let want = carried(from, self.info(m), d, l, c);
                if want == 0 || self.level(m, c) >= want {
                    continue;
                }
                self.set_level(m, c, want);
                queue.push_back(m);
            }
        }
    }

    /// Settles channel `c` again after the boxels in `changed` changed: takes away all the
    /// light that could have come through them, then spreads back in from what's left.
    fn relight(&mut self, changed: &[Node], c: usize) {
        let mut removing: VecDeque<(Node, u8)> = VecDeque::new();
        let mut adding: VecDeque<Node> = VecDeque::new();
        let mut cleared: Vec<Node> = Vec::new();
        for &n in changed {
            let l = self.level(n, c);
            if l > 0 {
                self.set_level(n, c, 0);
                removing.push_back((n, l));
            }
            cleared.push(n);
            for d in 0..6 {
                if let Step::In(m) = self.step(n, d) {
                    adding.push_back(m);
                }
            }
        }
        while let Some((n, l)) = removing.pop_front() {
            for d in 0..6 {
                let Step::In(m) = self.step(n, d) else { continue };
                let ml = self.level(m, c);
                if ml == 0 {
                    continue;
                }
                let fed_by_n = ml < l || (c == Light::SKY_CHANNEL && d == DOWN && l == Light::MAX && ml == Light::MAX);
                if fed_by_n {
                    self.set_level(m, c, 0);
                    removing.push_back((m, ml));
                    cleared.push(m);
                } else {
                    adding.push_back(m);
                }
            }
        }
        for n in cleared {
            let own = self.info(n).emits.channel(c).max(self.sky_from_missing(n, c));
            if own > self.level(n, c) {
                self.set_level(n, c, own);
                adding.push_back(n);
            }
        }
        self.spread(adding, c);
    }
}

/// Sky falling down through one chunk: boxels under open sky (`open`, bit x of row y) get full
/// sky light unless their top is covered, and each column closes at its first boxel with its
/// top or bottom covered. Also gives
/// every boxel the light it gives off, and lists the boxels that give some.
fn fall(cells: &Layer<CellId>, info: &[CellLight], open: &mut [u32; SIZE], emitting: &mut Vec<u16>) -> Layer<Light> {
    if let Some(id) = cells.uniform() {
        let cell = info[id.index()];
        if cell.emits != Light::DARK {
            emitting.extend(0..CHUNK_VOLUME as u16);
        }
        if cell.closed() {
            *open = [0; SIZE];
            return Layer::Uniform(cell.emits);
        }
        let through = !cell.covers(UP) && !cell.covers(DOWN);
        if through && open.iter().all(|&row| row == u32::MAX) {
            return Layer::Uniform(Light::new(cell.emits.rgb(), Light::MAX));
        }
        if through && open.iter().all(|&row| row == 0) {
            return Layer::Uniform(cell.emits);
        }
    }
    let mut values = vec![Light::DARK; CHUNK_VOLUME].into_boxed_slice();
    for z in (0..SIZE).rev() {
        for (y, row) in open.iter_mut().enumerate() {
            for x in 0..SIZE {
                let at = index([x, y, z]);
                let cell = info[cells.at(at).index()];
                if cell.emits != Light::DARK {
                    emitting.push(at as u16);
                }
                let lit = *row & (1 << x) != 0 && !cell.covers(UP);
                values[at] = Light::new(cell.emits.rgb(), if lit { Light::MAX } else { 0 });
                if !lit || cell.covers(DOWN) {
                    *row &= !(1 << x);
                }
            }
        }
    }
    let mut layer = Layer::Dense(values);
    layer.compact();
    layer
}

/// Every boxel on a chunk's face toward direction `d`.
fn face(d: usize) -> impl Iterator<Item = usize> {
    let axis = d / 2;
    let fixed = if d.is_multiple_of(2) { SIZE - 1 } else { 0 };
    (0..SIZE * SIZE).map(move |i| {
        let (a, b) = (i % SIZE, i / SIZE);
        let mut c = [a, b, 0];
        c.copy_within(axis..2, axis + 1);
        c[axis] = fixed;
        index(c)
    })
}

impl Grid {
    /// Brings the light rules for every occupant list up to date.
    fn update_light_info(&mut self, blocks: &BlockRegistry) {
        while self.light_info.len() < self.cells.len() {
            let id = CellId::from_index(self.light_info.len());
            self.light_info.push(CellLight::of(self.cells.get(id), blocks));
        }
    }

    fn pass(&mut self, track: bool) -> Pass<'_> {
        Pass {
            chunks: &mut self.chunks,
            positions: &self.positions,
            slots: &self.slots,
            info: &self.light_info,
            touched: track.then(HashMap::new),
        }
    }

    /// Works out the light in every chunk from scratch.
    pub fn relight_all(&mut self, blocks: &BlockRegistry) {
        self.light_info.clear();
        self.update_light_info(blocks);
        // Down each column of chunks, top first, carrying which columns of boxels see the sky.
        let mut order: Vec<usize> = (0..self.chunks.len()).collect();
        order.sort_by_key(|&i| {
            let p = self.positions[i].0;
            (p.x, p.y, std::cmp::Reverse(p.z))
        });
        let mut open = [u32::MAX; SIZE];
        let mut above: Option<ChunkPos> = None;
        let mut emitting: Vec<Node> = Vec::new();
        let mut found = Vec::new();
        for &slot in &order {
            let pos = self.positions[slot];
            if above.is_none_or(|a| a.0 != pos.0 + I64Vec3::Z) {
                open = [u32::MAX; SIZE];
            }
            above = Some(pos);
            found.clear();
            self.chunks[slot].light = fall(&self.chunks[slot].cells, &self.light_info, &mut open, &mut found);
            emitting.extend(found.iter().map(|&at| Node { slot: slot as u32, at }));
        }

        let mut pass = self.pass(false);
        // Sky comes in sideways wherever a fully lit boxel stands beside a darker open one, and
        // from missing chunks at the edges.
        let mut sky = VecDeque::new();
        for slot in 0..pass.chunks.len() {
            let node = |at: usize| Node { slot: slot as u32, at: at as u16 };
            let lights_beside = |pass: &Pass, n: Node| {
                SIDES.iter().any(|&d| match pass.step(n, d) {
                    Step::In(m) => {
                        let want = carried(pass.info(n), pass.info(m), d, Light::MAX, Light::SKY_CHANNEL);
                        pass.level(m, Light::SKY_CHANNEL) < want
                    }
                    Step::Missing => false,
                })
            };
            match &pass.chunks[slot].light {
                Layer::Uniform(l) if l.sky() == Light::MAX => {
                    for d in SIDES {
                        sky.extend(face(d).map(node).filter(|&n| lights_beside(&pass, n)));
                    }
                }
                Layer::Uniform(_) => {}
                Layer::Dense(_) => {
                    for at in 0..CHUNK_VOLUME {
                        let n = node(at);
                        if pass.level(n, Light::SKY_CHANNEL) == Light::MAX && lights_beside(&pass, n) {
                            sky.push_back(n);
                        }
                    }
                }
            }
            let pos = pass.positions[slot];
            for d in [0, 1, 2, 3, DOWN] {
                if !pass.slots.contains_key(&ChunkPos(pos.0 + I64Vec3::from_array(DIRS[d]))) {
                    for at in face(d) {
                        let n = node(at);
                        let side = Light::passed_on(Light::SKY_CHANNEL, Light::MAX);
                        if pass.level(n, Light::SKY_CHANNEL) < side && !pass.info(n).covers(d) {
                            pass.set_level(n, Light::SKY_CHANNEL, side);
                            sky.push_back(n);
                        }
                    }
                }
            }
        }
        pass.spread(sky, Light::SKY_CHANNEL);
        for c in 0..3 {
            let queue = emitting.iter().copied().filter(|&n| pass.info(n).emits.channel(c) > 0).collect();
            pass.spread(queue, c);
        }
        for chunk in &mut self.chunks {
            chunk.light.compact();
        }
    }

    /// Settles the light again after the boxels at `changed` changed. Returns every chunk whose
    /// light changed, and every chunk beside one whose border boxels did (as a face there reads
    /// them), in no particular order.
    pub fn relight(&mut self, blocks: &BlockRegistry, changed: &[BoxelPos]) -> Vec<ChunkPos> {
        self.update_light_info(blocks);
        let mut nodes: Vec<Node> = Vec::new();
        let mut seen = HashSet::new();
        for &pos in changed {
            let (chunk, local) = pos.split();
            if let Some(&slot) = self.slots.get(&chunk)
                && seen.insert(pos)
            {
                nodes.push(Node { slot: slot as u32, at: local.index() as u16 });
            }
        }
        if nodes.is_empty() {
            return Vec::new();
        }
        let mut pass = self.pass(true);
        for c in 0..4 {
            pass.relight(&nodes, c);
        }
        let touched = pass.touched.take().unwrap_or_default();
        // Each chunk whose light ended up different, with which of its faces the boxels that
        // changed lie on, so chunks whose border faces read them hear of it.
        let mut edged: HashSet<(u32, u8)> = HashSet::new();
        for (n, before) in touched {
            if pass.get(n) != before {
                let [x, y, z] = coords(n.at as usize);
                let edge = |v: usize, bit: u8| if v == SIZE - 1 { bit } else if v == 0 { bit << 1 } else { 0 };
                edged.insert((n.slot, edge(x, 1) | edge(y, 4) | edge(z, 16)));
            }
        }
        let mut out = HashSet::new();
        for (slot, edges) in edged {
            let pos = self.positions[slot as usize];
            let options = |bit: u8| {
                let mut o = vec![0i64];
                if edges & bit != 0 {
                    o.push(1);
                }
                if edges & (bit << 1) != 0 {
                    o.push(-1);
                }
                o
            };
            for &dx in &options(1) {
                for &dy in &options(4) {
                    for &dz in &options(16) {
                        out.insert(ChunkPos(pos.0 + I64Vec3::new(dx, dy, dz)));
                    }
                }
            }
            self.chunks[slot as usize].light.compact();
        }
        out.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{BlockBox, Orientation};

    use crate::BlockId;

    struct Blocks {
        reg: BlockRegistry,
        stone: BlockId,
        lamp: BlockId,
        slab: BlockId,
        lamp_slab: BlockId,
    }

    fn blocks() -> Blocks {
        let mut reg = BlockRegistry::default();
        let bottom = vec![BlockBox::new([0, 0, 0], [16, 16, 8])];
        let stone = reg.register(BlockDef::new("test:stone", true, [0; 3]).with_material("test:stone")).unwrap();
        let lamp = reg.register(BlockDef::new("test:lamp", true, [0; 3]).with_light([255, 200, 130])).unwrap();
        let slab = reg.register(BlockDef::new("test:slab", true, [0; 3]).with_shape(bottom.clone())).unwrap();
        let lamp_slab =
            reg.register(BlockDef::new("test:lamp_slab", true, [0; 3]).with_shape(bottom).with_light([0, 0, 150])).unwrap();
        Blocks { reg, stone, lamp, slab, lamp_slab }
    }

    fn fill(grid: &mut Grid, min: [i64; 3], max: [i64; 3], block: BlockId) {
        for z in min[2]..max[2] {
            for y in min[1]..max[1] {
                for x in min[0]..max[0] {
                    grid.set_block(BoxelPos::new(x, y, z), block);
                }
            }
        }
    }

    /// Chunk (0, 0, 0) all stone, with a hollow from 8 to 24 on every axis: no light gets in.
    fn cave(b: &Blocks) -> Grid {
        let mut grid = Grid::default();
        fill(&mut grid, [0; 3], [32; 3], b.stone);
        fill(&mut grid, [8; 3], [24; 3], BlockId::EMPTY);
        grid
    }

    fn at(x: i64, y: i64, z: i64) -> BoxelPos {
        BoxelPos::new(x, y, z)
    }

    const RED: usize = 0;
    const SKY: usize = Light::SKY_CHANNEL;

    /// `level` in channel `c`, `steps` boxels on.
    fn away(c: usize, level: u8, steps: usize) -> u8 {
        (0..steps).fold(level, |l, _| Light::passed_on(c, l))
    }

    #[test]
    fn packs_four_channels() {
        let l = Light::new([1, 2, 3], 15);
        assert_eq!((l.rgb(), l.sky()), ([1, 2, 3], 15));
        assert_eq!(l.with_channel(1, 9).rgb(), [1, 9, 3]);
        assert_eq!(Light::SKY.sky(), Light::MAX);
        assert_eq!(Light::new([255, 0, 0], 255), Light::new([255, 0, 0], 0).with_channel(SKY, 255));
    }

    #[test]
    fn a_lamp_fades_one_level_a_boxel_in_each_color() {
        let b = blocks();
        let mut grid = cave(&b);
        grid.set_block(at(16, 16, 16), b.lamp);
        grid.relight_all(&b.reg);
        assert_eq!(grid.light(at(16, 16, 16)), Light::new([255, 200, 130], 0));
        for d in 1..=7 {
            let l = grid.light(at(16 + d as i64, 16, 16));
            assert_eq!(l.rgb(), [away(0, 255, d), away(1, 200, d), away(2, 130, d)], "{d} away");
            assert_eq!(l.sky(), 0, "no sky in a sealed cave");
        }
        // Around a corner counts by steps, not straight-line distance.
        assert_eq!(grid.light(at(19, 20, 16)).channel(RED), away(RED, 255, 7));
        assert_eq!(grid.light(at(16, 16, 25)), Light::DARK, "inside the rock");
    }

    #[test]
    fn a_lamps_color_holds_as_it_fades() {
        let b = blocks();
        let mut grid = cave(&b);
        grid.set_block(at(16, 16, 16), b.lamp);
        grid.relight_all(&b.reg);
        let hue = |l: Light| [l.channel(1) as f32 / l.channel(0) as f32, l.channel(2) as f32 / l.channel(0) as f32];
        let near = hue(grid.light(at(16, 16, 16)));
        let far = hue(grid.light(at(22, 16, 16)));
        for (n, f) in near.iter().zip(far) {
            assert!((n - f).abs() < 0.06, "the same warm color six boxels off: {near:?} against {far:?}");
        }
        assert!(away(RED, 255, 14) == 0 && away(RED, 255, 12) > 0, "full light reaches about 14 boxels");
    }

    #[test]
    fn a_dark_cave_stays_dark() {
        let b = blocks();
        let mut grid = cave(&b);
        grid.relight_all(&b.reg);
        assert_eq!(grid.light(at(16, 16, 16)), Light::DARK);
    }

    #[test]
    fn open_ground_is_fully_lit_and_a_roof_shades_by_distance_to_its_edge() {
        let b = blocks();
        let mut grid = Grid::default();
        fill(&mut grid, [-32, -32, 0], [64, 64, 1], b.stone);
        // A roof 11 wide on the floor's chunks, three boxels up, open on every side.
        fill(&mut grid, [0, 0, 4], [11, 11, 5], b.stone);
        grid.relight_all(&b.reg);
        assert_eq!(grid.light(at(-5, -5, 1)).sky(), Light::MAX, "open ground");
        assert_eq!(grid.light(at(0, 5, 0)).sky(), 0, "the floor itself");
        // Under the roof's middle, five steps in from its edge.
        assert_eq!(grid.light(at(5, 5, 2)).sky(), away(SKY, Light::MAX, 6));
        assert_eq!(grid.light(at(0, 5, 2)).sky(), away(SKY, Light::MAX, 1), "just under the edge");
        assert_eq!(grid.light(at(5, 5, 5)).sky(), Light::MAX, "on top of the roof");
    }

    #[test]
    fn sky_falls_down_a_shaft_without_dimming() {
        let b = blocks();
        let mut grid = Grid::default();
        // Rock below the shaft too, as a missing chunk under it would be open sky.
        fill(&mut grid, [0, 0, -64], [32, 32, 32], b.stone);
        fill(&mut grid, [16, 16, -32], [17, 17, 32], BlockId::EMPTY);
        grid.relight_all(&b.reg);
        assert_eq!(grid.light(at(16, 16, -32)).sky(), Light::MAX, "across a chunk border too");
        // Capping it darkens the whole shaft.
        grid.set_block(at(16, 16, 31), b.stone);
        grid.relight(&b.reg, &[at(16, 16, 31)]);
        assert_eq!(grid.light(at(16, 16, -32)).sky(), 0);
        assert_eq!(grid.light(at(16, 16, 0)).sky(), 0);
        grid.set_block(at(16, 16, 31), BlockId::EMPTY);
        grid.relight(&b.reg, &[at(16, 16, 31)]);
        assert_eq!(grid.light(at(16, 16, -32)).sky(), Light::MAX, "and opening it lights it again");
    }

    #[test]
    fn pieces_cover_only_the_faces_they_fill() {
        let b = blocks();
        let slab = |o: Orientation| Occupant::turned(b.slab, o);
        let cell = |occupants: &[Occupant]| CellLight::of(occupants, &b.reg);
        assert!(cell(&[Occupant::new(b.stone)]).closed());
        assert_eq!(cell(&[slab(Orientation::IDENTITY)]).covered, 1 << DOWN, "a bottom slab covers its bottom");
        let upside_down = Orientation::quarter_turns(0, 2);
        assert_eq!(cell(&[slab(upside_down)]).covered, 1 << UP);
        assert!(cell(&[slab(Orientation::IDENTITY), slab(upside_down)]).closed(), "two halves fill it");
        assert_eq!(cell(&[]).covered, 0);
        assert_eq!(cell(&[Occupant::new(b.lamp_slab), slab(upside_down)]).emits, Light::new([0, 0, 150], 0));
        // A panel against the south face, 4 texels thick, like a shutter.
        let mut panel = ShapeGrid::EMPTY;
        for z in 0..16 {
            for y in 0..4 {
                for x in 0..16 {
                    panel.set(x, y, z, true);
                }
            }
        }
        assert_eq!(covered_faces(&panel), 1 << 3);
    }

    #[test]
    fn a_hidden_piece_neither_stops_nor_gives_light() {
        let mut b = blocks();
        let full = ShapeGrid::FULL;
        let hidden_stone = b.reg.derive(Occupant::new(b.stone), &full, true).unwrap();
        let hidden_lamp = b.reg.derive(Occupant::new(b.lamp), &full, true).unwrap();
        let cell = CellLight::of(&[Occupant::new(hidden_stone)], &b.reg);
        assert_eq!(cell.covered, 0);
        assert_eq!(CellLight::of(&[Occupant::new(hidden_lamp)], &b.reg).emits, Light::DARK);
    }

    #[test]
    fn light_goes_through_a_slab_and_across_chunk_borders() {
        let b = blocks();
        let mut grid = Grid::default();
        // Two chunks of rock side by side, with a tunnel through both along x at y = z = 16.
        fill(&mut grid, [0, 0, 0], [64, 32, 32], b.stone);
        fill(&mut grid, [20, 16, 16], [44, 17, 17], BlockId::EMPTY);
        grid.set_block(at(20, 16, 16), b.lamp);
        grid.set_block(at(25, 16, 16), b.slab);
        grid.relight_all(&b.reg);
        assert_eq!(grid.light(at(26, 16, 16)).channel(RED), away(RED, 255, 6), "through the slab");
        assert_eq!(grid.light(at(32, 16, 16)).channel(RED), away(RED, 255, 12), "over the border");
        // A stone in the tunnel stops it.
        grid.set_block(at(25, 16, 16), b.stone);
        let touched = grid.relight(&b.reg, &[at(25, 16, 16)]);
        assert_eq!(grid.light(at(26, 16, 16)), Light::DARK);
        assert_eq!(grid.light(at(32, 16, 16)), Light::DARK);
        assert!(touched.contains(&ChunkPos::new(1, 0, 0)), "the next chunk's light changed");
        assert!(touched.contains(&ChunkPos::new(0, 0, 0)));
    }

    #[test]
    fn breaking_a_lamp_darkens_and_a_second_lamp_keeps_its_share() {
        let b = blocks();
        let mut grid = cave(&b);
        grid.set_block(at(10, 16, 16), b.lamp);
        grid.set_block(at(20, 16, 16), b.lamp);
        grid.relight_all(&b.reg);
        assert_eq!(grid.light(at(15, 16, 16)).channel(RED), away(RED, 255, 5));
        grid.set_block(at(10, 16, 16), BlockId::EMPTY);
        grid.relight(&b.reg, &[at(10, 16, 16)]);
        assert_eq!(grid.light(at(15, 16, 16)).channel(RED), away(RED, 255, 5), "the other lamp still reaches");
        assert_eq!(grid.light(at(10, 16, 16)).channel(RED), away(RED, 255, 10));
        assert_eq!(grid.light(at(8, 16, 16)).channel(RED), away(RED, 255, 12));
    }

    #[test]
    fn a_slab_roof_shades_and_a_slab_floor_is_lit() {
        let b = blocks();
        let mut grid = Grid::default();
        fill(&mut grid, [-32, -32, 0], [64, 64, 1], b.stone);
        fill(&mut grid, [0, 0, 4], [11, 11, 5], b.slab);
        grid.relight_all(&b.reg);
        assert_eq!(grid.light(at(5, 5, 4)).sky(), Light::MAX, "the slab's own boxel, open above");
        assert_eq!(grid.light(at(5, 5, 2)).sky(), away(SKY, Light::MAX, 6), "shaded like a full roof");
        // Breaking one lets the sky straight down; putting it back shades again.
        grid.set_block(at(5, 5, 4), BlockId::EMPTY);
        grid.relight(&b.reg, &[at(5, 5, 4)]);
        assert_eq!(grid.light(at(5, 5, 1)).sky(), Light::MAX);
        grid.set_block(at(5, 5, 4), b.slab);
        grid.relight(&b.reg, &[at(5, 5, 4)]);
        assert_eq!(grid.light(at(5, 5, 2)).sky(), away(SKY, Light::MAX, 6));
    }

    #[test]
    fn a_lamp_shines_out_of_a_face_it_covers() {
        let b = blocks();
        let mut grid = cave(&b);
        // A lamp slab lies on its bottom face; the boxel under it still gets its light.
        grid.set_block(at(16, 16, 16), b.lamp_slab);
        grid.relight_all(&b.reg);
        assert_eq!(grid.light(at(16, 16, 15)).channel(2), away(2, 150, 1));
        assert_eq!(grid.light(at(16, 16, 17)).channel(2), away(2, 150, 1));
    }

    #[test]
    fn a_panel_against_a_face_shuts_a_lamp_in() {
        let mut b = blocks();
        let mut panel = ShapeGrid::EMPTY;
        for z in 0..16 {
            for y in 0..4 {
                for x in 0..16 {
                    panel.set(x, y, z, true);
                }
            }
        }
        let stone = b.reg.material("test:stone").unwrap();
        let shutter = b.reg.shaped(stone, &panel).unwrap();
        let mut grid = cave(&b);
        // A lamp in the rock north of the cave, at the back of a niche one boxel deep: the
        // shutter's boxel, its panel against the face toward the cave.
        grid.set_block(at(16, 25, 16), b.lamp);
        grid.set_block(at(16, 24, 16), BlockId::EMPTY);
        grid.relight_all(&b.reg);
        assert_eq!(grid.light(at(16, 22, 16)).channel(RED), away(RED, 255, 3), "open: the lamp lights the cave");
        grid.set_block(at(16, 24, 16), shutter);
        grid.relight(&b.reg, &[at(16, 24, 16)]);
        assert_eq!(grid.light(at(16, 24, 16)).channel(RED), away(RED, 255, 1), "light gets into the shutter's boxel");
        assert_eq!(grid.light(at(16, 22, 16)), Light::DARK, "shut: none gets past it");
    }

    #[test]
    fn a_stair_at_a_niches_mouth_shuts_the_lamp_in_when_it_covers_the_mouth() {
        let mut b = blocks();
        let stair = b.reg.register(BlockDef::new("test:stair", true, [0; 3]).with_shape(vec![
            BlockBox::new([0, 0, 0], [16, 16, 8]),
            BlockBox::new([0, 8, 8], [16, 16, 16]),
        ]));
        let stair = stair.unwrap();
        for o in Orientation::all() {
            // A lamp two deep in the cave's north wall, an empty gap in front of it, and the
            // stair at the mouth.
            let mut grid = cave(&b);
            grid.set_block(at(16, 26, 16), b.lamp);
            grid.set_block(at(16, 25, 16), BlockId::EMPTY);
            grid.set_cell(at(16, 24, 16), &[Occupant::turned(stair, o)]);
            grid.relight_all(&b.reg);
            let covered = CellLight::of(&[Occupant::turned(stair, o)], &b.reg).covered;
            // Covering either face along the niche, toward the gap or the cave, shuts it.
            let shuts = covered & (1 << 2 | 1 << 3) != 0;
            let room = grid.light(at(16, 22, 16));
            assert_eq!(room == Light::DARK, shuts, "orientation {o:?} covers {covered:06b}");
        }
    }

    /// A small random number source, so the test needs nothing else.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: u64) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }

    #[test]
    fn relighting_after_edits_matches_lighting_from_scratch() {
        let b = blocks();
        let kinds = [BlockId::EMPTY, BlockId::EMPTY, b.stone, b.stone, b.slab, b.lamp, b.lamp_slab];
        let mut rng = Rng(7);
        let mut grid = Grid::default();
        // Rough ground across 2×2×2 chunks: rock below a bumpy surface, with some holes.
        for y in 0..64 {
            for x in 0..64 {
                let top = 20 + rng.below(16) as i64;
                fill(&mut grid, [x, y, 0], [x + 1, y + 1, top], b.stone);
            }
        }
        // Make sure every chunk of the box exists, so missing ones don't change the answer.
        for c in [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [0, 0, 1], [1, 0, 1], [0, 1, 1], [1, 1, 1]] {
            let p = at(c[0] * 32 + 1, c[1] * 32 + 1, c[2] * 32 + 1);
            let keep = grid.block(p);
            grid.set_block(p, b.stone);
            grid.set_block(p, keep);
        }
        grid.relight_all(&b.reg);
        for round in 0..30 {
            let mut changed = Vec::new();
            for _ in 0..1 + rng.below(12) {
                let p = at(rng.below(64) as i64, rng.below(64) as i64, rng.below(64) as i64);
                grid.set_block(p, kinds[rng.below(kinds.len() as u64) as usize]);
                changed.push(p);
            }
            grid.relight(&b.reg, &changed);
            let mut fresh = Grid::default();
            for (pos, _) in grid.chunks() {
                fresh.import_chunk(&grid.export_chunk(pos));
            }
            fresh.relight_all(&b.reg);
            for (pos, chunk) in grid.chunks() {
                let want = &fresh.chunk(pos).unwrap().light;
                for i in 0..CHUNK_VOLUME {
                    assert_eq!(
                        chunk.light.at(i),
                        want.at(i),
                        "round {round}, chunk {pos:?}, boxel {:?}",
                        coords(i)
                    );
                }
            }
        }
    }
}
