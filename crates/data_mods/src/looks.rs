//! Look recipes: how a `[looks.<name>]` table in `blocks.toml` becomes a solid volume.
//!
//! A recipe has one base pattern (`noise`, `bricks`, `planks` or `rings`) and may add a `top`
//! cover. Every pattern repeats exactly at the volume's edges, so blocks side by side meet
//! without seams, and the same recipe always makes the same volume.
//!
//! Faces show the layer just inside them, so a full block's faces show layers 0 and 15 and a
//! slab's cut shows 7 or 8. Lines like mortar and seams stay off those layers by default:
//! a whole face of mortar looks broken, not built.

use api::LookDef;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

const SIZE: usize = LookDef::SIZE;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// Whether the look turns with its blocks, like a log's rings; otherwise it stays lined up
    /// with the world.
    #[serde(default)]
    pub turns: bool,
    noise: Option<Noise>,
    bricks: Option<Bricks>,
    planks: Option<Planks>,
    rings: Option<Rings>,
    top: Option<Top>,
}

/// Speckle: smooth noise, each value picking one of the colors.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Noise {
    colors: Vec<[u8; 3]>,
    /// Texels across one blob: one number, or one per axis to stretch it.
    #[serde(default)]
    scale: Scale,
    #[serde(default)]
    seed: u32,
}

/// Rows of bricks, each row shifted half a brick, with mortar between them if it has a color.
/// Or, as `herringbone`, bricks half as wide as they are long, zigzagging in each course.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bricks {
    #[serde(default)]
    pattern: BrickPattern,
    /// Texels along a brick, mortar included, on both level axes.
    length: usize,
    /// Texels up a row, mortar included.
    height: usize,
    colors: Vec<[u8; 3]>,
    /// Without it, bricks are told apart by their colors alone.
    mortar: Option<[u8; 3]>,
    /// How far the pattern is shifted, in texels. By default the mortar runs a quarter brick
    /// in from the block's sides and half a row up from its bottom, clear of the layers faces
    /// usually show.
    offset: Option<[usize; 3]>,
    /// How much each texel's shade varies, as a fraction either way.
    #[serde(default = "brick_speckle")]
    speckle: f32,
    #[serde(default)]
    seed: u32,
}

#[derive(Deserialize, Default, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
enum BrickPattern {
    /// Rows of bricks square from above, each row shifted half a brick.
    #[default]
    Running,
    /// In each course, bricks lying along X and along Y meet at right angles in a zigzag.
    /// Each course shifts a brick's width along X from the one below.
    Herringbone,
}

/// Boards running along X, with a joint somewhere along each. Seams run half a board in from
/// the block's sides, and joints keep off its ends and middle. With `cross`, each layer of
/// boards turns a quarter turn from the one below, along X then Y, a whole block long with no
/// joints.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Planks {
    /// Texels across a board, level, seam included.
    width: usize,
    /// Texels up a board, seam included. By default the same as `width`.
    thickness: Option<usize>,
    /// The grain.
    colors: Vec<[u8; 3]>,
    /// Without it, boards are told apart by `shade` alone.
    seam: Option<[u8; 3]>,
    /// How much each board's shade differs from the next, as a fraction either way.
    #[serde(default)]
    shade: f32,
    #[serde(default)]
    cross: bool,
    #[serde(default)]
    seed: u32,
}

/// Growth rings around the Z axis, inside a shell of bark.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rings {
    /// One per ring, repeating outward.
    colors: Vec<[u8; 3]>,
    /// Texels from one ring to the next.
    spacing: f32,
    bark: Vec<[u8; 3]>,
    /// How thick the bark is, in texels.
    #[serde(default = "one")]
    bark_depth: usize,
    #[serde(default)]
    seed: u32,
}

/// A cover over the top of the base pattern, like grass on dirt.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Top {
    colors: Vec<[u8; 3]>,
    /// Texels deep at its thinnest.
    depth: usize,
    /// Up to this many texels deeper, varying across the top.
    #[serde(default)]
    ragged: usize,
    #[serde(default)]
    scale: Scale,
    #[serde(default)]
    seed: u32,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(untagged, expecting = "a number, or [x, y, z]")]
enum Scale {
    All(f32),
    Axes([f32; 3]),
}

impl Default for Scale {
    fn default() -> Self {
        Scale::All(1.0)
    }
}

impl Scale {
    fn axes(self) -> [f32; 3] {
        match self {
            Scale::All(s) => [s; 3],
            Scale::Axes(a) => a,
        }
    }
}

fn one() -> usize {
    1
}

fn brick_speckle() -> f32 {
    0.08
}

/// A shade fraction from a recipe, checked.
fn check_fraction(what: &str, f: f32) -> Result<(), String> {
    if (0.0..=0.5).contains(&f) { Ok(()) } else { Err(format!("{what} must be from 0 to 0.5, not {f}")) }
}

impl Recipe {
    /// Makes the volume, or says what's wrong with the recipe.
    pub fn generate(&self) -> Result<Vec<u8>, String> {
        let bases = [self.noise.is_some(), self.bricks.is_some(), self.planks.is_some(), self.rings.is_some()];
        if bases.iter().filter(|&&b| b).count() != 1 {
            return Err("a look needs exactly one of noise, bricks, planks or rings".into());
        }
        let mut texels = vec![[0u8; 3]; LookDef::VOLUME];
        if let Some(n) = &self.noise {
            n.fill(&mut texels)?;
        }
        if let Some(b) = &self.bricks {
            b.fill(&mut texels)?;
        }
        if let Some(p) = &self.planks {
            p.fill(&mut texels)?;
        }
        if let Some(r) = &self.rings {
            r.fill(&mut texels)?;
        }
        if let Some(t) = &self.top {
            t.fill(&mut texels)?;
        }
        Ok(texels.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect())
    }
}

/// Every texel's index and position, X fastest.
fn positions() -> impl Iterator<Item = (usize, [usize; 3])> {
    (0..LookDef::VOLUME).map(|i| (i, [i % SIZE, i / SIZE % SIZE, i / (SIZE * SIZE)]))
}

/// The region id of a texel on a line, like mortar or a seam, that has no neighbors.
const MORTAR: u32 = u32::MAX;

/// Gives each region of the volume (texels sharing an id in `ids`) one of `choices`, so that no
/// two regions that touch, across the volume's edges too, get the same one. Which coloring
/// comes out depends on `seed`. `None` if it can't be done with that many choices.
fn distinct(ids: &[u32], choices: usize, seed: u32) -> Option<HashMap<u32, usize>> {
    let mut regions: Vec<u32> = ids.iter().copied().filter(|&id| id != MORTAR).collect();
    regions.sort_unstable();
    regions.dedup();
    // One choice means nothing to keep apart.
    if choices <= 1 {
        return Some(regions.into_iter().map(|id| (id, 0)).collect());
    }
    let index: HashMap<u32, usize> = regions.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    let mut touching = vec![HashSet::new(); regions.len()];
    for (i, p) in positions() {
        for axis in 0..3 {
            let mut q = p;
            q[axis] = (q[axis] + 1) % SIZE;
            let (a, b) = (ids[i], ids[LookDef::index(q[0], q[1], q[2])]);
            if a != b && a != MORTAR && b != MORTAR {
                touching[index[&a]].insert(index[&b]);
                touching[index[&b]].insert(index[&a]);
            }
        }
    }
    // Colors the most hemmed-in region next (the most different choices already among its
    // neighbors), each time taking the first free choice from a seeded starting point. If
    // that paints itself into a corner, it starts over with the order shuffled differently.
    let n = regions.len();
    for attempt in 0..64u32 {
        let mut chosen: Vec<Option<usize>> = vec![None; n];
        let tie = |r: usize| hash([regions[r], attempt, 0], seed ^ 0xc01d);
        let done = (0..n).all(|_| {
            let used = |r: usize| touching[r].iter().filter_map(|&t| chosen[t]).collect::<HashSet<_>>();
            let at = (0..n)
                .filter(|&r| chosen[r].is_none())
                .max_by_key(|&r| (used(r).len(), touching[r].len(), tie(r)))
                .unwrap();
            let used = used(at);
            let start = tie(at) as usize;
            chosen[at] = (0..choices).map(|t| (start + t) % choices).find(|c| !used.contains(c));
            chosen[at].is_some()
        });
        if done {
            return Some(regions.into_iter().zip(chosen.into_iter().flatten()).collect());
        }
    }
    None
}

fn check_colors(what: &str, colors: &[[u8; 3]]) -> Result<(), String> {
    if colors.is_empty() { Err(format!("{what} needs at least one color")) } else { Ok(()) }
}

/// Whether `n` texels tile the volume exactly.
fn check_divides(what: &str, n: usize) -> Result<(), String> {
    if n == 0 || !SIZE.is_multiple_of(n) {
        Err(format!("{what} must divide {SIZE} evenly, not {n}"))
    } else {
        Ok(())
    }
}

impl Noise {
    fn fill(&self, texels: &mut [[u8; 3]]) -> Result<(), String> {
        check_colors("noise", &self.colors)?;
        let field = Field::new(self.scale, self.seed)?;
        for (i, p) in positions() {
            texels[i] = pick(&self.colors, field.at(p));
        }
        Ok(())
    }
}

impl Bricks {
    fn fill(&self, texels: &mut [[u8; 3]]) -> Result<(), String> {
        check_colors("bricks", &self.colors)?;
        check_divides("bricks length", self.length)?;
        check_divides("bricks height", self.height)?;
        if self.length < 2 || self.height < 2 {
            return Err("bricks need a length and height of at least 2, to fit the mortar".into());
        }
        check_fraction("bricks speckle", self.speckle)?;
        let ids = match self.pattern {
            BrickPattern::Running => self.running(),
            BrickPattern::Herringbone => self.herringbone()?,
        };
        let pick = distinct(&ids, self.colors.len(), self.seed)
            .ok_or_else(|| format!("bricks need more than {} colors, so neighbors always differ", self.colors.len()))?;
        for (i, [x, y, z]) in positions() {
            texels[i] = match (ids[i], self.mortar) {
                (MORTAR, Some(mortar)) => mortar,
                (id, _) => {
                    // A little speckle, so a brick isn't one flat color.
                    let u = unit(hash([x as u32, y as u32, z as u32], self.seed ^ 0x5bd1));
                    shade(self.colors[pick[&id]], 1.0 + self.speckle * (2.0 * u - 1.0))
                }
            };
        }
        Ok(())
    }

    /// Which brick each texel is in, or `MORTAR`, for running bond.
    fn running(&self) -> Vec<u32> {
        let quarter = self.length - self.length / 4;
        let [ox, oy, oz] = self.offset.unwrap_or([quarter, quarter, self.height / 2]);
        positions()
            .map(|(_, [x, y, z])| {
                let z = z + oz;
                let row = z / self.height;
                // Every other row shifts half a brick, on both level axes.
                let shift = if row % 2 == 1 { self.length / 2 } else { 0 };
                let (bx, by) = ((x + ox + shift) % SIZE, (y + oy + shift) % SIZE);
                let joint = z % self.height == 0 || bx % self.length == 0 || by % self.length == 0;
                if joint && self.mortar.is_some() {
                    MORTAR
                } else {
                    let row = row % (SIZE / self.height);
                    ((row * SIZE + by / self.length) * SIZE + bx / self.length) as u32
                }
            })
            .collect()
    }

    /// Which brick each texel is in, for herringbone. Bricks are `length` by half that by
    /// `height`. Within a course, in cells a brick's width across, the pattern repeats every
    /// four cells along X, along Y and diagonally: by `(cx - cy) % 4`, a cell is the west half
    /// of a brick along X (0) or its east half (1), the north half of a brick along Y (2) or
    /// its south half (3).
    fn herringbone(&self) -> Result<Vec<u32>, String> {
        let unit = self.length / 2;
        if self.mortar.is_some() {
            return Err("herringbone bricks don't have mortar yet".into());
        }
        if unit == 0 || self.length % 2 == 1 || !(SIZE / unit).is_multiple_of(4) {
            return Err(format!("herringbone bricks need a length of 2, 4 or 8, not {}", self.length));
        }
        let cells = SIZE / unit;
        let courses = SIZE / self.height;
        let [ox, oy, oz] = self.offset.unwrap_or([0, 0, self.height / 2]);
        Ok(positions()
            .map(|(_, [x, y, z])| {
                let course = (z + oz) / self.height % courses;
                let cx = (x + ox) / unit + course;
                let cy = (y + oy) / unit;
                let (kind, sx, sy) = match (cx + 4 * cells - cy % cells) % 4 {
                    0 => (0, cx, cy),
                    1 => (0, cx + cells - 1, cy),
                    2 => (1, cx, cy + cells - 1),
                    _ => (1, cx, cy),
                };
                let (sx, sy) = (sx % cells, sy % cells);
                (((course * cells + sy) * cells + sx) * 2 + kind) as u32
            })
            .collect())
    }
}

impl Planks {
    fn fill(&self, texels: &mut [[u8; 3]]) -> Result<(), String> {
        check_colors("planks", &self.colors)?;
        let thickness = self.thickness.unwrap_or(self.width);
        check_divides("planks width", self.width)?;
        check_divides("planks thickness", thickness)?;
        if self.seam.is_some() && (self.width < 2 || thickness < 2) {
            return Err("planks need a width and thickness of at least 2, to fit the seam".into());
        }
        if self.cross && !(SIZE / thickness).is_multiple_of(2) {
            return Err("crossed planks need an even number of layers, so they repeat".into());
        }
        check_fraction("planks shade", self.shade)?;
        // Grain runs along the boards: long blobs along them, thin ones across.
        let grain = [
            Field::new(Scale::Axes([8.0, 1.0, 1.0]), self.seed)?,
            Field::new(Scale::Axes([1.0, 8.0, 1.0]), self.seed)?,
        ];
        // Each texel's board (or `MORTAR` on a seam), which way it runs, and where its grain
        // starts.
        let layout: Vec<(u32, usize)> = positions()
            .map(|(_, [x, y, z])| {
                // Seams keep half a board in from the block's sides.
                let az = (z + thickness / 2) % SIZE;
                let layer = az / thickness;
                // Which way the board runs: 0 along X, 1 along Y.
                let along = if self.cross { layer % 2 } else { 0 };
                let (a, c) = if along == 0 { (x, y) } else { (y, x) };
                let ac = (c + self.width / 2) % SIZE;
                let board = ac / self.width;
                // A joint in the first or second half of the block, never on its ends or at
                // its middle. Crossed boards run the whole block instead.
                let (joint, piece) = if self.cross {
                    (None, 0)
                } else {
                    let h = hash([board as u32, layer as u32, 0], self.seed) as usize;
                    let joint = 1 + h % (SIZE / 2 - 2) + (h >> 8) % 2 * (SIZE / 2);
                    (Some(joint), usize::from(a >= joint))
                };
                let seam = ac.is_multiple_of(self.width) || az.is_multiple_of(thickness) || joint == Some(a);
                let id = if seam && self.seam.is_some() { MORTAR } else { ((layer * SIZE + board) * 2 + piece) as u32 };
                (id, along)
            })
            .collect();
        let ids: Vec<u32> = layout.iter().map(|&(id, _)| id).collect();
        // Four shade steps, from `shade` darker to `shade` lighter, so neighbors always differ
        // by at least two thirds of it: a board touches the boards beside it and those across
        // the layers above and below, so three steps aren't always enough.
        let steps = if self.shade > 0.0 { 4 } else { 1 };
        let step = distinct(&ids, steps, self.seed).ok_or("planks boards can't be kept apart by shade")?;
        for (i, [x, y, z]) in positions() {
            let (id, along) = layout[i];
            texels[i] = match (id, self.seam) {
                (MORTAR, Some(seam)) => seam,
                (id, _) => {
                    // Each board's grain from its own part of the field, so neighbors differ.
                    let offset = (hash([id, 0, 1], self.seed) % SIZE as u32) as usize;
                    let p = if along == 0 { [(x + offset) % SIZE, y, z] } else { [x, (y + offset) % SIZE, z] };
                    let tint = if steps == 1 { 0.0 } else { step[&id] as f32 / (steps - 1) as f32 * 2.0 - 1.0 };
                    shade(pick(&self.colors, grain[along].at(p)), 1.0 + self.shade * tint)
                }
            };
        }
        Ok(())
    }
}

impl Rings {
    fn fill(&self, texels: &mut [[u8; 3]]) -> Result<(), String> {
        check_colors("rings", &self.colors)?;
        check_colors("rings bark", &self.bark)?;
        if !(self.spacing.is_finite() && self.spacing > 0.0) {
            return Err(format!("rings spacing must be more than 0, not {}", self.spacing));
        }
        if self.bark_depth >= SIZE / 2 {
            return Err(format!("rings bark_depth must be below {}, not {}", SIZE / 2, self.bark_depth));
        }
        let wobble = Field::new(Scale::All(4.0), self.seed)?;
        let bark = Field::new(Scale::Axes([1.0, 1.0, 4.0]), self.seed ^ 0xb4a2)?;
        let center = SIZE as f32 / 2.0;
        let (d, far) = (self.bark_depth, SIZE - self.bark_depth);
        for (i, [x, y, z]) in positions() {
            texels[i] = if x < d || y < d || x >= far || y >= far {
                pick(&self.bark, bark.at([x, y, z]))
            } else {
                let (dx, dy) = (x as f32 + 0.5 - center, y as f32 + 0.5 - center);
                let r = (dx * dx + dy * dy).sqrt() + (wobble.at([x, y, z]) - 0.5) * self.spacing * 0.8;
                self.colors[(r.max(0.0) / self.spacing) as usize % self.colors.len()]
            };
        }
        Ok(())
    }
}

impl Top {
    fn fill(&self, texels: &mut [[u8; 3]]) -> Result<(), String> {
        check_colors("top", &self.colors)?;
        if self.depth == 0 || self.depth + self.ragged > SIZE {
            return Err(format!("top depth must be at least 1, and depth plus ragged at most {SIZE}"));
        }
        let field = Field::new(self.scale, self.seed)?;
        // The edge's raggedness varies across the top only.
        let edge = Field::new(Scale::All(3.0), self.seed ^ 0x70b5)?;
        for (i, [x, y, z]) in positions() {
            let extra = (edge.at([x, y, 0]) * (self.ragged + 1) as f32) as usize;
            if z >= SIZE - (self.depth + extra.min(self.ragged)) {
                texels[i] = pick(&self.colors, field.at([x, y, z]));
            }
        }
        Ok(())
    }
}

/// One of `colors` for a noise value from 0 to 1. Smooth noise bunches around the middle, so
/// the value is stretched first to give the outer colors a fair share.
fn pick(colors: &[[u8; 3]], value: f32) -> [u8; 3] {
    let stretched = ((value - 0.5) * 1.8 + 0.5).clamp(0.0, 0.9999);
    colors[(stretched * colors.len() as f32) as usize]
}

fn shade(color: [u8; 3], by: f32) -> [u8; 3] {
    color.map(|c| (c as f32 * by).round().clamp(0.0, 255.0) as u8)
}

/// Smooth value noise that repeats every `SIZE` texels on each axis: two octaves of random
/// values on a lattice, blended between lattice points.
struct Field {
    /// Lattice cells across the volume, per axis.
    cells: [u32; 3],
    seed: u32,
}

impl Field {
    fn new(scale: Scale, seed: u32) -> Result<Self, String> {
        let axes = scale.axes();
        if axes.iter().any(|s| !(s.is_finite() && *s >= 1.0 && *s <= SIZE as f32)) {
            return Err(format!("scale must be from 1 to {SIZE}, not {axes:?}"));
        }
        // A whole number of cells across, so the noise wraps.
        Ok(Field { cells: axes.map(|s| ((SIZE as f32 / s).round() as u32).max(1)), seed })
    }

    /// The value at texel `p`, from 0 to 1.
    fn at(&self, p: [usize; 3]) -> f32 {
        let fine = self.cells.map(|c| (c * 2).min(SIZE as u32));
        0.65 * self.octave(self.cells, self.seed, p) + 0.35 * self.octave(fine, self.seed.wrapping_add(1), p)
    }

    fn octave(&self, cells: [u32; 3], seed: u32, p: [usize; 3]) -> f32 {
        // The texel's center, in lattice cells.
        let q: [f32; 3] = [0, 1, 2].map(|i| (p[i] as f32 + 0.5) * cells[i] as f32 / SIZE as f32);
        let base = q.map(|c| c.floor() as u32);
        let t = [0, 1, 2].map(|i| smooth(q[i] - base[i] as f32));
        let mut sum = 0.0;
        for corner in 0..8u32 {
            let d = [corner & 1, corner >> 1 & 1, corner >> 2 & 1];
            let at = [0, 1, 2].map(|i| (base[i] + d[i]) % cells[i]);
            let weight: f32 = (0..3).map(|i| if d[i] == 1 { t[i] } else { 1.0 - t[i] }).product();
            sum += weight * unit(hash(at, seed));
        }
        sum
    }
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// A well-mixed number for a lattice point.
fn hash(p: [u32; 3], seed: u32) -> u32 {
    let mut h = seed.wrapping_mul(0x9e37_79b9) ^ p[0].wrapping_mul(0x85eb_ca6b);
    h ^= p[1].wrapping_mul(0xc2b2_ae35).rotate_left(11);
    h ^= p[2].wrapping_mul(0x27d4_eb2f).rotate_left(22);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

/// A hash as a number from 0 to just below 1.
fn unit(h: u32) -> f32 {
    (h >> 8) as f32 / (1u32 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn recipe(toml: &str) -> Recipe {
        toml::from_str(toml).unwrap()
    }

    fn colors(rgba: &[u8]) -> HashSet<[u8; 3]> {
        rgba.as_chunks::<4>().0.iter().map(|px| [px[0], px[1], px[2]]).collect()
    }

    fn texel(rgba: &[u8], x: usize, y: usize, z: usize) -> [u8; 3] {
        let at = 4 * LookDef::index(x, y, z);
        [rgba[at], rgba[at + 1], rgba[at + 2]]
    }

    const STONE: &str = "noise = { colors = [[100, 100, 100], [120, 120, 120], [140, 140, 140]], scale = 3, seed = 7 }";

    #[test]
    fn the_same_recipe_makes_the_same_volume() {
        let a = recipe(STONE).generate().unwrap();
        assert_eq!(a, recipe(STONE).generate().unwrap());
        assert_eq!(a.len(), LookDef::VOLUME * 4);
        assert!(a.as_chunks::<4>().0.iter().all(|px| px[3] == 255), "opaque");
        let reseeded = recipe(&STONE.replace("seed = 7", "seed = 8")).generate().unwrap();
        assert_ne!(a, reseeded, "the seed changes the speckle");
    }

    #[test]
    fn noise_uses_every_color_and_only_those() {
        let v = recipe(STONE).generate().unwrap();
        assert_eq!(colors(&v), HashSet::from([[100, 100, 100], [120, 120, 120], [140, 140, 140]]));
    }

    #[test]
    fn noise_wraps_so_neighbors_meet_smoothly() {
        // Across the volume's edge, from texel 15 to the next block's texel 0, the noise changes
        // no more than between any two neighbors inside it.
        let field = Field::new(Scale::All(4.0), 3).unwrap();
        let step = |a: [usize; 3], b: [usize; 3]| (field.at(a) - field.at(b)).abs();
        let mut inside: f32 = 0.0;
        let mut across: f32 = 0.0;
        for y in 0..SIZE {
            for z in 0..SIZE {
                for x in 0..SIZE - 1 {
                    inside = inside.max(step([x, y, z], [x + 1, y, z]));
                }
                across = across.max(step([SIZE - 1, y, z], [0, y, z]));
            }
        }
        assert!(across <= inside + 1e-6, "across the edge {across}, inside {inside}");
    }

    #[test]
    fn a_top_covers_only_the_top() {
        let grass = recipe(
            "noise = { colors = [[120, 80, 50]] }\ntop = { colors = [[60, 140, 40]], depth = 2, ragged = 2 }",
        )
        .generate()
        .unwrap();
        let (dirt, green) = ([120, 80, 50], [60, 140, 40]);
        for x in 0..SIZE {
            for y in 0..SIZE {
                assert_eq!(texel(&grass, x, y, SIZE - 1), green, "the top layer is all cover");
                assert_eq!(texel(&grass, x, y, SIZE - 2), green, "at least depth deep");
                assert_eq!(texel(&grass, x, y, SIZE - 5), dirt, "no deeper than depth plus ragged");
            }
        }
        let ragged = (0..SIZE * SIZE).any(|i| texel(&grass, i % SIZE, i / SIZE, SIZE - 3) == green)
            && (0..SIZE * SIZE).any(|i| texel(&grass, i % SIZE, i / SIZE, SIZE - 3) == dirt);
        assert!(ragged, "the edge varies across the top");
    }

    const BRICKS: &str = "bricks = { length = 8, height = 4, colors = [[150, 60, 50]], mortar = [200, 200, 190] }";

    #[test]
    fn bricks_have_mortar_between_them() {
        let v = recipe(BRICKS).generate().unwrap();
        let mortar = [200, 200, 190];
        // Every fourth layer is mortar, half a row up from the bottom.
        assert!((0..SIZE).all(|x| texel(&v, x, 5, 2) == mortar && texel(&v, x, 5, 6) == mortar));
        assert_ne!(texel(&v, 5, 5, 0), mortar);
        // Joints a quarter brick in, shifting half a brick each row.
        assert_eq!(texel(&v, 2, 5, 1), mortar, "a joint in the first row");
        assert_ne!(texel(&v, 5, 5, 1), mortar, "the middle of a brick");
        assert_eq!(texel(&v, 6, 5, 4), mortar, "the next row's joint, half a brick over");
        assert!(colors(&v).len() > 2, "bricks are speckled");
    }

    /// The layers faces of full blocks and slabs show, along `axis`.
    fn face_layers(v: &[u8], axis: usize) -> impl Iterator<Item = (usize, [u8; 3])> + '_ {
        [0, 7, 8, SIZE - 1].into_iter().flat_map(move |layer| {
            (0..SIZE * SIZE).map(move |i| {
                let (a, b) = (i % SIZE, i / SIZE);
                let p = match axis {
                    0 => [layer, a, b],
                    1 => [a, layer, b],
                    _ => [a, b, layer],
                };
                (layer, texel(v, p[0], p[1], p[2]))
            })
        })
    }

    #[test]
    fn mortar_and_seams_stay_off_the_layers_faces_show() {
        let planks = "planks = { width = 4, colors = [[180, 140, 90]], seam = [90, 60, 40] }";
        for (toml, line) in [(BRICKS, [200, 200, 190]), (planks, [90, 60, 40])] {
            let v = recipe(toml).generate().unwrap();
            for axis in 0..3 {
                for layer in [0, 7, 8, SIZE - 1] {
                    let lines = face_layers(&v, axis).filter(|&(l, c)| l == layer && c == line).count();
                    assert!(lines < SIZE * SIZE / 2, "{toml}: layer {layer} on axis {axis} is mostly lines");
                }
            }
        }
    }

    /// How many patches of one color the volume has, counting patches that meet across its
    /// edges as one, as they do between blocks.
    fn patches(v: &[u8]) -> usize {
        let mut seen = vec![false; LookDef::VOLUME];
        let mut count = 0;
        for (start, _) in positions() {
            if seen[start] {
                continue;
            }
            count += 1;
            seen[start] = true;
            let mut todo = vec![start];
            while let Some(i) = todo.pop() {
                let p = [i % SIZE, i / SIZE % SIZE, i / (SIZE * SIZE)];
                let c = texel(v, p[0], p[1], p[2]);
                for axis in 0..3 {
                    for step in [1, SIZE - 1] {
                        let mut q = p;
                        q[axis] = (q[axis] + step) % SIZE;
                        let j = LookDef::index(q[0], q[1], q[2]);
                        if !seen[j] && texel(v, q[0], q[1], q[2]) == c {
                            seen[j] = true;
                            todo.push(j);
                        }
                    }
                }
            }
        }
        count
    }

    const FOUR_REDS: &str = "colors = [[150, 60, 50], [130, 50, 40], [170, 70, 60], [110, 40, 30]]";

    #[test]
    fn neighboring_bricks_never_share_a_color() {
        let v = recipe(&format!("bricks = {{ length = 8, height = 4, {FOUR_REDS}, speckle = 0 }}")).generate().unwrap();
        // 2 by 2 bricks in each of 4 rows: if any two that touch matched, they'd be one patch.
        assert_eq!(patches(&v), 16);
        for seed in 0..20 {
            let v = recipe(&format!("bricks = {{ length = 4, height = 2, {FOUR_REDS}, speckle = 0, seed = {seed} }}"))
                .generate()
                .unwrap();
            assert_eq!(patches(&v), 4 * 4 * 8, "seed {seed}");
        }
    }

    const HERRINGBONE: &str = "bricks = { pattern = \"herringbone\", length = 8, height = 4, speckle = 0";

    #[test]
    fn herringbone_bricks_zigzag_both_ways_in_a_course() {
        let v = recipe(&format!("{HERRINGBONE}, {FOUR_REDS} }}")).generate().unwrap();
        // In the bottom course, a brick lying along X: 8 along X by 4 along Y, then the next.
        let same = |a: [usize; 3], b: [usize; 3]| texel(&v, a[0], a[1], a[2]) == texel(&v, b[0], b[1], b[2]);
        assert!((0..8).all(|x| same([x, 0, 0], [0, 3, 0])), "a brick along X");
        assert!(!same([0, 0, 0], [8, 0, 0]) && !same([0, 0, 0], [0, 4, 0]));
        // Beside it, one lying along Y: 4 along X by 8 along Y.
        assert!((0..8).all(|y| same([12, y, 0], [15, 7, 0])), "a brick along Y");
        assert!(!same([12, 0, 0], [12, 8, 0]) && !same([12, 0, 0], [11, 0, 0]));
        // Courses 4 tall, half a course up, each shifted a brick's width along X.
        assert!(same([0, 0, 0], [0, 0, 1]) && !same([0, 0, 1], [0, 0, 2]));
        // In the course above, the brick along X starts a brick's width west, across the edge.
        assert!(!same([0, 0, 3], [7, 0, 3]), "not where the one below is");
        assert!((0..8).all(|x| same([(12 + x) % SIZE, 0, 3], [12, 3, 3])), "shifted along X");
        // 8 bricks a course, 4 courses: touching bricks never match, or they'd be one patch.
        assert_eq!(patches(&v), 32);
        for seed in 0..20 {
            let v = recipe(&format!("{HERRINGBONE}, {FOUR_REDS}, seed = {seed} }}")).generate().unwrap();
            assert_eq!(patches(&v), 32, "seed {seed}");
        }
    }

    #[test]
    fn herringbone_needs_a_length_that_repeats() {
        let err = |toml: &str| recipe(toml).generate().unwrap_err();
        assert!(err("bricks = { pattern = \"herringbone\", length = 16, height = 4, colors = [[1, 1, 1]] }")
            .contains("length of 2, 4 or 8"));
        assert!(err("bricks = { pattern = \"herringbone\", length = 8, height = 4, colors = [[1, 1, 1]], mortar = [0, 0, 0] }")
            .contains("mortar"));
    }

    #[test]
    fn bricks_without_enough_colors_to_keep_neighbors_apart_are_an_error() {
        // Each brick touches four in the row above, which touch each other, so it takes four.
        let err = recipe("bricks = { length = 8, height = 4, colors = [[150, 60, 50], [90, 90, 90], [1, 2, 3]] }")
            .generate()
            .unwrap_err();
        assert!(err.contains("more than 3 colors"), "{err}");
    }

    #[test]
    fn rings_sit_inside_a_shell_of_bark() {
        let v = recipe(
            "rings = { colors = [[200, 160, 100], [170, 130, 80]], spacing = 2, bark = [[80, 60, 40]], bark_depth = 1 }",
        )
        .generate()
        .unwrap();
        let bark = [80, 60, 40];
        for z in 0..SIZE {
            for i in 0..SIZE {
                for p in [[i, 0], [i, SIZE - 1], [0, i], [SIZE - 1, i]] {
                    assert_eq!(texel(&v, p[0], p[1], z), bark, "the shell is bark");
                }
            }
            for x in 1..SIZE - 1 {
                for y in 1..SIZE - 1 {
                    assert_ne!(texel(&v, x, y, z), bark, "inside is rings");
                }
            }
        }
        assert!(colors(&v).len() == 3, "both ring colors show");
    }

    #[test]
    fn planks_have_seams_between_boards() {
        let v = recipe("planks = { width = 4, colors = [[180, 140, 90], [160, 120, 75]], seam = [90, 60, 40] }")
            .generate()
            .unwrap();
        let seam = [90, 60, 40];
        // Seams half a board in from the sides.
        assert!((0..SIZE).all(|x| texel(&v, x, 2, 5) == seam && texel(&v, x, 5, 6) == seam));
        assert_ne!(texel(&v, 3, 0, 0), seam);
        // Each board has one joint across it, off the block's ends and middle.
        for (y, z) in [(0, 0), (4, 4), (12, 8)] {
            let joints: Vec<_> = (0..SIZE).filter(|&x| texel(&v, x, y, z) == seam).collect();
            assert_eq!(joints.len(), 1, "{joints:?}");
            assert!(![0, 7, 8, SIZE - 1].contains(&joints[0]), "{joints:?}");
        }
    }

    const CROSSED: &str = "planks = { width = 4, thickness = 2, colors = [[180, 140, 90]], shade = 0.1, cross = true }";

    #[test]
    fn crossed_boards_turn_every_layer_and_are_told_apart_by_shade() {
        let v = recipe(CROSSED).generate().unwrap();
        // With one grain color, a board is one shade all through.
        let along_x = |y: usize, z: usize| (0..SIZE).all(|x| texel(&v, x, y, z) == texel(&v, 0, y, z));
        let along_y = |x: usize, z: usize| (0..SIZE).all(|y| texel(&v, x, y, z) == texel(&v, x, 0, z));
        // Layers two texels thick, half a layer up: z 15 and 0 share one, then 1 and 2, and so on.
        assert!(along_x(5, 0) && along_x(5, 15) && !along_y(5, 0), "the bottom layer runs along X");
        assert!(along_y(5, 1) && along_y(5, 2) && !along_x(5, 1), "the next one along Y");
        assert!(along_x(5, 3) && along_x(5, 4), "and back along X");
        // Boards 4 wide, each its own shade, with no seams or joints.
        assert_eq!(texel(&v, 0, 2, 0), texel(&v, 0, 5, 0), "one board");
        assert_ne!(texel(&v, 0, 5, 0), texel(&v, 0, 6, 0), "the board beside it");
        // Four shade steps, and no two boards that touch share one: 4 boards in each of 8
        // layers would merge into fewer patches if any did.
        assert_eq!(colors(&v).len(), 4);
        assert_eq!(patches(&v), 32);
        // No board strays more than the shade from the grain.
        assert!(colors(&v).iter().all(|c| (162..=198).contains(&c[0])), "{:?}", colors(&v));
    }

    #[test]
    fn brick_speckle_can_be_turned_down() {
        let v = recipe("bricks = { length = 16, height = 8, colors = [[150, 60, 50]], speckle = 0.02 }")
            .generate()
            .unwrap();
        assert!(colors(&v).iter().all(|c| (147..=153).contains(&c[0])), "{:?}", colors(&v));
    }

    #[test]
    fn bad_recipes_say_what_is_wrong() {
        let err = |toml: &str| recipe(toml).generate().unwrap_err();
        assert!(err("").contains("exactly one"));
        assert!(err(&format!("{STONE}\nbricks = {{ length = 8, height = 4, colors = [[1, 1, 1]], mortar = [0, 0, 0] }}"))
            .contains("exactly one"));
        assert!(err("noise = { colors = [] }").contains("at least one color"));
        assert!(err("noise = { colors = [[1, 1, 1]], scale = 0.5 }").contains("scale"));
        assert!(err("bricks = { length = 5, height = 4, colors = [[1, 1, 1]], mortar = [0, 0, 0] }").contains("divide"));
        assert!(err("planks = { width = 4, thickness = 16, colors = [[1, 1, 1]], cross = true }").contains("even"));
        assert!(err("planks = { width = 4, colors = [[1, 1, 1]], shade = 2 }").contains("shade"));
        assert!(err("noise = { colors = [[1, 1, 1]] }\ntop = { colors = [[1, 1, 1]], depth = 0 }").contains("depth"));
        assert!(toml::from_str::<Recipe>("noise = { colours = [[1, 1, 1]] }").is_err(), "unknown fields are errors");
    }
}
