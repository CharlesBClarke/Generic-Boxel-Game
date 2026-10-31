//! The material every block is drawn with: unlit, each fragment colored from its block's look
//! volume (see `solid.wgsl`), times the vertex color and a tint.

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path};
use bevy::image::ImageSampler;
use bevy::mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError, TextureDimension, TextureFormat,
    VertexFormat,
};
use bevy::shader::ShaderRef;

use api::LookDef;

use crate::volumes::Volumes;

/// Where in its look a vertex sits, in texels, and which volume: `MeshData::solid`.
pub const ATTRIBUTE_SOLID: MeshVertexAttribute = MeshVertexAttribute::new("Solid", 517_093_611, VertexFormat::Float32x4);

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct SolidMaterial {
    /// Multiplied with everything, alpha included: white for blocks, see-through for the ghost.
    #[uniform(0)]
    pub tint: LinearRgba,
    #[texture(1, dimension = "3d", sample_type = "float", filterable = false)]
    pub volumes: Handle<Image>,
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
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(1),
            ATTRIBUTE_SOLID.at_shader_location(2),
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
