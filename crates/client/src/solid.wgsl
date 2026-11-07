// Solid looks: each fragment takes the texel of its block's look volume that it sits in. The
// mesher has already moved every face's sample position half a texel inside the block, so a
// face shows the outermost layer it cuts through. Then light and air (see `client::shading`):
// the lines along the quad's creases and ridges, the sun and sky, a sheen if the look has one,
// and fog toward the sky.

#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world}
#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::view_transformations::position_world_to_clip
#import boxel::atmosphere::{Atmosphere, sky, fog_amount, surface_light, sheen, edge_shade}
#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping::tone_mapping
#endif

// Texels along each side of a look; matches `api::LookDef::SIZE`.
const SIZE: i32 = 16;
// Texels a face's position counts up to; matches `client::mesher::FACE_SCALE`.
const FACE_SCALE: f32 = 16.0;

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> tint: vec4<f32>;
// Every look stacked along Z, one SIZE-deep volume after another.
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var volumes: texture_3d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> atmosphere: Atmosphere;
// Each volume's sheen, one texel per volume: strength, sharpness.
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var sheens: texture_2d<f32>;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    // Where in the look, in texels, and which volume.
    @location(2) solid: vec4<f32>,
    @location(3) normal: vec3<f32>,
    // What's past each of the quad's edges, texel by texel.
    @location(4) edges: vec4<u32>,
    // Where on the quad, and its size, over FACE_SCALE.
    @location(5) face: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) solid: vec4<f32>,
    @location(2) world_position: vec3<f32>,
    @location(3) world_normal: vec3<f32>,
    @location(4) @interpolate(flat) edges: vec4<u32>,
    @location(5) face: vec4<f32>,
};

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = get_world_from_local(vertex.instance_index);
    let world = mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    out.clip_position = position_world_to_clip(world.xyz);
    out.world_position = world.xyz;
    out.world_normal = mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
    out.color = vertex.color;
    out.solid = vertex.solid;
    out.edges = vertex.edges;
    out.face = vertex.face * FACE_SCALE;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = clamp(vec3<i32>(floor(in.solid.xyz)), vec3<i32>(0), vec3<i32>(SIZE - 1));
    let volume = i32(round(in.solid.w));
    let base = textureLoad(volumes, texel + vec3<i32>(0, 0, volume * SIZE), 0) * in.color * tint;
    let normal = normalize(in.world_normal);
    let to_eye_far = view.world_position - in.world_position;
    let distance = length(to_eye_far);
    let to_eye = to_eye_far / max(distance, 0.0001);

    // Each part can be switched off, to measure what it costs.
    let parts = atmosphere.parts;
    var rgb = base.rgb;
    if parts.w > 0.5 {
        rgb *= surface_light(atmosphere, normal);
    }
    if parts.x > 0.5 {
        rgb *= edge_shade(atmosphere, in.edges, in.face);
    }
    if parts.z > 0.5 {
        let shine = textureLoad(sheens, vec2<i32>(volume, 0), 0);
        rgb += sheen(atmosphere, normal, to_eye, shine.x, shine.y) * tint.rgb;
    }
    if parts.y > 0.5 {
        rgb = mix(rgb, sky(atmosphere, -to_eye), fog_amount(atmosphere, distance));
    }

    var color = vec4<f32>(rgb, base.a);
#ifdef TONEMAP_IN_SHADER
    color = tone_mapping(color, view.color_grading);
#endif
    return color;
}
