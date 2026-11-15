//! The material every block is drawn with: each fragment colored from its block's look (see
//! `solid.wgsl`): the face slices of the look's variant its boxel shows, their mip levels
//! farther away. Times the vertex color, ambient occlusion and a tint, then lit and fogged
//! from the atmosphere (see `crate::shading`). The sun's shadow is in `sun.wgsl`, which
//! models share (see `crate::model`).

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path};
use bevy::image::ImageSampler;
use bevy::mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError, TextureDimension,
    TextureFormat, VertexFormat,
};
use bevy::shader::ShaderDefVal;
use bevy::shader::ShaderRef;

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
#[bind_group_data(SolidKey)]
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
    /// Which variants a mesh's boxels show: the render origin's cell (as `u32`s, wrapping),
    /// then 1 for meshes drawn where they stand in the world (chunks, the ghost), whose boxels
    /// each show the variant picked for where they are (see [`crate::volumes::variant_of`]),
    /// 0 for ones that aren't (loose items), or 2 for far terrain (see
    /// [`crate::far::FAR_PLACE`]); both of those show the first variant. Kept up to date by
    /// `crate::shading`.
    #[uniform(6)]
    pub place: UVec4,
    /// For far terrain (see [`crate::far`]): how much of the darkness ambient occlusion puts
    /// on a staircase's steps its faces take (see [`crate::view::ViewTuning::far_crease`]),
    /// then where the render origin's cell is in its chunk, in boxels (see
    /// [`crate::far::mask_offset`]). Zero for everything else.
    #[uniform(8)]
    pub far: Vec4,
    /// For far terrain: which ordinary chunks around the render origin are drawn, where far
    /// terrain is left out (see [`crate::far::mask_image`]). An empty one for everything else.
    #[texture(7, dimension = "3d", sample_type = "float", filterable = false)]
    pub mask: Handle<Image>,
    pub alpha_mode: AlphaMode,
}

/// What of a material picks its pipeline: far terrain's shader leaves fragments out (see
/// `far_hidden` in `solid.wgsl`), which the others' never does, so theirs keeps its early
/// depth test.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SolidKey {
    far: bool,
}

impl From<&SolidMaterial> for SolidKey {
    fn from(material: &SolidMaterial) -> Self {
        SolidKey { far: material.place.w == crate::far::FAR_PLACE.w }
    }
}

/// `place` for meshes away from the world.
pub const AWAY: UVec4 = UVec4::new(0, 0, 0, 0);

/// `place` for meshes in the world, with the render origin at `origin`.
pub fn in_world(origin: api::I64Vec3) -> UVec4 {
    UVec4::new(origin.x as u32, origin.y as u32, origin.z as u32, 1)
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
        key: MaterialPipelineKey<Self>,
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
        if key.bind_group_data.far
            && let Some(fragment) = &mut descriptor.fragment
        {
            fragment.shader_defs.push("FAR_TERRAIN".into());
        }
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
