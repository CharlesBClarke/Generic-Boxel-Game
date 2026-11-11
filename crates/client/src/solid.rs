//! The material every block is drawn with: each fragment colored from its block's look volume
//! (see `solid.wgsl`), times the vertex color, ambient occlusion and a tint, then lit and
//! fogged from the atmosphere (see `crate::shading`). The sun's shadow is in `sun.wgsl`, which
//! models share (see `crate::model`).

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path};
use bevy::image::ImageSampler;
use bevy::mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError, TextureDimension, TextureFormat,
    VertexFormat,
};
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
    #[texture(1, dimension = "3d", sample_type = "float", filterable = false)]
    pub volumes: Handle<Image>,
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
    pub alpha_mode: AlphaMode,
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
        embedded_asset!(app, "solid.wgsl");
        app.add_plugins(MaterialPlugin::<SolidMaterial>::default());
    }
}

/// The volumes as one 3D image, read texel by texel.
pub fn volume_image(volumes: &Volumes) -> Image {
    let size = LookDef::SIZE as u32;
    let mut image = Image::new(
        Extent3d { width: size, height: size, depth_or_array_layers: size * volumes.count },
        TextureDimension::D3,
        volumes.rgba.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    // The shader loads exact texels; nothing is ever filtered or mipmapped.
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
