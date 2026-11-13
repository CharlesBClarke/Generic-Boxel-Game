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
//! boxel covers neither face, so light gets past it.
//!
//! A chunk that isn't loaded is open sky over its column's top and a dark wall below it (see
//! [`crate::tops`]), so the sky's light never leaks in underground from where the world hasn't
//! loaded; where no tops are known it's open sky all the way down.
//!
//! The rule has one settled state, so it isn't stepped tick by tick like the design doc's other
//! boxel rules: queues solve it straight there, all at once when terrain is made and around the
//! boxels that changed after edits, and it's settled again by the end of each tick. Stepping it
//! until it stopped changing would end in the same light.

use std::collections::{HashMap, HashSet, VecDeque};

use api::{BlockDef, BoxelPos, CHUNK_SIZE, CHUNK_VOLUME, ChunkPos, I64Vec3, NO_TOP, ShapeGrid};

use crate::tops::Tops;
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

    /// Stops light falling from the sky: nothing over it lights what's under it. What a
    /// column's top is the highest of (see [`crate::tops`]).
    fn stops_sky(self) -> bool {
        self.covers(UP) || self.covers(DOWN)
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
    /// A missing chunk, never written: open sky or a dark wall, by the tops.
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
    tops: &'a Tops,
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

    /// Where the boxel one step from `n` toward `d` is in the world.
    fn beside(&self, n: Node, d: usize) -> I64Vec3 {
        let [x, y, z] = coords(n.at as usize);
        self.positions[n.slot as usize].0 * CHUNK_SIZE + I64Vec3::new(x as i64, y as i64, z as i64) + I64Vec3::from_array(DIRS[d])
    }

    /// The light a boxel gets from missing chunks beside it, in channel `c`: the sky's, where
    /// the boxel beside it is open sky.
    fn sky_from_missing(&self, n: Node, c: usize) -> u8 {
        if c != Light::SKY_CHANNEL {
            return 0;
        }
        let info = self.info(n);
        let mut best = 0;
        for d in 0..6 {
            if !info.covers(d)
                && let Step::Missing = self.step(n, d)
                && self.tops.is_sky(self.beside(n, d))
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

/// Which boxel columns of the chunk at `pos` have open sky just over the chunk, when nothing
/// is loaded there: bit x of row y, as [`fall`] takes them.
fn open_over(tops: &Tops, pos: ChunkPos) -> [u32; SIZE] {
    let Some(column) = tops.column(pos.0.x, pos.0.y) else {
        return [u32::MAX; SIZE];
    };
    let above = (pos.0.z + 1) * CHUNK_SIZE;
    let mut open = [0u32; SIZE];
    for (y, row) in open.iter_mut().enumerate() {
        for x in 0..SIZE {
            if above > column[x + y * SIZE] {
                *row |= 1 << x;
            }
        }
    }
    open
}

/// Lights the chunk `data` as if nothing stood beside it: the sky over its column's top, dark
/// walls below (see [`crate::tops`]), from `tops` for its column and the four beside it. How
/// terrain chunks are lit, each on a thread of its own, before [`Grid::light_seams`] joins
/// them to the world.
pub fn light_alone(data: &mut crate::ChunkData, tops: &Tops, blocks: &BlockRegistry) {
    let mut grid = Grid { tops: tops.around(data.pos.0.x, data.pos.0.y), ..Grid::default() };
    grid.import_chunk(data);
    grid.relight_all(blocks);
    data.light = grid.chunks.swap_remove(0).light;
}

/// What a chunk that isn't loaded holds of one boxel column, as far as its world can tell
/// (see [`Grid::follow_tops`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unloaded {
    /// Something that stops the sky, no higher than this Z. Too high is safe.
    Top(i64),
    /// Nothing that stops the sky.
    Nothing,
}

/// What [`Grid::follow_tops`] moved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TopsMoved {
    /// Loaded boxels beside sky that opened or closed in chunks that aren't loaded: to
    /// relight along with the boxels that changed.
    pub relight: Vec<BoxelPos>,
    /// The chunk columns (X and Y) whose tops changed, with repeats.
    pub columns: Vec<(i64, i64)>,
}

/// How many chunks that aren't loaded and hold nothing a search for a column's top goes down
/// through before it takes the column for empty.
const EMPTY_ROWS: u32 = 256;

impl Grid {
    /// Brings the tops up to date after the boxels at `changed` changed: a boxel that stops the
    /// sky over its column's top is the new top, and where the top stopped doing so the next
    /// one down is. `unloaded` says what a chunk that isn't loaded holds of a boxel column,
    /// given the boxel X and Y and the chunk's Z. A boxel of `changed` that is the top and
    /// doesn't stop the sky is also how a top that was only a guess is checked. Does nothing
    /// where no tops are known.
    pub fn follow_tops(&mut self, blocks: &BlockRegistry, changed: &[BoxelPos], unloaded: &mut dyn FnMut(i64, i64, i64) -> Unloaded) -> TopsMoved {
        let mut moved = TopsMoved::default();
        if self.tops.is_empty() {
            return moved;
        }
        self.update_light_info(blocks);
        for &pos in changed {
            let p = pos.0;
            let Some(top) = self.tops.top(p.x, p.y) else { continue };
            if p.z < top {
                continue;
            }
            let (chunk, local) = pos.split();
            let Some(&slot) = self.slots.get(&chunk) else { continue };
            let stops = self.light_info[self.chunks[slot].cells.get(local).index()].stops_sky();
            let new = match (stops, p.z == top) {
                (true, false) => p.z,
                (false, true) => self.top_below(p.x, p.y, p.z - 1, unloaded),
                _ => continue,
            };
            self.tops.set_top(p.x, p.y, new);
            moved.columns.push((chunk.0.x, chunk.0.y));
            moved.relight.extend(self.beside_missing(p.x, p.y, top.min(new), top.max(new)));
        }
        moved
    }

    /// The boxels of the chunk at `pos` that show its column's tops are off, for
    /// [`Grid::follow_tops`] to put right: a top in the chunk that doesn't stop the sky after
    /// all, and with `raise` (for a chunk the tops were worked out without) a boxel over the top
    /// that does.
    pub fn tops_to_check(&mut self, blocks: &BlockRegistry, pos: ChunkPos, raise: bool) -> Vec<BoxelPos> {
        let (Some(column), Some(&slot)) = (self.tops.column(pos.0.x, pos.0.y).cloned(), self.slots.get(&pos)) else {
            return Vec::new();
        };
        self.update_light_info(blocks);
        let cells = &self.chunks[slot].cells;
        let origin = pos.0 * CHUNK_SIZE;
        let (bottom, highest) = (origin.z, origin.z + CHUNK_SIZE - 1);
        let uniform = cells.uniform().map(|id| self.light_info[id.index()].stops_sky());
        let mut out = Vec::new();
        for (i, &top) in column.iter().enumerate() {
            let (x, y) = (origin.x + (i % SIZE) as i64, origin.y + (i / SIZE) as i64);
            let stops = |z: i64| uniform.unwrap_or_else(|| self.light_info[cells.get(BoxelPos::new(x, y, z).split().1).index()].stops_sky());
            if raise
                && top < highest
                && let Some(z) = (bottom.max(top.saturating_add(1))..=highest).rev().find(|&z| stops(z))
            {
                out.push(BoxelPos::new(x, y, z));
            } else if (bottom..=highest).contains(&top) && !stops(top) {
                out.push(BoxelPos::new(x, y, top));
            }
        }
        out
    }

    /// The highest boxel at or below `z` in the boxel column `x, y` that stops the sky, or
    /// [`NO_TOP`].
    fn top_below(&self, x: i64, y: i64, mut z: i64, unloaded: &mut dyn FnMut(i64, i64, i64) -> Unloaded) -> i64 {
        let mut empty_rows = 0;
        loop {
            let (chunk, _) = BoxelPos::new(x, y, z).split();
            let bottom = chunk.0.z * CHUNK_SIZE;
            match self.slots.get(&chunk) {
                Some(&slot) => {
                    empty_rows = 0;
                    let cells = &self.chunks[slot].cells;
                    let stops = |at: i64| self.light_info[cells.get(BoxelPos::new(x, y, at).split().1).index()].stops_sky();
                    match cells.uniform() {
                        Some(id) if self.light_info[id.index()].stops_sky() => return z,
                        Some(_) => {}
                        None => {
                            if let Some(at) = (bottom..=z).rev().find(|&at| stops(at)) {
                                return at;
                            }
                        }
                    }
                }
                None => match unloaded(x, y, chunk.0.z) {
                    Unloaded::Top(top) => return top.min(z),
                    Unloaded::Nothing => {
                        empty_rows += 1;
                        if empty_rows > EMPTY_ROWS {
                            return NO_TOP;
                        }
                    }
                },
            }
            z = bottom - 1;
        }
    }

    /// The loaded boxels beside the boxels of the column `x, y` over `low` up to `high` that
    /// are in chunks that aren't loaded: what their being sky or not lights.
    fn beside_missing(&self, x: i64, y: i64, mut low: i64, mut high: i64) -> Vec<BoxelPos> {
        if high.saturating_sub(low) > 64 * CHUNK_SIZE {
            // Only as far as anything is loaded.
            let rows = self.positions.iter().map(|p| p.0.z);
            let (Some(first), Some(last)) = (rows.clone().min(), rows.max()) else { return Vec::new() };
            low = low.max((first - 1) * CHUNK_SIZE - 1);
            high = high.min((last + 2) * CHUNK_SIZE);
        }
        let mut out = Vec::new();
        let loaded = |p: I64Vec3| self.slots.contains_key(&BoxelPos(p).split().0);
        for z in low + 1..=high {
            let p = I64Vec3::new(x, y, z);
            if loaded(p) {
                continue;
            }
            out.extend(DIRS.iter().map(|d| p + I64Vec3::from_array(*d)).filter(|&q| loaded(q)).map(BoxelPos));
        }
        out
    }
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
            tops: &self.tops,
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
                // Nothing loaded above: the sky is open over the tops.
                open = open_over(&self.tops, pos);
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
                        if pass.level(n, Light::SKY_CHANNEL) < side && !pass.info(n).covers(d) && pass.tops.is_sky(pass.beside(n, d)) {
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
        self.touched_chunks(touched, &HashSet::new())
    }

    /// Settles the light where chunks lit apart meet the rest of the world. Each of `new` was
    /// lit as if nothing stood beside it (see [`light_alone`]), or with others of `new`; the
    /// rest was lit as if they were missing. Light crosses every seam between them both ways,
    /// and light that only came from sky a missing chunk stood for, where what's really there
    /// gives less, is taken away. Returns every chunk other than `new` whose light
    /// changed, and every chunk beside one whose border boxels did, in no particular order.
    pub fn light_seams(&mut self, blocks: &BlockRegistry, new: &[ChunkPos]) -> Vec<ChunkPos> {
        self.update_light_info(blocks);
        let fresh: HashSet<ChunkPos> = new.iter().copied().collect();
        let mut pass = self.pass(true);
        // Boxels lit by sky that came in through a seam's face, which isn't open any more.
        let mut darken: Vec<Node> = Vec::new();
        // Boxels whose light reaches further across a seam than what's there.
        let mut brighten: [VecDeque<Node>; 4] = Default::default();
        for &pos in new {
            let Some(&slot) = pass.slots.get(&pos) else { continue };
            for (d, dir) in DIRS.iter().enumerate() {
                let other = ChunkPos(pos.0 + I64Vec3::from_array(*dir));
                let Some(&far) = pass.slots.get(&other) else { continue };
                if fresh.contains(&other) && other.0.to_array() < pos.0.to_array() {
                    // A seam handled from the other side.
                    continue;
                }
                let axis = d / 2;
                let pair = |at: usize| {
                    let mut across = coords(at);
                    across[axis] = if d.is_multiple_of(2) { 0 } else { SIZE - 1 };
                    (Node { slot: slot as u32, at: at as u16 }, Node { slot: far as u32, at: index(across) as u16 })
                };
                // Whether `a` holds sky light a missing chunk toward `dir` may have given that
                // `b`, really there, gives less of: light to take back if the missing boxel
                // counted as open sky.
                let gave_more = |pass: &Pass, a: Node, b: Node, dir: usize| {
                    let (ia, ib) = (pass.info(a), pass.info(b));
                    let sky = pass.level(a, Light::SKY_CHANNEL);
                    // A missing chunk only ever gave sky: full from above, a step less from
                    // the sides and below.
                    let gave = if dir == UP { Light::MAX } else { Light::passed_on(Light::SKY_CHANNEL, Light::MAX) };
                    !ia.covers(dir)
                        && sky > 0
                        && (sky < Light::MAX || dir == UP)
                        && carried(ib, ia, dir ^ 1, pass.level(b, Light::SKY_CHANNEL), Light::SKY_CHANNEL) < gave
                };
                let reaches = |pass: &Pass, a: Node, b: Node, dir: usize, c: usize| {
                    carried(pass.info(a), pass.info(b), dir, pass.level(a, c), c) > pass.level(b, c)
                };
                // Two chunks each the same all through (open air, solid rock) meet the same
                // way at every boxel of the seam, so one pair says whether any needs anything.
                let same_all_through = |pass: &Pass, slot: usize| {
                    pass.chunks[slot].cells.uniform().is_some() && matches!(pass.chunks[slot].light, Layer::Uniform(_))
                };
                if same_all_through(&pass, slot) && same_all_through(&pass, far) {
                    let (n, m) = pair(face(d).next().expect("a face has boxels"));
                    let quiet = [(n, m, d), (m, n, d ^ 1)]
                        .into_iter()
                        .all(|(a, b, dir)| !gave_more(&pass, a, b, dir) && (0..4).all(|c| !reaches(&pass, a, b, dir, c)));
                    if quiet {
                        continue;
                    }
                }
                for at in face(d) {
                    let (n, m) = pair(at);
                    for (a, b, dir) in [(n, m, d), (m, n, d ^ 1)] {
                        if gave_more(&pass, a, b, dir) && pass.tops.is_sky(pass.beside(a, dir)) {
                            darken.push(a);
                        }
                        for (c, queue) in brighten.iter_mut().enumerate() {
                            if reaches(&pass, a, b, dir, c) {
                                queue.push_back(a);
                            }
                        }
                    }
                }
            }
        }
        darken.sort_unstable_by_key(|n| (n.slot, n.at));
        darken.dedup();
        if !darken.is_empty() {
            pass.relight(&darken, Light::SKY_CHANNEL);
        }
        for (c, queue) in brighten.into_iter().enumerate() {
            pass.spread(queue, c);
        }
        let touched = pass.touched.take().unwrap_or_default();
        let mut out = self.touched_chunks(touched, &fresh);
        out.retain(|pos| !fresh.contains(pos));
        for pos in new {
            if let Some(&slot) = self.slots.get(pos) {
                self.chunks[slot].light.compact();
            }
        }
        out
    }

    /// Every chunk but `skip` holding a boxel of `touched` whose light ended up different from
    /// what it was, and every chunk beside one whose border boxels did (as a face there reads
    /// them). Compacts the chunks whose light changed.
    fn touched_chunks(&mut self, touched: HashMap<Node, Light>, skip: &HashSet<ChunkPos>) -> Vec<ChunkPos> {
        // Each chunk whose light ended up different, with which of its faces the boxels that
        // changed lie on, so chunks whose border faces read them hear of it.
        let mut edged: HashSet<(u32, u8)> = HashSet::new();
        for (n, before) in touched {
            if self.chunks[n.slot as usize].light.at(n.at as usize) != before && !skip.contains(&self.positions[n.slot as usize]) {
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

    /// A world of 3×2 columns of 2 chunks each, rough ground with caves, overhangs, slabs and
    /// lamps across the seams.
    fn rough_world(b: &Blocks, seed: u64) -> Grid {
        let mut rng = Rng(seed);
        let kinds = [BlockId::EMPTY, b.stone, b.stone, b.slab, b.lamp, b.lamp_slab];
        let mut grid = Grid::default();
        for y in 0..64 {
            for x in 0..96 {
                let top = 20 + rng.below(16) as i64;
                fill(&mut grid, [x, y, 0], [x + 1, y + 1, top], b.stone);
            }
        }
        // Tunnels along X and Y through the rock, and roofs over open ground.
        fill(&mut grid, [0, 20, 8], [96, 23, 11], BlockId::EMPTY);
        fill(&mut grid, [40, 0, 12], [43, 64, 15], BlockId::EMPTY);
        fill(&mut grid, [20, 10, 44], [70, 40, 45], b.stone);
        for _ in 0..400 {
            let p = at(rng.below(96) as i64, rng.below(64) as i64, rng.below(64) as i64);
            grid.set_block(p, kinds[rng.below(kinds.len() as u64) as usize]);
        }
        for x in 0..3 {
            for y in 0..2 {
                for z in 0..2 {
                    let p = at(x * 32 + 1, y * 32 + 1, z * 32 + 1);
                    let keep = grid.block(p);
                    grid.set_block(p, b.stone);
                    grid.set_block(p, keep);
                }
            }
        }
        grid
    }

    #[test]
    fn columns_lit_apart_and_joined_match_lighting_together() {
        let b = blocks();
        for seed in 1..6 {
            let world = rough_world(&b, seed);
            let mut whole = Grid::default();
            for (pos, _) in world.chunks() {
                whole.import_chunk(&world.export_chunk(pos));
            }
            whole.relight_all(&b.reg);
            // Bring the columns in batches, in a shuffled order, as streaming does.
            let mut rng = Rng(seed * 31);
            let mut columns: Vec<(i64, i64)> = (0..3).flat_map(|x| (0..2).map(move |y| (x, y))).collect();
            for i in (1..columns.len()).rev() {
                columns.swap(i, rng.below(i as u64 + 1) as usize);
            }
            let mut joined = Grid::default();
            let mut rest = columns.as_slice();
            while !rest.is_empty() {
                let take = 1 + rng.below(rest.len().min(3) as u64) as usize;
                let (batch, later) = rest.split_at(take);
                rest = later;
                let mut new = Vec::new();
                for &(x, y) in batch {
                    let mut alone = Grid::default();
                    let chunks = [ChunkPos::new(x, y, 1), ChunkPos::new(x, y, 0)];
                    for pos in chunks {
                        alone.import_chunk(&world.export_chunk(pos));
                    }
                    alone.relight_all(&b.reg);
                    for pos in chunks {
                        joined.import_chunk(&alone.export_chunk(pos));
                        new.push(pos);
                    }
                }
                joined.light_seams(&b.reg, &new);
            }
            for (pos, chunk) in whole.chunks() {
                let got = &joined.chunk(pos).unwrap().light;
                for i in 0..CHUNK_VOLUME {
                    assert_eq!(got.at(i), chunk.light.at(i), "seed {seed}, chunk {pos:?}, boxel {:?}", coords(i));
                }
            }
        }
    }

    /// The tops of every chunk column `grid` has chunks in, worked out from all of them.
    fn tops_of(grid: &mut Grid, reg: &BlockRegistry) -> Tops {
        grid.update_light_info(reg);
        let mut tops = Tops::default();
        for (pos, chunk) in grid.chunks() {
            let key = (pos.0.x, pos.0.y);
            let mut column = tops.column(key.0, key.1).map_or([NO_TOP; api::TOPS_LEN], |c| **c);
            for i in 0..CHUNK_VOLUME {
                let [x, y, z] = coords(i);
                if grid.light_info[chunk.cells.at(i).index()].stops_sky() {
                    let top = &mut column[x + y * SIZE];
                    *top = (*top).max(pos.0.z * CHUNK_SIZE + z as i64);
                }
            }
            tops.set_column(key.0, key.1, std::sync::Arc::new(column));
        }
        tops
    }

    fn assert_same_light(got: &Grid, want: &Grid, what: &str) {
        for (pos, chunk) in want.chunks() {
            let got = &got.chunk(pos).unwrap().light;
            for i in 0..CHUNK_VOLUME {
                assert_eq!(got.at(i), chunk.light.at(i), "{what}, chunk {pos:?}, boxel {:?}", coords(i));
            }
        }
    }

    #[test]
    fn chunks_lit_alone_and_joined_match_lighting_together() {
        let b = blocks();
        for seed in 1..5 {
            let mut world = rough_world(&b, seed);
            // A row of empty chunks over it, and a shutter over the tops at a chunk's edge, which
            // stops the sky going sideways without stopping it falling.
            for x in 0..3 {
                for y in 0..2 {
                    world.import_chunk(&Grid::default().export_chunk(ChunkPos::new(x, y, 2)));
                }
            }
            let tops = tops_of(&mut world, &b.reg);
            let mut order: Vec<ChunkPos> = world.chunks().map(|(pos, _)| pos).collect();
            order.sort_by_key(|p| p.0.to_array());
            let mut rng = Rng(seed * 31);
            for i in (1..order.len()).rev() {
                order.swap(i, rng.below(i as u64 + 1) as usize);
            }
            let mut joined = Grid { tops: tops.clone(), ..Grid::default() };
            let mut together = Grid { tops: tops.clone(), ..Grid::default() };
            for (i, &pos) in order.iter().enumerate() {
                let mut data = world.export_chunk(pos);
                together.import_chunk(&data);
                light_alone(&mut data, &tops, &b.reg);
                joined.import_chunk(&data);
                joined.light_seams(&b.reg, &[pos]);
                // The same at every step on the way, not only once everything is in.
                if i % 4 == 3 || i + 1 == order.len() {
                    together.relight_all(&b.reg);
                    assert_same_light(&joined, &together, &format!("seed {seed}, {} chunks in", i + 1));
                }
            }
        }
    }

    #[test]
    fn a_shaft_stays_dark_beside_chunks_that_arent_loaded() {
        let b = blocks();
        // One chunk deep underground, with a shaft through it and a tunnel out of its side.
        let deep = ChunkPos::new(0, 0, -10);
        let o = deep.0.z * 32;
        let mut grid = Grid::default();
        fill(&mut grid, [0, 0, o], [32, 32, o + 32], b.stone);
        fill(&mut grid, [10, 10, o], [12, 12, o + 32], BlockId::EMPTY);
        fill(&mut grid, [0, 10, o + 10], [12, 12, o + 12], BlockId::EMPTY);
        grid.relight_all(&b.reg);
        assert_eq!(grid.light(at(10, 10, o + 31)).sky(), Light::MAX, "with no tops known it's open sky all round");
        assert!(grid.light(at(0, 10, o + 10)).sky() > 0);
        // With the ground known to be far above, nothing gets in.
        grid.tops.set_column(0, 0, std::sync::Arc::new([40; api::TOPS_LEN]));
        grid.tops.set_column(-1, 0, std::sync::Arc::new([40; api::TOPS_LEN]));
        grid.relight_all(&b.reg);
        for z in 0..32 {
            assert_eq!(grid.light(at(10, 10, o + z)), Light::DARK, "the shaft at {z}");
        }
        assert_eq!(grid.light(at(0, 10, o + 10)), Light::DARK, "the tunnel's mouth");
        assert_eq!(grid.light(at(10, 10, o + 40)), Light::DARK, "a missing chunk under the top reads dark");
        assert_eq!(grid.light(at(10, 10, 41)), Light::SKY, "and open sky over it");
    }

    #[test]
    fn a_roof_that_isnt_loaded_still_shades() {
        let b = blocks();
        let mut grid = Grid::default();
        fill(&mut grid, [0, 0, 0], [32, 32, 4], b.stone);
        // A roof far overhead, over the western half, in chunks that aren't loaded.
        let mut tops = [3; api::TOPS_LEN];
        for (i, top) in tops.iter_mut().enumerate() {
            if i % SIZE < 16 {
                *top = 500;
            }
        }
        // The same north and south, all roof to the west and open ground to the east.
        for (x, y, column) in [(0, 0, tops), (0, 1, tops), (0, -1, tops), (-1, 0, [500; api::TOPS_LEN]), (1, 0, [3; api::TOPS_LEN])] {
            grid.tops.set_column(x, y, std::sync::Arc::new(column));
        }
        grid.relight_all(&b.reg);
        assert_eq!(grid.light(at(20, 16, 4)).sky(), Light::MAX, "open ground");
        assert_eq!(grid.light(at(15, 16, 4)).sky(), away(Light::SKY_CHANNEL, Light::MAX, 1), "just under the roof's edge");
        assert_eq!(grid.light(at(0, 16, 4)).sky(), 0, "deep under it");
        assert_eq!(grid.light(at(15, 16, 31)).sky(), away(Light::SKY_CHANNEL, Light::MAX, 1), "all the way up");
    }

    #[test]
    fn tops_follow_placing_and_breaking() {
        let b = blocks();
        let mut grid = Grid::default();
        fill(&mut grid, [0, 0, 0], [32, 32, 4], b.stone);
        grid.import_chunk(&Grid::default().export_chunk(ChunkPos::new(0, 0, 1)));
        grid.tops = tops_of(&mut grid, &b.reg);
        grid.relight_all(&b.reg);
        let mut guess = |_: i64, _: i64, row: i64| Unloaded::Top((row + 1) * 32 - 1);
        let mut edit = |grid: &mut Grid, pos: BoxelPos, block: BlockId| {
            grid.set_block(pos, block);
            let moved = grid.follow_tops(&b.reg, &[pos], &mut guess);
            let mut changed = vec![pos];
            changed.extend(moved.relight.iter().copied());
            grid.relight(&b.reg, &changed);
            moved
        };
        // A block in the air is the new top, and shades the ground.
        let moved = edit(&mut grid, at(5, 5, 40), b.stone);
        assert_eq!((grid.tops.top(5, 5), moved.columns), (Some(40), vec![(0, 0)]));
        assert!(grid.light(at(5, 5, 4)).sky() < Light::MAX);
        // A block under the top doesn't move it.
        assert!(edit(&mut grid, at(5, 5, 20), b.stone).columns.is_empty());
        // Breaking the top finds the next one down, through the chunk between.
        edit(&mut grid, at(5, 5, 40), BlockId::EMPTY);
        assert_eq!(grid.tops.top(5, 5), Some(20));
        edit(&mut grid, at(5, 5, 20), BlockId::EMPTY);
        assert_eq!(grid.tops.top(5, 5), Some(3));
        assert_eq!(grid.light(at(5, 5, 4)).sky(), Light::MAX);
        // Digging through the floor runs into a chunk that isn't loaded: whoever knows says.
        for z in 0..4 {
            edit(&mut grid, at(5, 5, z), BlockId::EMPTY);
        }
        assert_eq!(grid.tops.top(5, 5), Some(-1), "the guess: the top boxel of the missing chunk");
        // A slab stops the sky under it but is lit itself.
        edit(&mut grid, at(9, 9, 50), b.slab);
        assert_eq!(grid.tops.top(9, 9), Some(50));
        assert_eq!(grid.light(at(9, 9, 50)).sky(), Light::MAX);
        // After all that, the light is what lighting from scratch gives.
        let mut scratch = Grid { tops: grid.tops.clone(), ..Grid::default() };
        for (pos, _) in grid.chunks() {
            scratch.import_chunk(&grid.export_chunk(pos));
        }
        scratch.relight_all(&b.reg);
        assert_same_light(&grid, &scratch, "after edits");
    }

    #[test]
    fn a_top_moving_in_chunks_that_arent_loaded_relights_what_is() {
        let b = blocks();
        // Two chunks side by side at ground level; over the eastern one's edge a tower stands
        // in a chunk that's loaded, far above a gap that isn't.
        let mut grid = Grid::default();
        fill(&mut grid, [0, 0, 0], [64, 32, 4], b.stone);
        grid.set_block(at(32, 16, 200), b.stone);
        grid.tops = tops_of(&mut grid, &b.reg);
        assert_eq!(grid.tops.top(32, 16), Some(200));
        grid.relight_all(&b.reg);
        // Breaking it opens the sky down the gap: the search goes down through chunks that
        // hold nothing to the ground.
        grid.set_block(at(32, 16, 200), BlockId::EMPTY);
        let mut nothing = |_: i64, _: i64, _: i64| Unloaded::Nothing;
        let moved = grid.follow_tops(&b.reg, &[at(32, 16, 200)], &mut nothing);
        assert_eq!(grid.tops.top(32, 16), Some(3));
        let mut changed = vec![at(32, 16, 200)];
        changed.extend(moved.relight);
        grid.relight(&b.reg, &changed);
        assert_eq!(grid.light(at(32, 16, 4)).sky(), Light::MAX);
        let mut scratch = Grid { tops: grid.tops.clone(), ..Grid::default() };
        for (pos, _) in grid.chunks() {
            scratch.import_chunk(&grid.export_chunk(pos));
        }
        scratch.relight_all(&b.reg);
        assert_same_light(&grid, &scratch, "after the tower went");
    }

    #[test]
    fn joining_columns_reports_the_old_chunks_whose_light_changed() {
        let b = blocks();
        let mut grid = Grid::default();
        // A tunnel from column 0 into column 1: column 0 alone lights the tunnel's end from the
        // open sky where column 1 is missing.
        fill(&mut grid, [0, 0, 0], [64, 32, 32], b.stone);
        fill(&mut grid, [10, 10, 10], [64, 13, 13], BlockId::EMPTY);
        let mut left = Grid::default();
        left.import_chunk(&grid.export_chunk(ChunkPos::new(0, 0, 0)));
        left.relight_all(&b.reg);
        assert!(left.light(at(31, 11, 11)).sky() > 0, "lit from the missing side");
        let mut right = Grid::default();
        right.import_chunk(&grid.export_chunk(ChunkPos::new(1, 0, 0)));
        right.relight_all(&b.reg);
        left.import_chunk(&right.export_chunk(ChunkPos::new(1, 0, 0)));
        let changed = left.light_seams(&b.reg, &[ChunkPos::new(1, 0, 0)]);
        assert!(changed.contains(&ChunkPos::new(0, 0, 0)));
        assert!(!changed.contains(&ChunkPos::new(1, 0, 0)), "new chunks aren't listed");
        assert_eq!(left.light(at(31, 11, 11)).sky(), 0, "a sealed tunnel is dark");
    }
}
