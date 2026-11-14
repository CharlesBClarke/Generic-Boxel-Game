//! Stacks every registered look into one image, so all chunks draw with one material: the
//! volumes one after another, and the same texels cut into face slices with mip levels (see
//! [`Volumes::atlas`]). Plain texels; turning them into Bevy images happens elsewhere.

use api::LookDef;
use api::look::{LookDetail, srgb_to_linear};
use sim::{BlockId, BlockRegistry, LookRegistry};

/// Every look's volume, stacked along Z, one after another. The first is plain white, for
/// blocks drawn in a flat vertex color.
pub struct Volumes {
    /// How many volumes are stacked.
    pub count: u32,
    /// Four bytes a texel, sRGB: X fastest, then Y, then Z through every volume in turn.
    pub rgba: Vec<u8>,
    /// Each volume's sheen as strength and sharpness, then its glow, by volume; zero strength
    /// for matte ones, zero glow for ones that don't glow.
    pub sheens: Vec<[f32; 3]>,
    /// Which volume each block uses and whether its look turns with it, by block ID; `None`
    /// for blocks without a look.
    of_block: Vec<Option<(u32, bool)>>,
    /// What each volume generates as it's drawn, by volume; nothing for the white one.
    pub details: Vec<LookDetail>,
    /// Each look's volume and whether it turns, by name, for blocks registered later.
    by_look: std::collections::HashMap<String, (u32, bool)>,
}

impl Volumes {
    /// The plain white volume.
    pub const WHITE: u32 = 0;

    pub fn build(looks: &LookRegistry, blocks: &BlockRegistry) -> Self {
        // A look's texels run X, then Y, then Z, so stacking along Z is just one after another.
        let mut rgba = vec![255; LookDef::VOLUME * 4];
        let mut by_look = std::collections::HashMap::new();
        let mut sheens = vec![[0.0, 1.0, 0.0]];
        let mut details = vec![LookDetail::default()];
        for (i, look) in looks.iter().enumerate() {
            details.push(look.detail.clone());
            rgba.extend_from_slice(&resolved(look));
            by_look.insert(look.name.clone(), (i as u32 + 1, look.turns));
            let [s, k] = look.sheen.map_or([0.0, 1.0], |s| [s.strength, s.sharpness]);
            sheens.push([s, k, look.glow]);
        }
        let mut volumes = Volumes { count: looks.iter().count() as u32 + 1, rgba, sheens, details, of_block: vec![None], by_look };
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
        [size * size, self.count * 3 * size]
    }

    /// The looks cut into face slices, every mip level one after another from level 0, four
    /// bytes a pixel, rows of [`Volumes::atlas_size`]'s width (halved each level). Volume `v`'s
    /// texels at depth `d` along axis `a` (the faces facing along `a` show them) are the 16×16
    /// tile in column `d`, row `3v + a`, laid out across the face's other two axes in order:
    /// pixel `(t[(a + 1) % 3], t[(a + 2) % 3])` of the tile is texel `t`. Each level averages
    /// two by two pixels of the one before, in linear light, so a tile shrinks within itself
    /// and never takes color from the next, and a face's texels never mix with the layers under
    /// them. Level 0's alpha holds each texel's detail byte (see [`api::look::LookDetail`]).
    pub fn atlas(&self) -> Vec<u8> {
        let size = LookDef::SIZE;
        let [width, height] = self.atlas_size().map(|n| n as usize);
        let mut level = vec![0u8; width * height * 4];
        for v in 0..self.count as usize {
            let detail = &self.details[v].texels;
            for a in 0..3 {
                let (u, w) = ((a + 1) % 3, (a + 2) % 3);
                for i in 0..LookDef::VOLUME {
                    let t = [i % size, i / size % size, i / (size * size)];
                    let x = t[a] * size + t[u];
                    let y = (3 * v + a) * size + t[w];
                    let from = 4 * (v * LookDef::VOLUME + i);
                    let to = 4 * (y * width + x);
                    level[to..to + 3].copy_from_slice(&self.rgba[from..from + 3]);
                    level[to + 3] = detail.get(i).copied().unwrap_or(0);
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

/// `look`'s texels as they look on average, so mip levels made from them (see
/// [`Volumes::atlas`]) are what its faces show from afar: a texel taking its color from noise
/// gets the noise's average over the world in place of its baked color. Grain needs nothing,
/// as it's drawn to average out to the texel's own color.
fn resolved(look: &LookDef) -> Vec<u8> {
    let mut rgba = look.rgba.clone();
    if look.detail.texels.is_empty() {
        return rgba;
    }
    let averages: Vec<[u8; 3]> = look.detail.noise.iter().map(|n| n.average().map(linear_to_srgb)).collect();
    for (i, &byte) in look.detail.texels.iter().enumerate() {
        let layer = api::look::detail_layer(byte) as usize;
        if layer > 0 {
            rgba[4 * i..4 * i + 3].copy_from_slice(&averages[layer - 1]);
        }
    }
    rgba
}

/// Mip levels of [`Volumes::atlas`]: down to one pixel a slice.
pub const LEVELS: u32 = 5;

fn linear_to_srgb(c: f32) -> u8 {
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_hold_each_face_layer_and_shrink_within_themselves() {
        let mut looks = LookRegistry::default();
        // A look whose texels say where they are: red is x, green y, blue z (times 16).
        let rgba: Vec<u8> = (0..LookDef::VOLUME)
            .flat_map(|i| [(i % 16 * 16) as u8, (i / 16 % 16 * 16) as u8, (i / 256 * 16) as u8, 255])
            .collect();
        let detail = LookDetail { texels: (0..LookDef::VOLUME).map(|i| ((i % 7) << 4) as u8).collect(), ..Default::default() };
        looks.register(LookDef { name: "test:where".into(), turns: false, rgba, sheen: None, glow: 0.0, detail }).unwrap();
        looks.register(look("test:red", [255, 0, 0, 255], false)).unwrap();
        let volumes = Volumes::build(&looks, &BlockRegistry::default());
        let [width, height] = volumes.atlas_size().map(|n| n as usize);
        assert_eq!([width, height], [256, 3 * 3 * 16]);
        let atlas = volumes.atlas();
        let total: usize = (0..LEVELS).map(|k| (width >> k) * (height >> k) * 4).sum();
        assert_eq!(atlas.len(), total);
        let px = |x: usize, y: usize| &atlas[4 * (y * width + x)..4 * (y * width + x) + 4];
        // Faces along Y (axis 1) at depth 9: across them run Z, then X.
        let (v, a, d) = (1, 1, 9);
        let (u, w) = (5, 3); // t[2] = 5, t[0] = 3
        let p = px(d * 16 + u, (3 * v + a) * 16 + w);
        assert_eq!(&p[..3], [3 * 16, 9 * 16, 5 * 16], "texel (3, 9, 5)");
        assert_eq!(p[3], ((LookDef::index(3, 9, 5) % 7) << 4) as u8, "its detail byte in alpha");
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
    /// What a pixel up close shows of texel `i` of `look` at world texel `p`, linear RGB: the
    /// shader's `look_color` with nothing faded. `grain_mean` is the grain's average factor.
    fn drawn(look: &LookDef, grain_mean: f32, i: usize, p: [u32; 3]) -> [f32; 3] {
        let byte = look.detail.texels.get(i).copied().unwrap_or(0);
        let layer = api::look::detail_layer(byte) as usize;
        if layer > 0 {
            return look.detail.noise[layer - 1].color(p).map(srgb_to_linear);
        }
        let baked = [0, 1, 2].map(|c| srgb_to_linear(look.rgba[4 * i + c]));
        let grain = api::look::detail_grain(byte);
        match &look.detail.grain {
            Some(g) if grain > 0 => baked.map(|c| c * g.factor(grain, api::look::detail_board(byte), p) / grain_mean),
            _ => baked,
        }
    }

    #[test]
    fn every_looks_last_mip_is_what_its_faces_show_on_average() {
        // Far off, a face is drawn from its slice's last mip (far terrain always, full detail
        // once its detail fades), so that one pixel has to be the average of what the face
        // shows up close, noise and grain included, over many boxels.
        let sim = crate::world::testing::base_sim();
        let looks = sim.looks();
        let volumes = Volumes::build(looks, sim.blocks());
        let [width, height] = volumes.atlas_size().map(|n| n as usize);
        let atlas = volumes.atlas();
        let last = LEVELS as usize - 1;
        let start: usize = (0..last).map(|k| (width >> k) * (height >> k) * 4).sum();
        let size = LookDef::SIZE;
        // Boxels far apart, so many blobs and streaks are counted.
        let boxels: Vec<[i64; 3]> = (0..64i64).map(|b| [b % 4 * 37, b / 4 % 4 * 53, b / 16 * 41]).collect();
        let mut wrong = Vec::new();
        for (v, look) in looks.iter().enumerate() {
            let v = v + 1;
            let grain_mean = look.detail.grain.map_or(1.0, |g| api::look::Grain::mean_factor(g.strength, g.mean_value()));
            for a in 0..3 {
                for d in [0, size - 1] {
                    let mut sum = [0.0f64; 3];
                    let mut n = 0.0;
                    for b in &boxels {
                        for j in 0..size * size {
                            let mut t = [0; 3];
                            t[a] = d;
                            t[(a + 1) % 3] = j % size;
                            t[(a + 2) % 3] = j / size;
                            let i = LookDef::index(t[0], t[1], t[2]);
                            let world = [0, 1, 2].map(|k| b[k] * size as i64 + t[k] as i64);
                            let c = drawn(look, grain_mean, i, api::look::world_texel(world));
                            for k in 0..3 {
                                sum[k] += c[k] as f64;
                            }
                            n += 1.0;
                        }
                    }
                    let want = sum.map(|s| linear_to_srgb((s / n) as f32));
                    let at = start + 4 * ((3 * v + a) * (width >> last) + d);
                    let got = &atlas[at..at + 3];
                    if (0..3).any(|k| (got[k] as i32 - want[k] as i32).abs() > 3) {
                        wrong.push(format!("{} facing axis {a} at depth {d}: mip {got:?}, shown {want:?}", look.name));
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    use api::BlockDef;

    fn look(name: &str, color: [u8; 4], turns: bool) -> LookDef {
        LookDef { name: name.into(), turns, rgba: color.repeat(LookDef::VOLUME), sheen: None, glow: 0.0, detail: Default::default() }
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
        assert_eq!(volumes.rgba.len(), 3 * LookDef::VOLUME * 4);
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

        // Texel (x, y, z) of volume v is at z + v * SIZE in the stack.
        let texel = |v: usize, x: usize, y: usize, z: usize| {
            let at = 4 * LookDef::index(x, y, z + v * LookDef::SIZE);
            &volumes.rgba[at..at + 4]
        };
        for (x, y, z) in [(0, 0, 0), (15, 15, 15), (3, 9, 12)] {
            assert_eq!(texel(0, x, y, z), [255; 4], "white first");
            assert_eq!(texel(1, x, y, z), [255, 0, 0, 255]);
            assert_eq!(texel(2, x, y, z), [0, 255, 0, 255]);
        }
    }
}
