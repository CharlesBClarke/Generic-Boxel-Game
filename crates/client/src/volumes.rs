//! Stacks every registered look into one image, so all chunks draw with one material: the
//! volumes one after another, each with its variants, and the same texels cut into face
//! slices with mip levels (see [`Volumes::atlas`]). Plain texels; turning them into Bevy
//! images happens elsewhere.

use api::LookDef;
use sim::{BlockId, BlockRegistry, LookRegistry};

/// Every look's volume with all its variants, one after another. The first is plain white,
/// for blocks drawn in a flat vertex color.
pub struct Volumes {
    /// How many volumes are stacked.
    pub count: u32,
    /// Four bytes a texel, sRGB: each volume's [`VARIANTS`] cubes in turn, each X fastest,
    /// then Y, then Z. A look with fewer variants repeats them to fill its cubes.
    pub rgba: Vec<u8>,
    /// Each volume's sheen as strength and sharpness, then its glow, by volume; zero strength
    /// for matte ones, zero glow for ones that don't glow.
    pub sheens: Vec<[f32; 3]>,
    /// Which volume each block uses and whether its look turns with it, by block ID; `None`
    /// for blocks without a look.
    of_block: Vec<Option<(u32, bool)>>,
    /// Each look's volume and whether it turns, by name, for blocks registered later.
    by_look: std::collections::HashMap<String, (u32, bool)>,
}

/// Variants each volume holds in the atlas: as many as any look can have.
pub const VARIANTS: usize = LookDef::MAX_VARIANTS;

impl Volumes {
    /// The plain white volume.
    pub const WHITE: u32 = 0;

    pub fn build(looks: &LookRegistry, blocks: &BlockRegistry) -> Self {
        let mut rgba = vec![255; LookDef::VOLUME * 4 * VARIANTS];
        let mut by_look = std::collections::HashMap::new();
        let mut sheens = vec![[0.0, 1.0, 0.0]];
        for (i, look) in looks.iter().enumerate() {
            for k in 0..VARIANTS {
                rgba.extend_from_slice(look.variant(k % look.variants()));
            }
            by_look.insert(look.name.clone(), (i as u32 + 1, look.turns));
            let [s, k] = look.sheen.map_or([0.0, 1.0], |s| [s.strength, s.sharpness]);
            sheens.push([s, k, look.glow]);
        }
        let mut volumes = Volumes { count: looks.iter().count() as u32 + 1, rgba, sheens, of_block: vec![None], by_look };
        volumes.sync(blocks);
        volumes
    }

    /// Catches up with blocks registered since: crafting registers shaped blocks while the
    /// game runs. Looks are fixed at startup, so this only looks new blocks' up.
    pub fn sync(&mut self, blocks: &BlockRegistry) {
        for (id, def) in blocks.iter().skip(self.of_block.len() - 1) {
            debug_assert_eq!(id.index(), self.of_block.len());
            // The simulation has checked every name exists; a missing one would draw white.
            let volume = def.look.as_ref().map(|l| self.by_look.get(l).copied().unwrap_or((Self::WHITE, false)));
            self.of_block.push(volume);
        }
    }

    /// The volume a block is drawn from, or `None` if it has no look.
    pub fn of(&self, id: BlockId) -> Option<u32> {
        self.of_block.get(id.index()).copied().flatten().map(|(volume, _)| volume)
    }

    /// The face-slice image's size at level 0: [`Volumes::atlas`].
    pub fn atlas_size(&self) -> [u32; 2] {
        let size = LookDef::SIZE as u32;
        [VARIANTS as u32 * size * size, self.count * 3 * size]
    }

    /// The looks cut into face slices, every mip level one after another from level 0, four
    /// bytes a pixel, rows of [`Volumes::atlas_size`]'s width (halved each level). Variant `k`
    /// of volume `v`'s texels at depth `d` along axis `a` (the faces facing along `a` show
    /// them) are the 16×16 tile in column `16k + d`, row `3v + a`, laid out across the face's
    /// other two axes in order: pixel `(t[(a + 1) % 3], t[(a + 2) % 3])` of the tile is texel
    /// `t`. Each level averages two by two pixels of the one before, in linear light, so a tile
    /// shrinks within itself and never takes color from the next, and a face's texels never
    /// mix with the layers under them. Alpha is unused (opaque).
    pub fn atlas(&self) -> Vec<u8> {
        let size = LookDef::SIZE;
        let [width, height] = self.atlas_size().map(|n| n as usize);
        let mut level = vec![255u8; width * height * 4];
        for v in 0..self.count as usize {
            for k in 0..VARIANTS {
                let cube = v * VARIANTS + k;
                for a in 0..3 {
                    let (u, w) = ((a + 1) % 3, (a + 2) % 3);
                    for i in 0..LookDef::VOLUME {
                        let t = [i % size, i / size % size, i / (size * size)];
                        let x = (k * size + t[a]) * size + t[u];
                        let y = (3 * v + a) * size + t[w];
                        let from = 4 * (cube * LookDef::VOLUME + i);
                        let to = 4 * (y * width + x);
                        level[to..to + 3].copy_from_slice(&self.rgba[from..from + 3]);
                    }
                }
            }
        }
        let mut all = level.clone();
        let (mut w, mut h) = (width, height);
        for _ in 1..LEVELS {
            let (nw, nh) = (w / 2, h / 2);
            let mut next = vec![255u8; nw * nh * 4];
            for y in 0..nh {
                for x in 0..nw {
                    for c in 0..3 {
                        let sum: f32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                            .iter()
                            .map(|&(dx, dy)| srgb_to_linear(level[4 * ((2 * y + dy) * w + 2 * x + dx) + c]))
                            .sum();
                        next[4 * (y * nw + x) + c] = linear_to_srgb(sum / 4.0);
                    }
                }
            }
            all.extend_from_slice(&next);
            (level, w, h) = (next, nw, nh);
        }
        all
    }

    /// Whether a block's look turns with it.
    pub fn turns(&self, id: BlockId) -> bool {
        self.of_block.get(id.index()).copied().flatten().is_some_and(|(_, turns)| turns)
    }
}

/// Which of the first `n` variants the boxel at `cell` shows, its cell's coordinates taken as
/// `u32`s (wrapping): a hash, so neighbors differ without a pattern. Matches `variant_of` in
/// `solid.wgsl`.
pub fn variant_of(cell: [u32; 3], n: u32) -> u32 {
    let mut h = cell[0].wrapping_mul(0x8da6_b343) ^ cell[1].wrapping_mul(0xd816_3841) ^ cell[2].wrapping_mul(0xcb1a_b31f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h % n.max(1)
}

/// Mip levels of [`Volumes::atlas`]: down to one pixel a slice.
pub const LEVELS: u32 = 5;

/// An sRGB byte as linear light, 0 to 1.
fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> u8 {
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_hold_each_face_layer_and_variant_and_shrink_within_themselves() {
        let mut looks = LookRegistry::default();
        // A look whose texels say where they are: red is x, green y, blue z (times 16); then
        // a second variant, the same with its red halved.
        let first: Vec<u8> = (0..LookDef::VOLUME)
            .flat_map(|i| [(i % 16 * 16) as u8, (i / 16 % 16 * 16) as u8, (i / 256 * 16) as u8, 255])
            .collect();
        let second: Vec<u8> = first.chunks(4).flat_map(|p| [p[0] / 2, p[1], p[2], 255]).collect();
        let rgba = [first, second].concat();
        looks.register(LookDef { name: "test:where".into(), turns: false, rgba, sheen: None, glow: 0.0 }).unwrap();
        looks.register(look("test:red", [255, 0, 0, 255], false)).unwrap();
        let volumes = Volumes::build(&looks, &BlockRegistry::default());
        let [width, height] = volumes.atlas_size().map(|n| n as usize);
        assert_eq!([width, height], [VARIANTS * 256, 3 * 3 * 16]);
        let atlas = volumes.atlas();
        let total: usize = (0..LEVELS).map(|k| (width >> k) * (height >> k) * 4).sum();
        assert_eq!(atlas.len(), total);
        let px = |x: usize, y: usize| &atlas[4 * (y * width + x)..4 * (y * width + x) + 4];
        // Faces along Y (axis 1) at depth 9: across them run Z, then X.
        let (v, a, d) = (1, 1, 9);
        let (u, w) = (5, 3); // t[2] = 5, t[0] = 3
        let tile = |k: usize| px((k * 16 + d) * 16 + u, (3 * v + a) * 16 + w);
        assert_eq!(&tile(0)[..3], [3 * 16, 9 * 16, 5 * 16], "texel (3, 9, 5)");
        assert_eq!(&tile(1)[..3], [3 * 8, 9 * 16, 5 * 16], "the second variant's");
        assert_eq!(&tile(2)[..3], &tile(0)[..3], "two variants repeat to fill the rest");
        assert_eq!(&tile(VARIANTS - 1)[..3], &tile(1)[..3]);
        // Every level of the red look's tiles is red: nothing bleeds in from its neighbors.
        let mut start = 0;
        for k in 0..LEVELS as usize {
            let (lw, lh) = (width >> k, height >> k);
            let tile = 16 >> k;
            for y in 2 * 3 * tile..lh {
                for x in 0..lw {
                    let at = start + 4 * (y * lw + x);
                    assert_eq!(&atlas[at..at + 3], [255, 0, 0], "level {k} at {x} {y}");
                }
            }
            start += lw * lh * 4;
        }
        // Level 1 averages two by two across the face, in linear light, and keeps the depth:
        // faces along Z at depth 4, texels x and y from 0 to 1.
        let level1 = width * height * 4;
        // Volume 1's row of slices facing along Z (row 3 + 2), column 4, at half size.
        let at = level1 + 4 * ((3 + 2) * 8 * (width / 2) + 4 * 8);
        let mean = |a: u8, b: u8| linear_to_srgb((srgb_to_linear(a) + srgb_to_linear(b)) / 2.0);
        assert_eq!(&atlas[at..at + 3], [mean(0, 16), mean(0, 16), 4 * 16]);
    }

    #[test]
    fn every_looks_last_mip_is_what_its_faces_show_on_average() {
        // Far off, a face is drawn from its slice's last mip, so across the variants that one
        // pixel has to be the average of what the face shows up close.
        let sim = crate::world::testing::base_sim();
        let looks = sim.looks();
        let volumes = Volumes::build(looks, sim.blocks());
        let [width, height] = volumes.atlas_size().map(|n| n as usize);
        let atlas = volumes.atlas();
        let last = LEVELS as usize - 1;
        let start: usize = (0..last).map(|k| (width >> k) * (height >> k) * 4).sum();
        let size = LookDef::SIZE;
        let mut wrong = Vec::new();
        for (v, look) in looks.iter().enumerate() {
            let v = v + 1;
            for a in 0..3 {
                for d in [0, size - 1] {
                    let (mut shown, mut mip) = ([0.0f64; 3], [0.0f64; 3]);
                    for k in 0..VARIANTS {
                        let cube = look.variant(k % look.variants());
                        for j in 0..size * size {
                            let mut t = [0; 3];
                            t[a] = d;
                            t[(a + 1) % 3] = j % size;
                            t[(a + 2) % 3] = j / size;
                            let i = LookDef::index(t[0], t[1], t[2]);
                            for c in 0..3 {
                                shown[c] += srgb_to_linear(cube[4 * i + c]) as f64;
                            }
                        }
                        let at = start + 4 * ((3 * v + a) * (width >> last) + k * size + d);
                        for c in 0..3 {
                            mip[c] += srgb_to_linear(atlas[at + c]) as f64;
                        }
                    }
                    let want = shown.map(|s| linear_to_srgb((s / (VARIANTS * size * size) as f64) as f32));
                    let got = mip.map(|s| linear_to_srgb((s / VARIANTS as f64) as f32));
                    if (0..3).any(|k| (got[k] as i32 - want[k] as i32).abs() > 2) {
                        wrong.push(format!("{} facing axis {a} at depth {d}: mips {got:?}, shown {want:?}", look.name));
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn variants_are_picked_evenly_without_a_pattern() {
        for n in [1, 4, 8, 16] {
            let mut counts = vec![0u32; n as usize];
            for z in 0..8u32 {
                for y in 0..32u32 {
                    for x in 0..32u32 {
                        counts[variant_of([x, y, z.wrapping_sub(4)], n) as usize] += 1;
                    }
                }
            }
            let even = 8 * 32 * 32 / n;
            assert!(counts.iter().all(|&c| c.abs_diff(even) * 5 < even), "{n}: {counts:?}");
        }
        // Neighbors along each axis match about as often as chance, not always or never.
        for axis in 0..3 {
            let mut same = 0;
            for i in 0..4096u32 {
                let p = [i % 16, i / 16 % 16, i / 256];
                let mut q = p;
                q[axis] += 1;
                same += u32::from(variant_of(p, 8) == variant_of(q, 8));
            }
            assert!((300..=724).contains(&same), "axis {axis}: {same} of 4096 neighbors match");
        }
        // One variant is all there is.
        assert_eq!(variant_of([5, 6, 7], 1), 0);
    }

    use api::BlockDef;

    fn look(name: &str, color: [u8; 4], turns: bool) -> LookDef {
        LookDef { name: name.into(), turns, rgba: color.repeat(LookDef::VOLUME), sheen: None, glow: 0.0 }
    }

    #[test]
    fn blocks_find_their_volumes() {
        let mut looks = LookRegistry::default();
        looks.register(look("test:red", [255, 0, 0, 255], false)).unwrap();
        looks.register(look("test:green", [0, 255, 0, 255], true)).unwrap();
        let mut blocks = BlockRegistry::default();
        let grass = blocks.register(BlockDef::new("test:grass", true, [0; 3]).with_look("test:green")).unwrap();
        let slab = blocks.register(BlockDef::new("test:slab", true, [0; 3]).with_look("test:green")).unwrap();
        let plain = blocks.register(BlockDef::new("test:plain", true, [9; 3])).unwrap();
        let volumes = Volumes::build(&looks, &blocks);
        assert_eq!(volumes.count, 3);
        assert_eq!(volumes.rgba.len(), 3 * VARIANTS * LookDef::VOLUME * 4);
        assert_eq!((volumes.of(grass), volumes.of(slab)), (Some(2), Some(2)), "blocks share their look's volume");
        assert_eq!((volumes.of(plain), volumes.of(BlockId::EMPTY)), (None, None));
        assert!(volumes.turns(grass) && volumes.turns(slab), "every shape follows its look's rule");
        assert!(!volumes.turns(plain));
        assert_eq!(volumes.sheens, [[0.0, 1.0, 0.0]; 3], "matte looks, white too");

        // A shiny look's sheen sits at its volume.
        let mut shiny = LookRegistry::default();
        shiny.register(look("test:matte", [1, 1, 1, 255], false)).unwrap();
        shiny.register(LookDef { sheen: Some(api::Sheen { strength: 0.5, sharpness: 40.0 }), ..look("test:ice", [9, 9, 9, 255], false) }).unwrap();
        assert_eq!(Volumes::build(&shiny, &blocks).sheens, [[0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.5, 40.0, 0.0]]);

        // A glowing look's glow sits beside its sheen.
        let mut glowing = LookRegistry::default();
        glowing.register(LookDef { glow: 1.5, ..look("test:lamp", [9, 9, 9, 255], false) }).unwrap();
        assert_eq!(Volumes::build(&glowing, &blocks).sheens[1], [0.0, 1.0, 1.5]);

        // A block registered later, as crafting does, finds its look once synced.
        let mut volumes = volumes;
        let later = blocks.register(BlockDef::new("test:later", true, [0; 3]).with_look("test:red")).unwrap();
        assert_eq!(volumes.of(later), None);
        volumes.sync(&blocks);
        assert_eq!((volumes.of(later), volumes.of(grass)), (Some(1), Some(2)));

        // Texel (x, y, z) of volume v's variant k is at z + (v * VARIANTS + k) * SIZE in the stack.
        let texel = |v: usize, x: usize, y: usize, z: usize| {
            let at = 4 * LookDef::index(x, y, z + (v * VARIANTS + 3) * LookDef::SIZE);
            &volumes.rgba[at..at + 4]
        };
        for (x, y, z) in [(0, 0, 0), (15, 15, 15), (3, 9, 12)] {
            assert_eq!(texel(0, x, y, z), [255; 4], "white first");
            assert_eq!(texel(1, x, y, z), [255, 0, 0, 255]);
            assert_eq!(texel(2, x, y, z), [0, 255, 0, 255]);
        }
    }
}
