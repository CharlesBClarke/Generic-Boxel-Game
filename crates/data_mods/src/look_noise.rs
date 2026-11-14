//! Noise that never repeats, for baking a look's variants (see [`crate::looks`]): speckle and
//! grain worked out from a texel's place in an endless field, so each variant is a different
//! 16³ window of it. A tool for making looks; nothing is worked out as the game draws.
//!
//! Field positions are `u32`s that wrap: a texel's place in the field, in texels, plus
//! [`WORLD_OFFSET`] so the wrap sits far from the middle.

use api::LookDef;

/// What a look takes from the field: which noise layer or grain each texel gets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LookDetail {
    /// Noise layers a texel can take its color from: the base noise and a `top` cover.
    pub noise: Vec<NoiseLayer>,
    /// Grain for the texels marked with a direction.
    pub grain: Option<Grain>,
    /// One detail byte a texel, in the cube's order (see [`api::LookDef::index`]), or empty
    /// when nothing comes from the field. Bits 0–1: which noise layer the texel shows (0 for
    /// its baked color, `n` for `noise[n - 1]`). Bits 2–3: which way its grain runs (0 none,
    /// then [`GRAIN_X`], [`GRAIN_Y`], [`GRAIN_UP`]). Bits 4–7: its board, so neighboring
    /// boards' grain differs.
    pub texels: Vec<u8>,
}

impl LookDetail {
    /// `base` (a volume, four bytes a texel) with its marked texels taken from the field's
    /// window whose lowest corner is at field texel `corner`: noise texels get the noise's
    /// color there, grain texels their own color shaded by the grain there (divided by the
    /// grain's average, so grain never shifts a color on average).
    pub fn bake(&self, base: &[u8], corner: [u32; 3]) -> Vec<u8> {
        let mut rgba = base.to_vec();
        if self.texels.is_empty() {
            return rgba;
        }
        let mean = self.grain.map(|g| Grain::mean_factor(g.strength, g.mean_value()));
        let size = LookDef::SIZE as u32;
        for (i, &byte) in self.texels.iter().enumerate() {
            let n = i as u32;
            let p = [n % size, n / size % size, n / (size * size)];
            let p = [0, 1, 2].map(|a| corner[a].wrapping_add(p[a]));
            let at = 4 * i;
            let layer = detail_layer(byte) as usize;
            if layer > 0 {
                rgba[at..at + 3].copy_from_slice(&self.noise[layer - 1].color(p));
            } else if let (Some(g), Some(mean), d @ 1..) = (self.grain, mean, detail_grain(byte)) {
                let factor = g.factor(d, detail_board(byte), p) / mean;
                for c in &mut rgba[at..at + 3] {
                    *c = (*c as f32 * factor).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        rgba
    }
}

/// Grain runs along X.
pub const GRAIN_X: u8 = 1;
/// Grain runs along Y.
pub const GRAIN_Y: u8 = 2;
/// Grain runs up.
pub const GRAIN_UP: u8 = 3;
/// Added to a world texel position before it's used, so the `u32` wrap is about 134 million
/// blocks away in every direction rather than at the world's middle.
pub const WORLD_OFFSET: u32 = 1 << 31;

/// A detail byte.
pub const fn detail_byte(layer: u8, grain: u8, board: u8) -> u8 {
    (layer & 3) | (grain & 3) << 2 | (board & 15) << 4
}

/// The noise layer a detail byte shows, 0 for none.
pub const fn detail_layer(byte: u8) -> u8 {
    byte & 3
}

/// The grain direction in a detail byte, 0 for none.
pub const fn detail_grain(byte: u8) -> u8 {
    byte >> 2 & 3
}

/// The board in a detail byte.
pub const fn detail_board(byte: u8) -> u8 {
    byte >> 4
}

/// Speckle: smooth noise, each value picking one of the colors.
#[derive(Clone, Debug, PartialEq)]
pub struct NoiseLayer {
    /// sRGB.
    pub colors: Vec<[u8; 3]>,
    /// Texels across one blob, per axis, from 1 up.
    pub scale: [f32; 3],
    pub seed: u32,
}

/// Grain: noise stretched along each board, shading its texels lighter and darker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grain {
    /// Texels across one streak along the board, across it and through it (the third axis),
    /// each from 1 up.
    pub scale: [f32; 3],
    /// How much it shades, as a fraction either way.
    pub strength: f32,
    pub seed: u32,
}

/// A scale in sixteenths of a texel, as the noise uses it: whole numbers keep the lattice
/// exact at any distance.
pub fn scale_units(scale: f32) -> u32 {
    (scale * 16.0).round().max(16.0) as u32
}

impl NoiseLayer {
    /// The value at world texel `p`, from 0 to 1.
    pub fn value(&self, p: [u32; 3]) -> f32 {
        value(self.scale.map(scale_units), self.seed, p)
    }

    /// The color at world texel `p`, sRGB.
    pub fn color(&self, p: [u32; 3]) -> [u8; 3] {
        pick(&self.colors, self.value(p))
    }
}

impl Grain {
    /// How much grain shades world texel `p` running `direction` on `board`: a factor around 1.
    pub fn factor(&self, direction: u8, board: u8, p: [u32; 3]) -> f32 {
        let [along, across, through] = self.scale.map(scale_units);
        let scale = match direction {
            GRAIN_X => [along, across, through],
            GRAIN_Y => [across, along, through],
            _ => [across, through, along],
        };
        let v = value(scale, self.seed.wrapping_add(board as u32 * 7919), p);
        1.0 + self.strength * (2.0 * v - 1.0)
    }

    /// The grain's noise value on average over the world, every direction and board alike.
    /// Drawing divides [`Grain::factor`] by the factor this value gives, so grain shades
    /// around a texel's own color and never shifts it on average.
    pub fn mean_value(&self) -> f32 {
        let mut sum = 0.0f64;
        let mut count = 0u32;
        for direction in [GRAIN_X, GRAIN_Y, GRAIN_UP] {
            for board in 0..16u8 {
                for i in 0..400u32 {
                    // Spread out, so many streaks are counted.
                    let p = [i % 20 * 13, i / 20 * 11, i * 7].map(|c| c.wrapping_add(WORLD_OFFSET));
                    let factor = self.factor(direction, board, p);
                    sum += ((factor - 1.0) / self.strength.max(1e-6) + 1.0) as f64 / 2.0;
                    count += 1;
                }
            }
        }
        (sum / count as f64) as f32
    }

    /// [`Grain::factor`] on average at strength `strength`: drawing divides by it.
    pub fn mean_factor(strength: f32, mean_value: f32) -> f32 {
        1.0 + strength * (2.0 * mean_value - 1.0)
    }
}

/// Smooth value noise at world texel `p`, from 0 to 1: two octaves of random values on a
/// lattice `scale` sixteenths of a texel apart per axis (the finer one half that, but no
/// finer than a texel), blended between lattice points.
pub fn value(scale: [u32; 3], seed: u32, p: [u32; 3]) -> f32 {
    let fine = scale.map(|s| (s / 2).max(16));
    0.65 * octave(scale, seed, p) + 0.35 * octave(fine, seed.wrapping_add(1), p)
}

fn octave(scale: [u32; 3], seed: u32, p: [u32; 3]) -> f32 {
    // The texel's center in lattice cells, `16p + 8` over `scale`, worked in pieces so it
    // never overflows: the whole cell and how far into it, from 0 to 1.
    let mut base = [0u32; 3];
    let mut t = [0.0f32; 3];
    for i in 0..3 {
        let (whole, rest) = (p[i] / scale[i], p[i] % scale[i]);
        let inner = 16 * rest + 8;
        base[i] = 16u32.wrapping_mul(whole).wrapping_add(inner / scale[i]);
        t[i] = smooth((inner % scale[i]) as f32 / scale[i] as f32);
    }
    let mut sum = 0.0;
    for corner in 0..8u32 {
        let d = [corner & 1, corner >> 1 & 1, corner >> 2 & 1];
        let at = [0, 1, 2].map(|i| base[i].wrapping_add(d[i]));
        let weight: f32 = (0..3).map(|i| if d[i] == 1 { t[i] } else { 1.0 - t[i] }).product();
        sum += weight * unit(hash(at, seed));
    }
    sum
}

pub fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// A well-mixed number for a lattice point.
pub fn hash(p: [u32; 3], seed: u32) -> u32 {
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
pub fn unit(h: u32) -> f32 {
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// One of `colors` for a noise value from 0 to 1. Smooth noise bunches around the middle, so
/// the value is stretched first to give the outer colors a fair share.
pub fn pick(colors: &[[u8; 3]], value: f32) -> [u8; 3] {
    let stretched = ((value - 0.5) * 1.8 + 0.5).clamp(0.0, 0.9999);
    colors[(stretched * colors.len() as f32) as usize]
}

/// A world texel position (in texels, any size) as the noise takes it.
pub fn world_texel(p: [i64; 3]) -> [u32; 3] {
    p.map(|c| (c as u32).wrapping_add(WORLD_OFFSET))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIRT: [[u8; 3]; 4] = [[98, 67, 44], [112, 78, 52], [126, 89, 60], [139, 100, 68]];

    fn dirt(scale: f32) -> NoiseLayer {
        NoiseLayer { colors: DIRT.to_vec(), scale: [scale; 3], seed: 2 }
    }

    #[test]
    fn detail_bytes_hold_their_parts() {
        let b = detail_byte(2, GRAIN_Y, 13);
        assert_eq!((detail_layer(b), detail_grain(b), detail_board(b)), (2, GRAIN_Y, 13));
        assert_eq!(detail_byte(0, 0, 0), 0);
    }

    #[test]
    fn world_noise_never_repeats_a_block_apart() {
        let n = dirt(1.5);
        let at = |x: i64, y: i64, z: i64| n.color(world_texel([x, y, z]));
        // Every block along a line shows a different row of texels.
        let rows: Vec<Vec<[u8; 3]>> = (0..20).map(|b| (0..16).map(|x| at(b * 16 + x, 5, 3)).collect()).collect();
        for i in 0..rows.len() {
            for j in i + 1..rows.len() {
                assert_ne!(rows[i], rows[j], "blocks {i} and {j} match");
            }
        }
    }

    #[test]
    fn world_noise_is_smooth_across_block_edges_and_the_middle_of_the_world() {
        // From one texel to the next, the noise moves about as much across a block edge (or
        // across 0) as inside a block.
        let n = dirt(3.0);
        let step = |a: i64| (n.value(world_texel([a, 7, 2])) - n.value(world_texel([a + 1, 7, 2]))).abs();
        let inside = (1..15).map(step).fold(0.0f32, f32::max);
        for edge in [-1, 15, 31, 16 * 1000 - 1] {
            assert!(step(edge) <= inside * 2.0 + 0.05, "jump at {edge}: {} vs {inside}", step(edge));
        }
    }

    #[test]
    fn world_noise_is_exact_far_away() {
        // Far from the middle, a whole number of blocks away, the lattice still lines up: the
        // same scale and positions a multiple of the lattice apart give the same pattern shape.
        let n = dirt(2.0);
        let far = 100_000_000i64 * 16;
        let near: Vec<f32> = (0..64).map(|x| n.value(world_texel([x, 0, 0]))).collect();
        let away: Vec<f32> = (0..64).map(|x| n.value(world_texel([far + x, 0, 0]))).collect();
        assert_ne!(near, away, "different places differ");
        // Values stay in range and vary smoothly out there too.
        assert!(away.iter().all(|v| (0.0..=1.0).contains(v)));
        let jumps = away.windows(2).map(|w| (w[0] - w[1]).abs()).fold(0.0f32, f32::max);
        assert!(jumps < 0.5, "{jumps}");
    }

    #[test]
    fn grain_streaks_run_along_the_board() {
        let g = Grain { scale: [12.0, 1.0, 1.0], strength: 0.1, seed: 3 };
        let along = |d: u8, p: [i64; 3]| g.factor(d, 0, world_texel(p));
        // Along X, neighbors along the board are closer than neighbors across it.
        let (mut a, mut c) = (0.0, 0.0);
        for i in 0..200i64 {
            let p = [i * 3, i * 5, 0];
            a += (along(GRAIN_X, p) - along(GRAIN_X, [p[0] + 1, p[1], 0])).abs();
            c += (along(GRAIN_X, p) - along(GRAIN_X, [p[0], p[1] + 1, 0])).abs();
        }
        assert!(a * 3.0 < c, "along {a}, across {c}");
        // Every factor stays within the strength.
        for i in 0..100 {
            let f = along(GRAIN_Y, [i, i * 7, i * 3]);
            assert!((0.9..=1.1).contains(&f), "{f}");
        }
        // Boards differ.
        assert_ne!(g.factor(GRAIN_X, 1, world_texel([3, 3, 3])), g.factor(GRAIN_X, 2, world_texel([3, 3, 3])));
    }
}
