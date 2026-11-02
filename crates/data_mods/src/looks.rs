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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bricks {
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

/// Boards running along X, with a joint somewhere along each. Seams run half a board in from
/// the block's sides, and joints keep off its ends and middle. With `weave`, the volume is cut
/// into cubes in a checkerboard instead, boards running along X in one and along Y in the next,
/// lined up with the cubes and with no joints.
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
    /// Texels across each cube of the checkerboard.
    weave: Option<usize>,
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
        let quarter = self.length - self.length / 4;
        let [ox, oy, oz] = self.offset.unwrap_or([quarter, quarter, self.height / 2]);
        for (i, [x, y, z]) in positions() {
            let z = z + oz;
            let row = z / self.height;
            // Every other row shifts half a brick, on both level axes.
            let shift = if row % 2 == 1 { self.length / 2 } else { 0 };
            let (bx, by) = ((x + ox + shift) % SIZE, (y + oy + shift) % SIZE);
            let joint = z % self.height == 0 || bx % self.length == 0 || by % self.length == 0;
            texels[i] = if let Some(mortar) = self.mortar.filter(|_| joint) {
                mortar
            } else {
                let row = (row % (SIZE / self.height)) as u32;
                let brick = hash([(bx / self.length) as u32, (by / self.length) as u32, row], self.seed);
                let color = self.colors[brick as usize % self.colors.len()];
                // A little speckle, so a brick isn't one flat color.
                let u = unit(hash([x as u32, y as u32, z as u32], self.seed ^ 0x5bd1));
                shade(color, 1.0 + self.speckle * (2.0 * u - 1.0))
            };
        }
        Ok(())
    }
}

impl Planks {
    fn fill(&self, texels: &mut [[u8; 3]]) -> Result<(), String> {
        check_colors("planks", &self.colors)?;
        let thickness = self.thickness.unwrap_or(self.width);
        let span = self.weave.unwrap_or(SIZE);
        check_divides("planks weave", span)?;
        for (what, n) in [("planks width", self.width), ("planks thickness", thickness)] {
            if n == 0 || !span.is_multiple_of(n) {
                return Err(format!("{what} must divide {span} evenly, not {n}"));
            }
        }
        if self.seam.is_some() && (self.width < 2 || thickness < 2) {
            return Err("planks need a width and thickness of at least 2, to fit the seam".into());
        }
        check_fraction("planks shade", self.shade)?;
        // Grain runs along the boards: long blobs along them, thin ones across.
        let grain = [
            Field::new(Scale::Axes([8.0, 1.0, 1.0]), self.seed)?,
            Field::new(Scale::Axes([1.0, 8.0, 1.0]), self.seed)?,
        ];
        for (i, [x, y, z]) in positions() {
            // Which cube of the checkerboard, and which way its boards run: 0 along X, 1 along Y.
            let cube = [x / span, y / span, z / span];
            let along = if self.weave.is_some() { (cube[0] + cube[1] + cube[2]) % 2 } else { 0 };
            let (a, c) = if along == 0 { (x, y) } else { (y, x) };
            // Across and up the board, within its cube. Without a weave, seams keep half a board
            // in from the block's sides.
            let (ac, az) = if self.weave.is_some() {
                (c % span, z % span)
            } else {
                ((c + self.width / 2) % SIZE, (z + thickness / 2) % SIZE)
            };
            let cube_id = (cube[0] + cube[1] * 4 + cube[2] * 16) as u32;
            let board = [(ac / self.width) as u32, (az / thickness) as u32, cube_id];
            // A joint in the first or second half of the block, never on its ends or at its
            // middle. A woven board ends with its cube instead.
            let (joint, piece) = if self.weave.is_some() {
                (None, 0)
            } else {
                let h = hash([board[0], board[1], 0], self.seed) as usize;
                let joint = 1 + h % (SIZE / 2 - 2) + (h >> 8) % 2 * (SIZE / 2);
                (Some(joint), u32::from(a >= joint))
            };
            let seam = ac % self.width == 0 || az % thickness == 0 || joint == Some(a);
            texels[i] = match self.seam.filter(|_| seam) {
                Some(seam) => seam,
                None => {
                    // Each board's grain from its own part of the field, so neighbors differ.
                    let id = [board[0], board[1], board[2] * 2 + piece];
                    let offset = (hash([id[0], id[1], id[2] * 4 + 1], self.seed) % SIZE as u32) as usize;
                    let p = if along == 0 { [(x + offset) % SIZE, y, z] } else { [x, (y + offset) % SIZE, z] };
                    let tint = unit(hash([id[0], id[1], id[2] * 4 + 2], self.seed));
                    shade(pick(&self.colors, grain[along].at(p)), 1.0 + self.shade * (2.0 * tint - 1.0))
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

    #[test]
    fn bricks_without_mortar_are_told_apart_by_color() {
        let v = recipe("bricks = { length = 8, height = 4, colors = [[150, 60, 50], [90, 90, 90]] }").generate().unwrap();
        let gray = |c: [u8; 3]| c[0] < 120;
        // A brick is one color all through, apart from its speckle.
        let first = gray(texel(&v, 3, 3, 3));
        assert!((3..=5).all(|x| gray(texel(&v, x, 3, 3)) == first));
        assert!(colors(&v).iter().any(|&c| gray(c)) && colors(&v).iter().any(|&c| !gray(c)), "both colors show");
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

    const WOVEN: &str = "planks = { width = 4, thickness = 4, colors = [[180, 140, 90]], shade = 0.1, weave = 8 }";

    #[test]
    fn woven_boards_run_both_ways_and_are_told_apart_by_shade() {
        let v = recipe(WOVEN).generate().unwrap();
        // With one grain color, a board is one shade all through.
        let along_x = |x0: usize, y: usize, z: usize| (x0..x0 + 8).all(|x| texel(&v, x, y, z) == texel(&v, x0, y, z));
        let along_y = |x: usize, y0: usize, z: usize| (y0..y0 + 8).all(|y| texel(&v, x, y, z) == texel(&v, x, y0, z));
        // The first cube's boards run along X, its neighbors' along Y, and back above it.
        assert!(along_x(0, 3, 1) && !along_y(3, 0, 1), "the first cube runs along X");
        assert!(along_y(11, 0, 1) && !along_x(8, 3, 1), "the next cube over runs along Y");
        assert!(along_y(3, 8, 1), "and the next cube north");
        assert!(along_y(3, 0, 9), "and the next cube up");
        // Boards 4 wide and 4 thick, each its own shade.
        assert_eq!(texel(&v, 0, 0, 0), texel(&v, 7, 3, 3), "one board");
        assert_ne!(texel(&v, 0, 0, 0), texel(&v, 0, 0, 4), "the board above");
        assert!(colors(&v).len() >= 6, "boards differ: {}", colors(&v).len());
        // Gently: no board strays more than the shade from the grain.
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
        assert!(err("planks = { width = 8, colors = [[1, 1, 1]], weave = 4 }").contains("divide 4"));
        assert!(err("planks = { width = 4, colors = [[1, 1, 1]], shade = 2 }").contains("shade"));
        assert!(err("noise = { colors = [[1, 1, 1]] }\ntop = { colors = [[1, 1, 1]], depth = 0 }").contains("depth"));
        assert!(toml::from_str::<Recipe>("noise = { colours = [[1, 1, 1]] }").is_err(), "unknown fields are errors");
    }
}
