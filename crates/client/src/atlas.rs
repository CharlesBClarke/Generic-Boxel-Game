//! Packs every registered texture into one image, so all chunks draw with one material.
//! Plain pixels and UVs; turning it into a Bevy image happens elsewhere.

use sim::{BlockId, BlockRegistry, TextureRegistry};

/// Where a texture sits in the atlas, in UV space: 0 to 1 across the whole atlas, `min`
/// at the top left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

/// Textures packed into a grid of equal cells. Each cell has a one-pixel border copied from
/// the texture's edge, so filtering never picks up a neighbor's pixels. The first cell is
/// plain white, for blocks drawn in a flat vertex color.
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    /// Row by row from the top left, four bytes a pixel, sRGB.
    pub rgba: Vec<u8>,
    white: Rect,
    /// Top, bottom and side for each block, by block ID; `None` for untextured blocks.
    faces: Vec<Option<[Rect; 3]>>,
}

const BORDER: u32 = 1;

impl Atlas {
    pub fn build(textures: &TextureRegistry, blocks: &BlockRegistry) -> Self {
        let list: Vec<_> = textures.iter().collect();
        let size = list.iter().map(|t| t.width.max(t.height)).max().unwrap_or(1);
        let cell = size + 2 * BORDER;
        let cells = list.len() as u32 + 1;
        let columns = (cells as f32).sqrt().ceil() as u32;
        let rows = cells.div_ceil(columns);
        let (width, height) = (columns * cell, rows * cell);
        let mut rgba = vec![0; (width * height * 4) as usize];

        let mut rects = std::collections::HashMap::new();
        let mut place = |index: u32, w: u32, h: u32, pixel: &dyn Fn(u32, u32) -> [u8; 4]| -> Rect {
            let (x0, y0) = ((index % columns) * cell, (index / columns) * cell);
            // The texture plus its border, edge pixels repeated outward.
            for y in 0..h + 2 * BORDER {
                for x in 0..w + 2 * BORDER {
                    let src = pixel(x.saturating_sub(BORDER).min(w - 1), y.saturating_sub(BORDER).min(h - 1));
                    let at = (((y0 + y) * width + x0 + x) * 4) as usize;
                    rgba[at..at + 4].copy_from_slice(&src);
                }
            }
            let uv = |x: u32, y: u32| [x as f32 / width as f32, y as f32 / height as f32];
            Rect { min: uv(x0 + BORDER, y0 + BORDER), max: uv(x0 + BORDER + w, y0 + BORDER + h) }
        };
        let white = place(0, 1, 1, &|_, _| [255; 4]);
        for (i, t) in list.iter().enumerate() {
            let pixel = |x: u32, y: u32| {
                let at = ((y * t.width + x) * 4) as usize;
                t.rgba[at..at + 4].try_into().unwrap()
            };
            rects.insert(t.name.as_str(), place(i as u32 + 1, t.width, t.height, &pixel));
        }

        let mut faces = vec![None; blocks.len() + 1];
        for (id, def) in blocks.iter() {
            // The simulation has checked every name exists; a missing one would draw white.
            faces[id.index()] =
                def.textures.as_ref().map(|t| t.names().map(|n| rects.get(n).copied().unwrap_or(white)));
        }
        Atlas { width, height, rgba, white, faces }
    }

    /// The plain white cell, for flat-colored faces.
    pub fn white(&self) -> Rect {
        self.white
    }

    /// The top, bottom and side textures of a block, or `None` if it has no textures.
    pub fn faces(&self, id: BlockId) -> Option<&[Rect; 3]> {
        self.faces.get(id.index()).and_then(Option::as_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{BlockDef, FaceTextures, TextureDef};

    fn texture(name: &str, size: u32, color: [u8; 4]) -> TextureDef {
        TextureDef { name: name.into(), width: size, height: size, rgba: color.repeat((size * size) as usize) }
    }

    fn setup() -> (TextureRegistry, BlockRegistry) {
        let mut textures = TextureRegistry::default();
        textures.register(texture("test:red", 4, [255, 0, 0, 255])).unwrap();
        textures.register(texture("test:green", 4, [0, 255, 0, 255])).unwrap();
        textures.register(texture("test:small", 2, [0, 0, 255, 255])).unwrap();
        let mut blocks = BlockRegistry::default();
        let grass = FaceTextures { top: "test:green".into(), bottom: "test:red".into(), side: "test:small".into() };
        blocks.register(BlockDef::new("test:grass", true, [0; 3]).with_textures(grass)).unwrap();
        blocks.register(BlockDef::new("test:plain", true, [9; 3])).unwrap();
        (textures, blocks)
    }

    /// The color at a UV point.
    fn sample(atlas: &Atlas, uv: [f32; 2]) -> [u8; 4] {
        let (x, y) = ((uv[0] * atlas.width as f32) as u32, (uv[1] * atlas.height as f32) as u32);
        let at = ((y * atlas.width + x) * 4) as usize;
        atlas.rgba[at..at + 4].try_into().unwrap()
    }

    fn all_rects(atlas: &Atlas, blocks: &BlockRegistry) -> Vec<Rect> {
        let mut rects = vec![atlas.white()];
        rects.extend(atlas.faces(blocks.id("test:grass").unwrap()).unwrap());
        rects
    }

    #[test]
    fn faces_find_their_textures() {
        let (textures, blocks) = setup();
        let atlas = Atlas::build(&textures, &blocks);
        let [top, bottom, side] = *atlas.faces(blocks.id("test:grass").unwrap()).unwrap();
        // Sample every pixel's center, edges included.
        for (rect, color) in
            [(top, [0, 255, 0, 255]), (bottom, [255, 0, 0, 255]), (side, [0, 0, 255, 255]), (atlas.white(), [255; 4])]
        {
            let (w, h) =
                ((rect.max[0] - rect.min[0]) * atlas.width as f32, (rect.max[1] - rect.min[1]) * atlas.height as f32);
            for y in 0..h.round() as u32 {
                for x in 0..w.round() as u32 {
                    let uv = [
                        rect.min[0] + (x as f32 + 0.5) / atlas.width as f32,
                        rect.min[1] + (y as f32 + 0.5) / atlas.height as f32,
                    ];
                    assert_eq!(sample(&atlas, uv), color, "{rect:?} at {x}, {y}");
                }
            }
        }
        assert!(atlas.faces(blocks.id("test:plain").unwrap()).is_none());
        assert!(atlas.faces(BlockId::EMPTY).is_none());
    }

    #[test]
    fn textures_do_not_overlap_and_keep_their_size() {
        let (textures, blocks) = setup();
        let atlas = Atlas::build(&textures, &blocks);
        let rects = all_rects(&atlas, &blocks);
        let px = |r: &Rect| {
            let x = |u: f32| (u * atlas.width as f32).round() as i32;
            let y = |v: f32| (v * atlas.height as f32).round() as i32;
            (x(r.min[0]), y(r.min[1]), x(r.max[0]), y(r.max[1]))
        };
        let sizes: Vec<_> = rects
            .iter()
            .map(|r| {
                let (x0, y0, x1, y1) = px(r);
                (x1 - x0, y1 - y0)
            })
            .collect();
        assert_eq!(sizes, [(1, 1), (4, 4), (4, 4), (2, 2)]);
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                let ((ax0, ay0, ax1, ay1), (bx0, by0, bx1, by1)) = (px(a), px(b));
                // A border pixel apart at least.
                let apart = ax1 < bx0 || bx1 < ax0 || ay1 < by0 || by1 < ay0;
                assert!(apart, "{a:?} and {b:?} touch");
            }
        }
    }

    #[test]
    fn borders_repeat_the_edge() {
        let (textures, blocks) = setup();
        let atlas = Atlas::build(&textures, &blocks);
        let [top, ..] = *atlas.faces(blocks.id("test:grass").unwrap()).unwrap();
        // Half a pixel outside the texture lands in the border, which matches the edge.
        let outside = [top.min[0] - 0.5 / atlas.width as f32, top.min[1] - 0.5 / atlas.height as f32];
        assert_eq!(sample(&atlas, outside), [0, 255, 0, 255]);
    }

    #[test]
    fn no_textures_is_just_white() {
        let atlas = Atlas::build(&TextureRegistry::default(), &BlockRegistry::default());
        assert_eq!((atlas.width, atlas.height), (3, 3));
        assert_eq!(sample(&atlas, [0.5, 0.5]), [255; 4]);
    }
}
