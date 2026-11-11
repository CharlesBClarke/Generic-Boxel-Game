//! The material models are drawn with: cows, the held club and the chunks a hit sends flying.
//! Each fragment takes its texture's color times a plain color, then is lit like the world's
//! blocks (`model.wgsl`, sharing `sun.wgsl` with `solid.wgsl`): the sun through its shadows, the
//! sky and moon as far as stored sky light reaches, and light from blocks. A model carries no
//! stored light of its own, so whoever draws it reads the light where it stands
//! ([`stored_light`]) and eases toward it ([`ease_light`]). A hit's flash washes it out the same
//! by day or night.

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;

use api::WorldPos;
use sim::{BlockRegistry, Grid, Light};

use crate::shading::Atmosphere;

/// Seconds the light on a model takes to come most of the way (about 95%) to the light where it
/// now stands, so stepping between boxels or a lamp going on doesn't jump.
pub const LIGHT_EASE: f32 = 0.2;

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct ModelMaterial {
    /// Multiplied with the texture: white for a textured model, the color of a plain one.
    #[uniform(0)]
    pub color: LinearRgba,
    #[texture(1)]
    #[sampler(3)]
    pub texture: Handle<Image>,
    /// Kept up to date by `crate::shading`.
    #[uniform(2)]
    pub atmosphere: Atmosphere,
    /// The far shadow's height map, kept up to date by `crate::shading`.
    #[texture(4, sample_type = "float", filterable = false)]
    pub heights: Handle<Image>,
    /// The stored light where the model stands: red, green, blue and sky, 0 to 1.
    #[uniform(5)]
    pub light: Vec4,
    /// A flash: how much brighter than its texture in each channel, then how far toward that,
    /// 0 to 1.
    #[uniform(6)]
    pub flash: Vec4,
}

impl Material for ModelMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::from_path_buf(embedded_path!("model.wgsl")).with_source("embedded"))
    }
}

/// What every model material starts from: a white texture for plain ones, and a stand-in height
/// map until the real one is built.
#[derive(Resource, Clone)]
pub struct ModelMedia {
    pub white: Handle<Image>,
    pub heights: Handle<Image>,
}

impl ModelMedia {
    /// A material showing `texture` (white if none) times `color`, lit as under open sky until
    /// its light is set.
    pub fn material(&self, texture: Option<Handle<Image>>, color: Color) -> ModelMaterial {
        ModelMaterial {
            color: color.into(),
            texture: texture.unwrap_or_else(|| self.white.clone()),
            // Filled in by `shading` before the first frame is drawn.
            atmosphere: default(),
            heights: self.heights.clone(),
            light: Vec4::new(0.0, 0.0, 0.0, 1.0),
            flash: Vec4::new(1.0, 1.0, 1.0, 0.0),
        }
    }
}

fn setup_media(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let white = Image::new(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![255; 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    let heights = crate::heights::height_image(&[crate::heights::NOTHING; 2], 2, 1);
    commands.insert_resource(ModelMedia { white: images.add(white), heights: images.add(heights) });
}

/// Registers the material and its shader, and makes [`ModelMedia`] before startup.
pub struct ModelPlugin;

impl Plugin for ModelPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "model.wgsl");
        app.add_plugins(MaterialPlugin::<ModelMaterial>::default()).add_systems(PreStartup, setup_media);
    }
}

/// The stored light at `pos`, red, green, blue and sky, 0 to 1: blended between the middles of
/// the eight boxels around it, so it changes smoothly as something moves. Boxels closed on every
/// side (inside a solid block) hold no light anyone sees, so they're left out; if all eight are,
/// it's the light of the boxel `pos` is in.
pub fn stored_light(grid: &Grid, blocks: &BlockRegistry, pos: WorldPos) -> Vec4 {
    let to_vec = |l: Light| Vec4::from_array(std::array::from_fn(|c| l.channel(c) as f32 / Light::MAX as f32));
    // Measured from the middle of the boxel below and behind on each axis.
    let from = pos.offset() - api::Vec3::splat(0.5);
    let floor = from.floor();
    let frac = from - floor;
    let base = pos.cell() + api::I64Vec3::new(floor.x as i64, floor.y as i64, floor.z as i64);
    let (mut sum, mut weights) = (Vec4::ZERO, 0.0);
    for i in 0..8 {
        let bit = |k: usize| (i >> k) & 1;
        let at = api::BoxelPos(base + api::I64Vec3::new(bit(0) as i64, bit(1) as i64, bit(2) as i64));
        let weight: f32 = (0..3).map(|k| if bit(k) == 1 { frac[k] } else { 1.0 - frac[k] }).product();
        if weight <= 0.0 || sim::light::covered_faces_of(grid.cell(at), blocks) == sim::light::ALL_FACES {
            continue;
        }
        sum += to_vec(grid.light(at)) * weight;
        weights += weight;
    }
    if weights > 1e-6 { sum / weights } else { to_vec(grid.light(pos.boxel())) }
}

/// `now` moved toward `want` for `dt` seconds, coming most of the way in [`LIGHT_EASE`].
pub fn ease_light(now: Vec4, want: Vec4, dt: f32) -> Vec4 {
    now + (want - now) * (1.0 - (-3.0 * dt / LIGHT_EASE).exp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{BlockDef, I64Vec3, Vec3 as ZVec3};

    /// A world of open sky, with a roofed hut of stone around the origin, lit.
    fn hut() -> (Grid, BlockRegistry) {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(BlockDef::new("test:stone", true, [128; 3])).unwrap();
        let mut grid = Grid::default();
        for x in -3..=3i64 {
            for y in -3..=3i64 {
                for z in -1..=4i64 {
                    let wall = x.abs() == 3 || y.abs() == 3 || z == -1 || z == 4;
                    if wall {
                        grid.set_block(api::BoxelPos::new(x, y, z), stone);
                    }
                }
            }
        }
        grid.relight_all(&blocks);
        (grid, blocks)
    }

    #[test]
    fn inside_a_closed_hut_there_is_no_sky_light_and_outside_there_is() {
        let (grid, blocks) = hut();
        let inside = stored_light(&grid, &blocks, WorldPos::new(I64Vec3::new(0, 0, 0), ZVec3::new(0.5, 0.5, 0.7)));
        let outside = stored_light(&grid, &blocks, WorldPos::new(I64Vec3::new(10, 10, 0), ZVec3::new(0.5, 0.5, 0.7)));
        assert!(inside.w < 0.01, "sky light in the hut: {inside}");
        assert!(outside.w > 0.99, "sky light outside: {outside}");
    }

    #[test]
    fn the_walls_themselves_do_not_darken_the_light_beside_them() {
        let (grid, blocks) = hut();
        // Right by the hut's outer wall, half the boxels around are inside the stone.
        let by_wall = stored_light(&grid, &blocks, WorldPos::new(I64Vec3::new(4, 0, 0), ZVec3::new(0.05, 0.5, 0.5)));
        let open = stored_light(&grid, &blocks, WorldPos::new(I64Vec3::new(4, 0, 0), ZVec3::new(0.5, 0.5, 0.5)));
        assert!((by_wall.w - open.w).abs() < 0.01, "{by_wall} by the wall, {open} a little out");
    }

    #[test]
    fn light_changes_smoothly_between_boxels() {
        let (mut grid, blocks) = hut();
        // A doorway in the east wall, two high.
        grid.set_block(api::BoxelPos::new(3, 0, 0), sim::BlockId::EMPTY);
        grid.set_block(api::BoxelPos::new(3, 0, 1), sim::BlockId::EMPTY);
        grid.relight_all(&blocks);
        // Walking out through it is a smooth climb into the sky's light, never a jump.
        let sky = |x: f32| stored_light(&grid, &blocks, WorldPos::new(I64Vec3::ZERO, ZVec3::new(x, 0.5, 0.5))).w;
        let (first, last) = (sky(0.5), sky(6.5));
        assert!(last - first > 0.2, "the walk should climb: {first} to {last}");
        let steps = 120;
        for i in 0..steps {
            let (a, b) = (0.5 + 6.0 * i as f32 / steps as f32, 0.5 + 6.0 * (i + 1) as f32 / steps as f32);
            let jump = (sky(b) - sky(a)).abs();
            assert!(jump < 0.05, "sky light jumped from {} to {} between x {a} and {b}", sky(a), sky(b));
        }
    }

    #[test]
    fn eased_light_comes_most_of_the_way_in_the_ease_time_and_not_at_once() {
        let (dark, lit) = (Vec4::ZERO, Vec4::ONE);
        let one_frame = ease_light(dark, lit, 1.0 / 60.0);
        assert!(one_frame.x > 0.0 && one_frame.x < 0.4, "one frame in: {one_frame}");
        let mut now = dark;
        for _ in 0..(LIGHT_EASE * 60.0).round() as usize {
            now = ease_light(now, lit, 1.0 / 60.0);
        }
        assert!(now.x > 0.9 && now.x < 1.0, "after the ease time: {now}");
    }
}
