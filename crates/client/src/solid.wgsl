// Solid looks: each fragment takes the texel of its block's look volume that it sits in. The
// mesher has already moved every face's sample position half a texel inside the block, so a
// face shows the outermost layer it cuts through.

#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_clip}
#import bevy_pbr::mesh_view_bindings::view
#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping::tone_mapping
#endif

// Texels along each side of a look; matches `api::LookDef::SIZE`.
const SIZE: i32 = 16;

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> tint: vec4<f32>;
// Every look stacked along Z, one SIZE-deep volume after another.
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var volumes: texture_3d<f32>;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    // Where in the look, in texels, and which volume.
    @location(2) solid: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) solid: vec4<f32>,
};

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = get_world_from_local(vertex.instance_index);
    out.clip_position = mesh_position_local_to_clip(world_from_local, vec4<f32>(vertex.position, 1.0));
    out.color = vertex.color;
    out.solid = vertex.solid;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = clamp(vec3<i32>(floor(in.solid.xyz)), vec3<i32>(0), vec3<i32>(SIZE - 1));
    let volume = i32(round(in.solid.w));
    var color = textureLoad(volumes, texel + vec3<i32>(0, 0, volume * SIZE), 0) * in.color * tint;
#ifdef TONEMAP_IN_SHADER
    color = tone_mapping(color, view.color_grading);
#endif
    return color;
}
