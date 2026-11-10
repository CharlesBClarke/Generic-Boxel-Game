//! A height map of the world for the far shadow (see [`crate::shadows`]): the top of each column
//! of boxels, as a texture around the camera that `solid.wgsl` steps across toward the sun.
//! Coarse on purpose: a column is as tall as the highest thing in it, so overhangs read as
//! solid down to the ground, and far away one texel covers several columns. Hidden pieces don't
//! count. Edits update their column; the texture follows the camera in jumps.

use std::collections::{HashMap, HashSet};

use api::{CHUNK_SIZE, ChunkPos, I64Vec3};
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sim::{BlockRegistry, CellId, Grid, Layer};

use crate::RenderOrigin;
use crate::view::ViewTuning;
use crate::world::ClientWorld;

/// Texels along each side of the texture.
pub const SIZE: usize = 1024;

/// What a texel holds where nothing is in its columns: far below anything.
pub const NOTHING: f32 = -1.0e5;

const CHUNK: usize = CHUNK_SIZE as usize;
/// Columns in a chunk, 32 × 32, x fastest.
const COLUMNS: usize = CHUNK * CHUNK;

/// The top of each column in one column of chunks: the Z of the highest filled boxel's top
/// face, or `None` where every chunk in it is empty there.
type Tops = Box<[Option<i64>; COLUMNS]>;

/// The world's column tops, and the texture made from them around the camera.
#[derive(Resource, Default)]
pub struct Heights {
    /// Which chunks each column of chunks holds, by their Z.
    stacks: HashMap<(i64, i64), Vec<i64>>,
    tops: HashMap<(i64, i64), Tops>,
    /// Columns of chunks to work out again.
    pending: HashSet<(i64, i64)>,
    /// Whether every chunk has been read once.
    started: bool,
    /// Where the texture is, once built.
    pub placed: Option<Placement>,
    /// The texture, replaced by a new one each time it changes so materials pick it up.
    pub image: Option<Handle<Image>>,
}

/// Where the texture lies in the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// The boxel column (X, Y) at the corner of texel (0, 0).
    pub corner: (i64, i64),
    /// Columns along each side of a texel.
    pub scale: i64,
    /// The Z texel heights are measured from.
    pub base: i64,
}

impl Placement {
    /// Texture placement covering `distance` meters around the column `eye`, in jumps of an
    /// eighth of its width so it moves rarely; heights measured from `base`.
    pub fn around(eye: I64Vec3, distance: f32, base: i64) -> Placement {
        let wanted = (2.0 * (distance.max(1.0) + CHUNK_SIZE as f32)) / SIZE as f32;
        let scale = (wanted.ceil().max(1.0) as u64).next_power_of_two() as i64;
        let width = SIZE as i64 * scale;
        let jump = width / 8;
        let snap = |v: i64| v.div_euclid(jump) * jump - width / 2;
        Placement { corner: (snap(eye.x), snap(eye.y)), scale, base }
    }

    /// Whether `eye` is still well inside, so the texture needn't move.
    pub fn covers(&self, eye: I64Vec3, distance: f32) -> bool {
        let fresh = Placement::around(eye, distance, self.base);
        fresh.scale == self.scale && fresh.corner == self.corner
    }

    /// The texel holding column (x, y), if any.
    pub fn texel(&self, x: i64, y: i64) -> Option<usize> {
        let i = (x - self.corner.0).div_euclid(self.scale);
        let j = (y - self.corner.1).div_euclid(self.scale);
        let size = SIZE as i64;
        ((0..size).contains(&i) && (0..size).contains(&j)).then(|| (i + j * size) as usize)
    }
}

/// Whether any of the occupants a cell holds is shown.
fn shown(grid: &Grid, blocks: &BlockRegistry, id: CellId) -> bool {
    id != CellId::EMPTY && grid.occupants(id).iter().any(|o| !blocks.is_hidden(o.block))
}

/// The tops of a column of chunks, from its chunks at the Zs in `stack`.
pub fn column_tops(grid: &Grid, blocks: &BlockRegistry, column: (i64, i64), stack: &[i64]) -> Tops {
    let mut tops: Tops = Box::new([None; COLUMNS]);
    let mut left = COLUMNS;
    let mut known: HashMap<CellId, bool> = HashMap::new();
    let mut zs = stack.to_vec();
    zs.sort_unstable_by(|a, b| b.cmp(a));
    for cz in zs {
        let pos = ChunkPos(I64Vec3::new(column.0, column.1, cz));
        let Some(chunk) = grid.chunk(pos) else { continue };
        let floor = pos.origin().0.z;
        match &chunk.cells {
            Layer::Uniform(id) => {
                if shown(grid, blocks, *id) {
                    for top in tops.iter_mut().filter(|t| t.is_none()) {
                        *top = Some(floor + CHUNK_SIZE);
                    }
                    left = 0;
                }
            }
            Layer::Dense(cells) => {
                for (column, top) in tops.iter_mut().enumerate().filter(|(_, t)| t.is_none()) {
                    for z in (0..CHUNK).rev() {
                        let id = cells[column + z * COLUMNS];
                        if *known.entry(id).or_insert_with(|| shown(grid, blocks, id)) {
                            *top = Some(floor + z as i64 + 1);
                            left -= 1;
                            break;
                        }
                    }
                }
            }
        }
        if left == 0 {
            break;
        }
    }
    tops
}

impl Heights {
    /// Notes chunks that changed, so their columns are worked out again.
    pub fn changed(&mut self, chunks: &[ChunkPos]) {
        for pos in chunks {
            let column = (pos.0.x, pos.0.y);
            let stack = self.stacks.entry(column).or_default();
            if !stack.contains(&pos.0.z) {
                stack.push(pos.0.z);
            }
            self.pending.insert(column);
        }
    }

    /// Works out pending columns again. Returns whether any changed.
    fn refresh(&mut self, grid: &Grid, blocks: &BlockRegistry) -> bool {
        if !self.started {
            self.started = true;
            let all: Vec<ChunkPos> = grid.chunks().map(|(pos, _)| pos).collect();
            self.changed(&all);
        }
        let mut any = false;
        for column in std::mem::take(&mut self.pending) {
            let tops = column_tops(grid, blocks, column, self.stacks.get(&column).map_or(&[], Vec::as_slice));
            if self.tops.get(&column) != Some(&tops) {
                self.tops.insert(column, tops);
                any = true;
            }
        }
        any
    }

    /// The texels for `placed`: each the highest top among its columns, from `placed.base`.
    pub fn texels(&self, placed: Placement) -> Vec<f32> {
        let mut texels = vec![NOTHING; SIZE * SIZE];
        let width = SIZE as i64 * placed.scale;
        let (x0, y0) = placed.corner;
        for (&(cx, cy), tops) in &self.tops {
            let (bx, by) = (cx * CHUNK_SIZE, cy * CHUNK_SIZE);
            if bx + CHUNK_SIZE <= x0 || by + CHUNK_SIZE <= y0 || bx >= x0 + width || by >= y0 + width {
                continue;
            }
            for (i, top) in tops.iter().enumerate() {
                let Some(top) = top else { continue };
                let (x, y) = (bx + (i % CHUNK) as i64, by + (i / CHUNK) as i64);
                if let Some(t) = placed.texel(x, y) {
                    texels[t] = texels[t].max((top - placed.base) as f32);
                }
            }
        }
        texels
    }
}

/// A texture of heights, read texel by texel.
pub fn height_image(texels: &[f32], size: usize) -> Image {
    let mut image = Image::new(
        Extent3d { width: size as u32, height: size as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        texels.iter().flat_map(|t| t.to_le_bytes()).collect(),
        TextureFormat::R32Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// Works out changed columns and rebuilds the texture when they or the camera's place call for it.
pub(crate) fn update_heights(
    world: Res<ClientWorld>,
    origin: Res<RenderOrigin>,
    view: Res<ViewTuning>,
    camera: Single<&Transform, With<crate::PlayerCamera>>,
    mut heights: ResMut<Heights>,
    mut images: ResMut<Assets<Image>>,
) {
    let grid = world.universe().main();
    let changed = heights.refresh(grid, world.blocks());
    let eye = origin.cell + crate::coords::from_render(camera.translation).floor().as_i64vec3();
    let moved = !heights.placed.is_some_and(|p| p.covers(eye, view.render_distance));
    if !(changed || moved) {
        return;
    }
    let placed = match heights.placed {
        Some(p) if !moved => p,
        _ => Placement::around(eye, view.render_distance, eye.z),
    };
    let texels = heights.texels(placed);
    heights.image = Some(images.add(height_image(&texels, SIZE)));
    heights.placed = Some(placed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{BlockDef, BoxelPos};
    use sim::BlockId;

    fn setup() -> (Grid, BlockRegistry, BlockId) {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(BlockDef::new("test:stone", true, [128, 128, 128])).unwrap();
        (Grid::default(), blocks, stone)
    }

    fn fill_chunk(grid: &mut Grid, c: I64Vec3, id: BlockId) {
        let lists = vec![Vec::new(), vec![sim::Occupant::new(id)]];
        grid.import_chunk(&sim::ChunkData { pos: ChunkPos(c), lists, cells: Layer::Uniform(1), light: Layer::Uniform(sim::Light::DARK) });
    }

    fn heights_of(grid: &Grid, blocks: &BlockRegistry) -> Heights {
        let mut h = Heights::default();
        h.refresh(grid, blocks);
        h
    }

    #[test]
    fn a_columns_top_is_its_highest_filled_boxel() {
        let (mut grid, blocks, stone) = setup();
        fill_chunk(&mut grid, I64Vec3::new(0, 0, -1), stone);
        grid.set_block(BoxelPos::new(3, 4, 5), stone);
        grid.set_block(BoxelPos::new(3, 4, 40), stone);
        let h = heights_of(&grid, &blocks);
        let tops = &h.tops[&(0, 0)];
        assert_eq!(tops[3 + 4 * 32], Some(41), "the highest boxel, across chunks");
        assert_eq!(tops[0], Some(0), "the solid chunk below ends at 0");
        assert_eq!(h.tops.len(), 1);
    }

    #[test]
    fn empty_columns_have_no_top() {
        let (mut grid, blocks, stone) = setup();
        grid.set_block(BoxelPos::new(-1, -1, 7), stone);
        let h = heights_of(&grid, &blocks);
        let tops = &h.tops[&(-1, -1)];
        assert_eq!(tops[31 + 31 * 32], Some(8));
        assert_eq!(tops.iter().filter(|t| t.is_some()).count(), 1);
    }

    #[test]
    fn an_edit_updates_its_column() {
        let (mut grid, blocks, stone) = setup();
        grid.set_block(BoxelPos::new(1, 1, 1), stone);
        let mut h = heights_of(&grid, &blocks);
        grid.set_block(BoxelPos::new(1, 1, 70), stone);
        h.changed(&[ChunkPos(I64Vec3::new(0, 0, 2))]);
        assert!(h.refresh(&grid, &blocks));
        assert_eq!(h.tops[&(0, 0)][1 + 32], Some(71));
        grid.set_block(BoxelPos::new(1, 1, 70), BlockId::EMPTY);
        h.changed(&[ChunkPos(I64Vec3::new(0, 0, 2))]);
        assert!(h.refresh(&grid, &blocks));
        assert_eq!(h.tops[&(0, 0)][1 + 32], Some(2));
        h.changed(&[ChunkPos(I64Vec3::new(0, 0, 0))]);
        assert!(!h.refresh(&grid, &blocks), "nothing moved");
    }

    #[test]
    fn the_texture_covers_the_render_distance_and_moves_in_jumps() {
        let p = Placement::around(I64Vec3::new(10, 20, 30), 512.0, 30);
        assert_eq!(p.scale, 2, "1024 texels can't cover 2 × 544 columns one each");
        assert!(p.texel(10 - 544, 20 + 544).is_some() && p.texel(10 + 544, 20 - 544).is_some());
        assert_eq!(Placement::around(I64Vec3::ZERO, 300.0, 0).scale, 1);
        assert!(p.covers(I64Vec3::new(40, 50, 0), 512.0), "a small move keeps it");
        assert!(!p.covers(I64Vec3::new(400, 20, 0), 512.0), "a big one moves it");
        assert!(!p.covers(I64Vec3::new(10, 20, 0), 2000.0), "a longer distance rescales it");
    }

    #[test]
    fn a_texel_is_its_highest_column_from_the_base() {
        let (mut grid, blocks, stone) = setup();
        grid.set_block(BoxelPos::new(0, 0, 9), stone);
        grid.set_block(BoxelPos::new(1, 1, 4), stone);
        grid.set_block(BoxelPos::new(-3, 0, 2), stone);
        let h = heights_of(&grid, &blocks);
        let placed = Placement { corner: (-4, -4), scale: 2, base: 5 };
        let texels = h.texels(placed);
        let at = |x, y| texels[placed.texel(x, y).unwrap()];
        assert_eq!(at(0, 0), 5.0, "two columns share a texel: the higher wins");
        assert_eq!(at(-3, 0), -2.0);
        assert_eq!(at(10, 10), NOTHING);
    }
}
