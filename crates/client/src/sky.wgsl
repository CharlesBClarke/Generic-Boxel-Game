// The sky: one triangle over the whole screen at the far plane, colored by which way each
// pixel looks (see `client::shading`).

#import bevy_pbr::mesh_view_bindings::view
#import boxel::atmosphere::{Atmosphere, sky}
#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping::tone_mapping
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> atmosphere: Atmosphere;

struct Vertex {
    // Already in clip space.
    @location(0) position: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    // Depth 0 is the far plane: depth runs backwards, near is 1.
    out.clip_position = vec4<f32>(vertex.position.xy, 0.0, 1.0);
    out.ndc = vertex.position.xy;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // A point on the near plane through this pixel, then the way from the eye to it.
    let near = view.world_from_clip * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(near.xyz / near.w - view.world_position);
    var color = vec4<f32>(sky(atmosphere, dir, true), 1.0);
#ifdef TONEMAP_IN_SHADER
    color = tone_mapping(color, view.color_grading);
#endif
    return color;
}
