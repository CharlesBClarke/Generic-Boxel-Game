//! The material every block is drawn with: each fragment colored from its block's look (see
//! `solid.wgsl`): the look's face slices, their mip levels farther away, and the detail it
//! generates from the world position. Times the vertex color, ambient occlusion and a tint,
//! then lit and
//! fogged from the atmosphere (see `crate::shading`). The sun's shadow is in `sun.wgsl`, which
//! models share (see `crate::model`).

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path};
use bevy::image::ImageSampler;
use bevy::mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension,
    TextureFormat, VertexFormat,
};
use bevy::render::storage::ShaderStorageBuffer;
use bevy::shader::ShaderDefVal;
use bevy::shader::ShaderRef;

use api::LookDef;

use crate::shading::Atmosphere;
use crate::volumes::Volumes;

/// Where in its look a vertex sits, in texels, and which volume: `MeshData::solid`.
pub const ATTRIBUTE_SOLID: MeshVertexAttribute = MeshVertexAttribute::new("Solid", 517_093_611, VertexFormat::Float32x4);
/// What lies past each edge of the vertex's quad: `MeshData::edges`.
pub const ATTRIBUTE_EDGES: MeshVertexAttribute = MeshVertexAttribute::new("Edges", 517_093_612, VertexFormat::Uint32x4);
/// Where on its quad a vertex sits, and the quad's size: `MeshData::face`.
pub const ATTRIBUTE_FACE: MeshVertexAttribute = MeshVertexAttribute::new("Face", 517_093_613, VertexFormat::Unorm16x4);
/// The stored light at a vertex, red, green, blue and sky (see [`crate::mesher::MeshData::light`]).
pub const ATTRIBUTE_LIGHT: MeshVertexAttribute = MeshVertexAttribute::new("Light", 517_093_614, VertexFormat::Unorm8x4);
/// How full the space in front of the vertex's quad is around its corners (see
/// [`crate::mesher::MeshData::ao`]).
pub const ATTRIBUTE_AO: MeshVertexAttribute = MeshVertexAttribute::new("Ao", 517_093_615, VertexFormat::Uint32x4);

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct SolidMaterial {
    /// Multiplied with everything, alpha included: white for blocks, see-through for the ghost.
    #[uniform(0)]
    pub tint: LinearRgba,
    /// The looks' face slices with their mip levels: [`atlas_image`].
    #[texture(1, sample_type = "float", filterable = false)]
    pub looks: Handle<Image>,
    /// Kept up to date by `crate::shading`.
    #[uniform(2)]
    pub atmosphere: Atmosphere,
    /// Each volume's sheen and glow: see [`sheen_image`].
    #[texture(3, sample_type = "float", filterable = false)]
    pub sheens: Handle<Image>,
    /// The height map the far shadow steps across (see `crate::heights`), kept up to date by
    /// `crate::shading`.
    #[texture(4, sample_type = "float", filterable = false)]
    pub heights: Handle<Image>,
    /// Stored light added to every vertex's (red, green, blue, sky, 0 to 1): a loose item's,
    /// as its mesh carries none (see `crate::items`). Zero for the world's meshes.
    #[uniform(5)]
    pub light: Vec4,
    /// Where generated detail is read from: the render origin's cell in texels, as the noise
    /// takes positions (see [`api::look::world_texel`]), then 1 for meshes drawn where they
    /// stand in the world (chunks, the ghost) or 0 for ones that aren't (loose items), which
    /// read it as if at the world's middle. Kept up to date by `crate::shading`.
    #[uniform(6)]
    pub place: UVec4,
    /// What each look generates: [`look_table`].
    #[storage(7, read_only)]
    pub generated: Handle<ShaderStorageBuffer>,
    pub alpha_mode: AlphaMode,
}

/// `place` for meshes away from the world.
pub const AWAY: UVec4 = UVec4::new(api::look::WORLD_OFFSET, api::look::WORLD_OFFSET, api::look::WORLD_OFFSET, 0);

/// `place` for meshes in the world, with the render origin at `origin`.
pub fn in_world(origin: api::I64Vec3) -> UVec4 {
    let [x, y, z] = api::look::world_texel((origin * LookDef::SIZE as i64).to_array());
    UVec4::new(x, y, z, 1)
}

/// One noise layer as the shader reads it. Matches `Noise` in `solid.wgsl`.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct NoiseGpu {
    /// Linear RGB.
    pub colors: [Vec4; api::look::MAX_COLORS],
    pub average: Vec4,
    /// In sixteenths of a texel (see [`api::look::scale_units`]), then how many colors.
    pub scale: UVec4,
    /// The seed, the rest unused.
    pub seed: UVec4,
}

/// What one look generates, as the shader reads it. Matches `Generated` in `solid.wgsl`.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct LookGpu {
    pub noise: [NoiseGpu; api::look::MAX_LAYERS],
    /// Along, across and through the board, in sixteenths of a texel, then the seed.
    pub grain_scale: UVec4,
    /// The grain's strength, the rest unused.
    pub grain: Vec4,
}

/// Every volume's generated detail, by volume.
pub fn look_table(volumes: &Volumes) -> Vec<LookGpu> {
    use api::look::{scale_units, srgb_to_linear};
    volumes
        .details
        .iter()
        .map(|d| {
            let mut look = LookGpu::default();
            for (gpu, layer) in look.noise.iter_mut().zip(&d.noise) {
                for (c, rgb) in gpu.colors.iter_mut().zip(&layer.colors) {
                    *c = Vec4::new(srgb_to_linear(rgb[0]), srgb_to_linear(rgb[1]), srgb_to_linear(rgb[2]), 1.0);
                }
                gpu.average = Vec3::from_array(layer.average()).extend(1.0);
                let [x, y, z] = layer.scale.map(scale_units);
                gpu.scale = UVec4::new(x, y, z, layer.colors.len() as u32);
                gpu.seed = UVec4::new(layer.seed, 0, 0, 0);
            }
            if let Some(g) = &d.grain {
                let [x, y, z] = g.scale.map(scale_units);
                look.grain_scale = UVec4::new(x, y, z, g.seed);
                look.grain = Vec4::new(g.strength, 0.0, 0.0, 0.0);
            }
            look
        })
        .collect()
}

/// [`look_table`] as a storage buffer.
pub fn look_buffer(volumes: &Volumes) -> ShaderStorageBuffer {
    ShaderStorageBuffer::from(look_table(volumes))
}

impl Material for SolidMaterial {
    fn vertex_shader() -> ShaderRef {
        shader()
    }

    fn fragment_shader() -> ShaderRef {
        shader()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // The shadow pass draws with Bevy's own prepass shader, which needs only positions
        // (and colors, which it is given), as Bevy laid them out.
        if descriptor.vertex.shader_defs.iter().any(|d| matches!(d, ShaderDefVal::Bool(name, true) if name == "PREPASS_PIPELINE")) {
            return Ok(());
        }
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(1),
            ATTRIBUTE_SOLID.at_shader_location(2),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(3),
            ATTRIBUTE_EDGES.at_shader_location(4),
            ATTRIBUTE_FACE.at_shader_location(5),
            ATTRIBUTE_LIGHT.at_shader_location(6),
            ATTRIBUTE_AO.at_shader_location(7),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        Ok(())
    }
}

/// The shader, built into the binary by [`SolidPlugin`].
fn shader() -> ShaderRef {
    ShaderRef::Path(AssetPath::from_path_buf(embedded_path!("solid.wgsl")).with_source("embedded"))
}

/// Registers the material and its shader.
pub struct SolidPlugin;

impl Plugin for SolidPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "sun.wgsl");
        bevy::shader::load_shader_library!(app, "look_noise.wgsl");
        embedded_asset!(app, "solid.wgsl");
        app.add_plugins(MaterialPlugin::<SolidMaterial>::default());
    }
}

/// The looks' face slices as one 2D image with its mip levels (see [`Volumes::atlas`]), read
/// texel by texel: the shader picks the levels and blends between them itself.
pub fn atlas_image(volumes: &Volumes) -> Image {
    let [width, height] = volumes.atlas_size();
    let mut image = Image::new_uninit(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(volumes.atlas());
    image.texture_descriptor.mip_level_count = crate::volumes::LEVELS;
    image.sampler = ImageSampler::nearest();
    image
}

/// Each volume's sheen and glow as one texel: strength, sharpness, glow.
pub fn sheen_image(volumes: &Volumes) -> Image {
    let data: Vec<u8> = volumes.sheens.iter().flat_map(|&[s, k, g]| [s, k, g, 0.0]).flat_map(f32::to_le_bytes).collect();
    let mut image = Image::new(
        Extent3d { width: volumes.sheens.len() as u32, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba32Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}
