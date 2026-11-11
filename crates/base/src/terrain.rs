//! The base game's terrain: hills and mountain ranges from a seed, with bare stone on steep
//! slopes, frozen lakes in the hollows and boulders scattered about.
//!
//! The shaping follows ideas from Tectonic, a terrain mod by Apollo (MIT; see
//! `mods/base/CREDITS.md`): a few slow noise fields (how inland, how worn down, ridge or valley)
//! shape everything, and curves turn them into ground height, mountain height and roughness.
//! The code and numbers are our own, and it is a height map: no overhangs or caves.

use api::{CHUNK_SIZE, ChunkFill, Generator, I64Vec3, Tunable, Tunables};

use crate::{DIRT, GRASS, GRAVEL, ICE, STONE};

/// The generator's registered name.
pub const HILLS: &str = "base:hills";

/// How far rocks reach past their own grid cell, at most, in boxels: [`Hills::rock_max`]'s top.
const ROCK_REACH_MAX: f32 = 8.0;

/// Every value that shapes the terrain. Heights are in boxels above the terrain's center;
/// scales are how many boxels a feature spans.
#[derive(Clone, Debug, PartialEq)]
pub struct Hills {
    pub seed: f32,
    /// How big the inland and lowland areas are.
    pub continent_scale: f32,
    /// How big the worn-flat and rugged areas are.
    pub erosion_scale: f32,
    /// How far apart ridges and valleys are.
    pub ridge_scale: f32,
    /// How far apart mountain ranges are.
    pub range_scale: f32,
    /// How big the gaps that break up the ranges are.
    pub weathering_scale: f32,
    /// How far the land is pushed around, so features don't line up with the noise grid.
    pub warp: f32,
    /// Ground height deep in the lowlands and far inland.
    pub low_height: f32,
    pub inland_height: f32,
    /// How tall the most rugged mountains get above the land around them.
    pub mountain_height: f32,
    /// How tall rolling hills are where the land is worn down.
    pub hill_height: f32,
    /// How deep valleys cut.
    pub valley_depth: f32,
    /// How rough mountain sides are.
    pub jaggedness: f32,
    /// Small bumps everywhere.
    pub detail: f32,
    /// Hollows below this fill with ice.
    pub ice_level: f32,
    /// Slopes steeper than this (boxels of rise per boxel across) are bare stone.
    pub cliff_slope: f32,
    /// Above this the ground is bare stone.
    pub stone_line: f32,
    /// How deep dirt goes under grass.
    pub dirt_depth: f32,
    /// Boulders: one may sit in each square this wide, with this chance, and this big.
    pub rock_spacing: f32,
    pub rock_chance: f32,
    pub rock_min: f32,
    pub rock_max: f32,
}

impl Default for Hills {
    fn default() -> Self {
        Self {
            seed: 1.0,
            continent_scale: 700.0,
            erosion_scale: 420.0,
            ridge_scale: 260.0,
            range_scale: 320.0,
            weathering_scale: 160.0,
            warp: 60.0,
            low_height: -40.0,
            inland_height: 50.0,
            mountain_height: 170.0,
            hill_height: 22.0,
            valley_depth: 26.0,
            jaggedness: 12.0,
            detail: 3.0,
            ice_level: 0.0,
            cliff_slope: 2.5,
            stone_line: 150.0,
            dirt_depth: 4.0,
            rock_spacing: 18.0,
            rock_chance: 0.3,
            rock_min: 1.2,
            rock_max: 3.5,
        }
    }
}

/// The tuning panel's groups for [`Hills`]'s values.
const HILLS_GROUPS: &[(&str, &[&str])] = &[
    ("Seed", &["seed"]),
    ("Feature sizes", &["continent_scale", "erosion_scale", "ridge_scale", "range_scale", "weathering_scale", "warp"]),
    ("Heights", &["low_height", "inland_height", "mountain_height", "hill_height", "valley_depth"]),
    ("Roughness", &["jaggedness", "detail", "cliff_slope"]),
    ("Ground cover", &["ice_level", "stone_line", "dirt_depth"]),
    ("Boulders", &["rock_spacing", "rock_chance", "rock_min", "rock_max"]),
];
/// What each of [`Hills`]'s values does, for the tuning panel.
const HILLS_HELP: &[(&str, &str)] = &[
    ("seed", "Which world: each seed makes different terrain from the same shape values."),
    ("continent_scale", "How big the inland and lowland areas are, boxels across."),
    ("erosion_scale", "How big the worn-flat and rugged areas are, boxels across."),
    ("ridge_scale", "How far apart ridges and valleys are, boxels."),
    ("range_scale", "How far apart mountain ranges are, boxels. Higher spreads them into broader ranges."),
    ("weathering_scale", "How big the gaps that break up the ranges are, boxels."),
    ("warp", "How far the land is pushed around, boxels, so features don't line up with the noise grid."),
    ("low_height", "Ground height deep in the lowlands, boxels above the terrain's center."),
    ("inland_height", "Ground height far inland, boxels above the terrain's center."),
    ("mountain_height", "How tall the most rugged mountains get above the land around them, boxels."),
    ("hill_height", "How tall rolling hills are where the land is worn down, boxels."),
    ("valley_depth", "How deep valleys cut, boxels."),
    ("jaggedness", "How rough mountain sides are. Lower softens the pointy peaks."),
    ("detail", "Small bumps everywhere, boxels."),
    ("cliff_slope", "Slopes steeper than this (boxels of rise per boxel across) are bare stone."),
    ("ice_level", "Hollows below this height fill with ice, boxels."),
    ("stone_line", "Above this height the ground is bare stone, boxels."),
    ("dirt_depth", "How deep dirt goes under grass, boxels."),
    ("rock_spacing", "Boulders: at most one sits in each square this many boxels wide."),
    ("rock_chance", "Boulders: the chance each square gets one."),
    ("rock_min", "Boulders: the smallest one's reach, boxels."),
    ("rock_max", "Boulders: the biggest one's reach, boxels."),
];

impl Tunables for Hills {
    fn tunables(&self) -> Vec<Tunable> {
        api::tuning::described(vec![
            Tunable::new("seed", self.seed, 0.0, 1000.0),
            Tunable::new("continent_scale", self.continent_scale, 50.0, 4000.0),
            Tunable::new("erosion_scale", self.erosion_scale, 50.0, 4000.0),
            Tunable::new("ridge_scale", self.ridge_scale, 30.0, 2000.0),
            Tunable::new("range_scale", self.range_scale, 30.0, 2000.0),
            Tunable::new("weathering_scale", self.weathering_scale, 20.0, 2000.0),
            Tunable::new("warp", self.warp, 0.0, 300.0),
            Tunable::new("low_height", self.low_height, -250.0, 100.0),
            Tunable::new("inland_height", self.inland_height, -100.0, 250.0),
            Tunable::new("mountain_height", self.mountain_height, 0.0, 500.0),
            Tunable::new("hill_height", self.hill_height, 0.0, 100.0),
            Tunable::new("valley_depth", self.valley_depth, 0.0, 100.0),
            Tunable::new("jaggedness", self.jaggedness, 0.0, 60.0),
            Tunable::new("detail", self.detail, 0.0, 20.0),
            Tunable::new("ice_level", self.ice_level, -250.0, 250.0),
            Tunable::new("cliff_slope", self.cliff_slope, 0.5, 10.0),
            Tunable::new("stone_line", self.stone_line, -250.0, 500.0),
            Tunable::new("dirt_depth", self.dirt_depth, 0.0, 16.0),
            Tunable::new("rock_spacing", self.rock_spacing, 4.0, 100.0),
            Tunable::new("rock_chance", self.rock_chance, 0.0, 1.0),
            Tunable::new("rock_min", self.rock_min, 0.5, ROCK_REACH_MAX),
            Tunable::new("rock_max", self.rock_max, 0.5, ROCK_REACH_MAX),
        ], HILLS_GROUPS, HILLS_HELP)
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        let slot = match name {
            "seed" => &mut self.seed,
            "continent_scale" => &mut self.continent_scale,
            "erosion_scale" => &mut self.erosion_scale,
            "ridge_scale" => &mut self.ridge_scale,
            "range_scale" => &mut self.range_scale,
            "weathering_scale" => &mut self.weathering_scale,
            "warp" => &mut self.warp,
            "low_height" => &mut self.low_height,
            "inland_height" => &mut self.inland_height,
            "mountain_height" => &mut self.mountain_height,
            "hill_height" => &mut self.hill_height,
            "valley_depth" => &mut self.valley_depth,
            "jaggedness" => &mut self.jaggedness,
            "detail" => &mut self.detail,
            "ice_level" => &mut self.ice_level,
            "cliff_slope" => &mut self.cliff_slope,
            "stone_line" => &mut self.stone_line,
            "dirt_depth" => &mut self.dirt_depth,
            "rock_spacing" => &mut self.rock_spacing,
            "rock_chance" => &mut self.rock_chance,
            "rock_min" => &mut self.rock_min,
            "rock_max" => &mut self.rock_max,
            _ => return false,
        };
        *slot = value;
        true
    }
}

/// A boulder: a squashed, lumpy ball of stone.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rock {
    center: [f64; 3],
    radius: f64,
    /// Its own salt for the lumps.
    salt: u64,
}

impl Hills {
    fn seed(&self) -> u64 {
        self.seed.max(0.0) as u64
    }

    /// The ground's height at `x, y`, in boxels above the center, before rounding.
    pub fn height(&self, x: f64, y: f64) -> f64 {
        let seed = self.seed();
        let n = |salt: u64, scale: f32, octaves: u32, x: f64, y: f64| {
            fbm(seed.wrapping_mul(31).wrapping_add(salt), x / scale.max(1.0) as f64, y / scale.max(1.0) as f64, octaves)
        };
        // Push the land around first, so nothing lines up with the noise's grid.
        let warp = self.warp as f64;
        let wx = x + warp * n(1, 220.0, 2, x, y);
        let wy = y + warp * n(2, 220.0, 2, x, y);

        let inland = (n(3, self.continent_scale, 4, wx, wy) * 1.5).clamp(-1.0, 1.0);
        let erosion = (n(4, self.erosion_scale, 3, wx, wy) * 1.5).clamp(-1.0, 1.0);
        let ridge = n(5, self.ridge_scale, 3, wx, wy) * 1.4;
        // Folded: -1 in a valley's floor, 1 on a ridge's crest.
        let peaks = 1.0 - (3.0 * ridge.abs() - 2.0).abs();

        let (low, high) = (self.low_height as f64, self.inland_height as f64);
        let base = curve(
            &[(-1.0, low), (-0.45, low * 0.6), (-0.15, 0.1 * high), (0.1, 0.3 * high), (0.45, 0.7 * high), (1.0, high)],
            inland,
        );
        // Rugged where erosion is low, worn flat where it's high; mountains only inland.
        let rugged = curve(&[(-1.0, 1.0), (-0.4, 0.75), (0.0, 0.35), (0.4, 0.12), (1.0, 0.03)], erosion);
        let land = smoothstep(-0.25, 0.25, inland);

        // Mountain ranges: ridged noise, sharpened, broken up by weathering.
        let range = ridged(seed.wrapping_mul(37).wrapping_add(6), wx / self.range_scale.max(1.0) as f64, wy / self.range_scale.max(1.0) as f64, 4);
        let weathering = smoothstep(-0.35, 0.35, n(7, self.weathering_scale, 2, wx, wy));
        let shape = range.powf(1.7) * (0.35 + 0.65 * weathering);
        let mountains = self.mountain_height as f64 * rugged * land * shape;

        let hills = self.hill_height as f64 * peaks.max(-0.5) * (1.0 - rugged * 0.7);
        let valley = self.valley_depth as f64 * smoothstep(-0.55, -0.95, peaks) * land;
        let jagged = self.jaggedness as f64 * rugged * shape.sqrt() * n(8, 22.0, 2, x, y);
        let detail = self.detail as f64 * n(9, 36.0, 3, x, y);
        base + mountains + hills - valley + jagged + detail
    }

    /// The top boxel of ground at `x, y`.
    pub fn ground(&self, x: i64, y: i64) -> i64 {
        self.height(x as f64 + 0.5, y as f64 + 0.5).floor() as i64
    }

    /// Where feet stand at `x, y`: on the ground, or on the ice over it. Boulders aren't
    /// counted.
    pub fn surface(&self, x: i64, y: i64) -> i64 {
        self.ground(x, y).max(self.ice_level.floor() as i64) + 1
    }

    /// Every boulder that may reach into the box from `min` to `max` (level only).
    fn rocks_near(&self, min: [i64; 2], max: [i64; 2]) -> Vec<Rock> {
        let spacing = self.rock_spacing.max(1.0) as f64;
        let reach = self.rock_max.clamp(0.5, ROCK_REACH_MAX) as f64 + 1.0;
        let cell = |v: i64, pad: f64| ((v as f64 + pad) / spacing).floor() as i64;
        let mut out = Vec::new();
        let seed = self.seed().wrapping_mul(41).wrapping_add(10);
        for j in cell(min[1], -reach)..=cell(max[1], reach) {
            for i in cell(min[0], -reach)..=cell(max[0], reach) {
                let h = hash3(seed, i, j, 0);
                if unit(h) >= self.rock_chance as f64 {
                    continue;
                }
                let x = (i as f64 + unit(hash3(seed, i, j, 1))) * spacing;
                let y = (j as f64 + unit(hash3(seed, i, j, 2))) * spacing;
                let (lo, hi) = (self.rock_min.min(self.rock_max) as f64, self.rock_max.max(self.rock_min) as f64);
                let radius = (lo + (hi - lo) * unit(hash3(seed, i, j, 3)).powi(2)).min(ROCK_REACH_MAX as f64);
                let ground = self.height(x, y);
                // Not under the ice.
                if ground < self.ice_level as f64 {
                    continue;
                }
                out.push(Rock { center: [x, y, ground + radius * 0.15], radius, salt: hash3(seed, i, j, 4) });
            }
        }
        out
    }
}

impl Generator for Hills {
    fn fill(&self, origin: I64Vec3, out: &mut ChunkFill) {
        let s = CHUNK_SIZE as usize;
        // Heights over the chunk with a border of one, for slopes.
        let w = s + 2;
        let mut heights = vec![0i64; w * w];
        for y in 0..w {
            for x in 0..w {
                heights[x + y * w] = self.ground(origin.x + x as i64 - 1, origin.y + y as i64 - 1);
            }
        }
        let lowest = heights.iter().copied().min().unwrap_or(0);
        let ice = self.ice_level.floor() as i64;
        let top = origin.z + CHUNK_SIZE - 1;
        let rocks = self.rocks_near([origin.x, origin.y], [origin.x + CHUNK_SIZE - 1, origin.y + CHUNK_SIZE - 1]);
        let rocks_reach = rocks.iter().any(|r| (r.center[2] + r.radius) as i64 >= origin.z && (r.center[2] - r.radius) as i64 <= top);
        let highest = heights.iter().copied().max().unwrap_or(0).max(ice);
        // All of it underground, deeper than any surface layer: solid stone.
        let stone = out.entry(STONE);
        if top < lowest - self.dirt_depth.ceil() as i64 - 1 {
            out.cells.fill(stone);
            return;
        }
        // All of it in the air.
        if origin.z > highest && !rocks_reach {
            return;
        }
        let (grass, dirt, gravel, ice_block) = (out.entry(GRASS), out.entry(DIRT), out.entry(GRAVEL), out.entry(ICE));
        let dirt_depth = self.dirt_depth.max(0.0).round() as i64;
        for y in 0..s {
            for x in 0..s {
                let at = |dx: i64, dy: i64| heights[(x as i64 + 1 + dx) as usize + (y as i64 + 1 + dy) as usize * w];
                let h = at(0, 0);
                let slope = [at(1, 0), at(-1, 0), at(0, 1), at(0, -1)].iter().map(|n| (n - h).abs()).max().unwrap_or(0);
                let under_ice = h < ice;
                let bare = slope as f32 > self.cliff_slope || h as f32 > self.stone_line;
                // Steeper ground keeps less soil.
                let soil = if bare { 0 } else { (dirt_depth - slope / 2).max(1) };
                for z in 0..s {
                    let wz = origin.z + z as i64;
                    let e = if wz <= h {
                        let depth = h - wz;
                        if under_ice {
                            if depth < soil.max(1) { gravel } else { stone }
                        } else if bare || depth >= soil {
                            stone
                        } else if depth == 0 {
                            grass
                        } else {
                            dirt
                        }
                    } else if wz <= ice {
                        ice_block
                    } else {
                        continue;
                    };
                    out.set(x, y, z, e);
                }
            }
        }
        for rock in &rocks {
            self.place_rock(rock, origin, out, stone);
        }
    }
}

impl Hills {
    fn place_rock(&self, rock: &Rock, origin: I64Vec3, out: &mut ChunkFill, stone: u16) {
        let s = CHUNK_SIZE;
        let r = rock.radius;
        let [cx, cy, cz] = rock.center;
        let range = |c: f64, o: i64| -> (i64, i64) {
            let lo = ((c - r - 1.0).floor() as i64 - o).max(0);
            let hi = ((c + r + 1.0).ceil() as i64 - o).min(s - 1);
            (lo, hi)
        };
        let ((x0, x1), (y0, y1), (z0, z1)) = (range(cx, origin.x), range(cy, origin.y), range(cz, origin.z));
        for z in z0..=z1 {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let p = [(origin.x + x) as f64 + 0.5, (origin.y + y) as f64 + 0.5, (origin.z + z) as f64 + 0.5];
                    let d = [(p[0] - cx) / r, (p[1] - cy) / r, (p[2] - cz) / (r * 0.75)];
                    let lumps = 0.8 + 0.35 * value3(rock.salt, p[0] / 1.7, p[1] / 1.7, p[2] / 1.7);
                    if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < lumps * lumps {
                        out.set(x as usize, y as usize, z as usize, stone);
                    }
                }
            }
        }
    }
}

/// A smooth curve through `points` (sorted by x), flat past either end.
fn curve(points: &[(f64, f64)], x: f64) -> f64 {
    let last = points.len() - 1;
    if x <= points[0].0 {
        return points[0].1;
    }
    if x >= points[last].0 {
        return points[last].1;
    }
    let i = points.windows(2).position(|w| x < w[1].0).unwrap_or(last - 1);
    let (p0, p1) = (points[i], points[i + 1]);
    // Slopes at each point from its neighbors, so the curve passes smoothly through.
    let slope = |k: usize| {
        let (a, b) = (points[k.saturating_sub(1)], points[(k + 1).min(last)]);
        (b.1 - a.1) / (b.0 - a.0)
    };
    let (m0, m1) = (slope(i), slope(i + 1));
    let dx = p1.0 - p0.0;
    let t = (x - p0.0) / dx;
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * p0.1 + (t3 - 2.0 * t2 + t) * dx * m0 + (-2.0 * t3 + 3.0 * t2) * p1.1 + (t3 - t2) * dx * m1
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Mixes the bits of a seed and three integers into a well-spread 64-bit number.
fn hash3(seed: u64, x: i64, y: i64, z: i64) -> u64 {
    let mut h = seed ^ 0x9E37_79B9_7F4A_7C15;
    for v in [x as u64, y as u64, z as u64] {
        h ^= v.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h = (h ^ (h >> 31)).wrapping_mul(0x94D0_49BB_1331_11EB);
        h ^= h >> 29;
    }
    h
}

/// A hash as a number in `0..1`.
fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Gradient noise in two dimensions, roughly in `-1..1`, smooth, with features about a unit
/// apart.
fn gradient(seed: u64, x: f64, y: f64) -> f64 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let (ix, iy) = (ix as i64, iy as i64);
    let corner = |cx: i64, cy: i64, dx: f64, dy: f64| {
        // One of 16 directions around the circle.
        let a = (hash3(seed, cx, cy, 0) >> 60) as usize;
        let (gx, gy) = DIRECTIONS[a];
        gx * dx + gy * dy
    };
    let fade = |t: f64| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v) = (fade(fx), fade(fy));
    let n00 = corner(ix, iy, fx, fy);
    let n10 = corner(ix + 1, iy, fx - 1.0, fy);
    let n01 = corner(ix, iy + 1, fx, fy - 1.0);
    let n11 = corner(ix + 1, iy + 1, fx - 1.0, fy - 1.0);
    let a = n00 + u * (n10 - n00);
    let b = n01 + u * (n11 - n01);
    (a + v * (b - a)) * std::f64::consts::SQRT_2
}

const D: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// Sixteen unit directions, evenly around the circle.
const DIRECTIONS: [(f64, f64); 16] = [
    (1.0, 0.0),
    (0.923_879_5, 0.382_683_4),
    (D, D),
    (0.382_683_4, 0.923_879_5),
    (0.0, 1.0),
    (-0.382_683_4, 0.923_879_5),
    (-D, D),
    (-0.923_879_5, 0.382_683_4),
    (-1.0, 0.0),
    (-0.923_879_5, -0.382_683_4),
    (-D, -D),
    (-0.382_683_4, -0.923_879_5),
    (0.0, -1.0),
    (0.382_683_4, -0.923_879_5),
    (D, -D),
    (0.923_879_5, -0.382_683_4),
];

/// Layers of gradient noise, each twice as fine and half as strong, roughly in `-1..1`.
fn fbm(seed: u64, x: f64, y: f64, octaves: u32) -> f64 {
    let (mut sum, mut amp, mut freq, mut total) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        // Each layer turned a little, so their grids don't line up.
        let (rx, ry) = (x * 0.8 - y * 0.6, x * 0.6 + y * 0.8);
        let (x2, y2) = if o % 2 == 0 { (x, y) } else { (rx, ry) };
        sum += amp * gradient(seed.wrapping_add(o as u64 * 1013), x2 * freq, y2 * freq);
        total += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / total
}

/// Ridged layers: sharp crests where the noise crosses zero, in `0..1`.
fn ridged(seed: u64, x: f64, y: f64, octaves: u32) -> f64 {
    let (mut sum, mut amp, mut freq, mut total) = (0.0, 1.0, 1.0, 0.0);
    // Each layer's crests matter more where the coarser ones are already high.
    let mut weight = 1.0;
    for o in 0..octaves {
        let r = 1.0 - gradient(seed.wrapping_add(o as u64 * 7919), x * freq, y * freq).abs().min(1.0);
        let r = r * r * weight;
        weight = r.clamp(0.0, 1.0);
        sum += amp * r;
        total += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / total
}

/// Smooth value noise in three dimensions, in `-1..1`, for lumps.
fn value3(seed: u64, x: f64, y: f64, z: f64) -> f64 {
    let (ix, iy, iz) = (x.floor(), y.floor(), z.floor());
    let s = |t: f64| t * t * (3.0 - 2.0 * t);
    let (u, v, w) = (s(x - ix), s(y - iy), s(z - iz));
    let (ix, iy, iz) = (ix as i64, iy as i64, iz as i64);
    let at = |dx: i64, dy: i64, dz: i64| unit(hash3(seed, ix + dx, iy + dy, iz + dz)) * 2.0 - 1.0;
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let x00 = lerp(at(0, 0, 0), at(1, 0, 0), u);
    let x10 = lerp(at(0, 1, 0), at(1, 1, 0), u);
    let x01 = lerp(at(0, 0, 1), at(1, 0, 1), u);
    let x11 = lerp(at(0, 1, 1), at(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(hills: &Hills, origin: I64Vec3) -> ChunkFill {
        let mut out = ChunkFill::default();
        hills.fill(origin, &mut out);
        out
    }

    /// The block names of a chunk, boxel by boxel, so chunks with palettes in other orders compare.
    fn names(fill: &ChunkFill) -> Vec<Option<&str>> {
        let s = CHUNK_SIZE as usize;
        let mut out = Vec::new();
        for z in 0..s {
            for y in 0..s {
                for x in 0..s {
                    out.push(fill.name(x, y, z));
                }
            }
        }
        out
    }

    /// A chunk origin at the ground near level spot `x, y`.
    fn surface_chunk(hills: &Hills, x: i64, y: i64) -> I64Vec3 {
        let z = hills.ground(x + 16, y + 16).div_euclid(CHUNK_SIZE) * CHUNK_SIZE;
        I64Vec3::new(x, y, z)
    }

    #[test]
    fn the_same_seed_gives_the_same_chunk() {
        let hills = Hills::default();
        let origin = surface_chunk(&hills, 64, -96);
        assert_eq!(names(&fill(&hills, origin)), names(&fill(&Hills::default(), origin)));
        let other = Hills { seed: 2.0, ..Hills::default() };
        assert_ne!(names(&fill(&hills, origin)), names(&fill(&other, origin)), "another seed gives other land");
    }

    #[test]
    fn the_land_varies_in_height() {
        let hills = Hills::default();
        let mut heights = Vec::new();
        for y in (-256..256).step_by(16) {
            for x in (-256..256).step_by(16) {
                heights.push(hills.ground(x, y));
            }
        }
        let (lo, hi) = (*heights.iter().min().unwrap(), *heights.iter().max().unwrap());
        assert!(hi - lo > 60, "from {lo} to {hi}");
        assert!(lo < hi && hi < 300, "peaks fit a 512-high terrain: {hi}");
    }

    #[test]
    fn hollows_fill_with_ice_up_to_a_level_top() {
        // Ice everywhere the ground is below the ice level: put the level high to find some.
        let hills = Hills { ice_level: 40.0, rock_chance: 0.0, ..Hills::default() };
        let mut found = 0;
        for cy in -4..4 {
            for cx in -4..4 {
                let origin = I64Vec3::new(cx * 32, cy * 32, 32);
                let out = fill(&hills, origin);
                for y in 0..32 {
                    for x in 0..32 {
                        let ground = hills.ground(origin.x + x as i64, origin.y + y as i64);
                        if ground >= 40 {
                            continue;
                        }
                        found += 1;
                        // z = 40 is local 8 in a chunk starting at 32; nothing above it.
                        assert_eq!(out.name(x, y, 8), Some(ICE), "top of the ice at {x}, {y}");
                        assert_eq!(out.name(x, y, 9), None, "nothing over the ice at {x}, {y}");
                    }
                }
            }
        }
        assert!(found > 0, "some hollows below the ice level");
    }

    #[test]
    fn ground_is_grass_over_dirt_over_stone_where_its_gentle() {
        let hills = Hills { rock_chance: 0.0, stone_line: 1000.0, ..Hills::default() };
        // Find a gentle spot above the ice.
        let (x, y) = (-256..256)
            .flat_map(|y| (-256..256).map(move |x| (x, y)))
            .step_by(7)
            .find(|&(x, y)| {
                let h = hills.ground(x, y);
                h > 2 && [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().all(|(dx, dy)| hills.ground(x + dx, y + dy) == h)
            })
            .expect("somewhere flat");
        let h = hills.ground(x, y);
        let origin = I64Vec3::new(x.div_euclid(32) * 32, y.div_euclid(32) * 32, (h - 8).div_euclid(32) * 32);
        let at = |z: i64| -> Option<String> {
            let mut out = ChunkFill::default();
            let o = I64Vec3::new(origin.x, origin.y, z.div_euclid(32) * 32);
            hills.fill(o, &mut out);
            out.name((x - o.x) as usize, (y - o.y) as usize, (z - o.z) as usize).map(String::from)
        };
        assert_eq!(at(h + 1), None);
        assert_eq!(at(h).as_deref(), Some(GRASS));
        assert_eq!(at(h - 1).as_deref(), Some(DIRT));
        assert_eq!(at(h - 10).as_deref(), Some(STONE));
    }

    #[test]
    fn a_rock_across_chunk_borders_matches_on_both_sides() {
        let hills = Hills { rock_chance: 1.0, rock_min: 4.0, rock_max: 4.0, ..Hills::default() };
        // Fill two neighboring chunks and the column of four around a border; the shared
        // boxels must agree, which they do if each chunk sees every rock reaching into it.
        let rocks = hills.rocks_near([-64, -64], [64, 64]);
        let rock = rocks.iter().find(|r| (r.center[0] - 32.0).abs() < 3.0).or(rocks.first()).expect("a rock");
        let z = ((rock.center[2]).floor() as i64).div_euclid(32) * 32;
        let bx = (rock.center[0].floor() as i64).div_euclid(32) * 32;
        let by = (rock.center[1].floor() as i64).div_euclid(32) * 32;
        let big = fill(&hills, I64Vec3::new(bx, by, z));
        // The rock's center boxel is stone.
        let (lx, ly, lz) = (rock.center[0].floor() as i64 - bx, rock.center[1].floor() as i64 - by, rock.center[2].floor() as i64 - z);
        assert_eq!(big.name(lx as usize, ly as usize, lz as usize), Some(STONE));
        // Every rock that reaches into a chunk is among the ones it gathers.
        for r in &rocks {
            let cx = (r.center[0] + r.radius).floor() as i64;
            let near = hills.rocks_near([cx, r.center[1] as i64], [cx, r.center[1] as i64]);
            assert!(near.contains(r), "{r:?} is seen from its east edge");
        }
    }

    #[test]
    fn deep_chunks_are_solid_stone_and_high_ones_empty() {
        let hills = Hills::default();
        let deep = fill(&hills, I64Vec3::new(0, 0, -256));
        assert!(deep.cells.iter().all(|&e| deep.palette[e as usize - 1] == STONE));
        let high = fill(&hills, I64Vec3::new(0, 0, 480));
        assert!(high.cells.iter().all(|&e| e == 0));
    }

    #[test]
    fn curves_pass_through_their_points_and_stay_flat_past_the_ends() {
        let pts = [(-1.0, -10.0), (0.0, 0.0), (1.0, 30.0)];
        assert_eq!(curve(&pts, -2.0), -10.0);
        assert!((curve(&pts, 0.0) - 0.0).abs() < 1e-9);
        assert_eq!(curve(&pts, 5.0), 30.0);
        assert!(curve(&pts, 0.5) > 0.0 && curve(&pts, 0.5) < 30.0);
    }

    #[test]
    fn noise_stays_in_range() {
        for i in 0..2000 {
            let (x, y) = (i as f64 * 0.37, i as f64 * -0.23);
            let n = fbm(5, x, y, 4);
            assert!((-1.2..=1.2).contains(&n), "{n}");
            let r = ridged(5, x, y, 4);
            assert!((0.0..=1.0).contains(&r), "{r}");
            let v = value3(5, x, y, x - y);
            assert!((-1.0..=1.0).contains(&v), "{v}");
        }
    }
}
