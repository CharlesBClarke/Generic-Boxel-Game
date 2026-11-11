// Models: cows, the held club, chunks from a hit. Each fragment takes its texture's color times
// a plain color, then is lit like the world's blocks (see `solid.wgsl`): the light from
// `sun.wgsl`, given the stored light where the model stands (`client::model::stored_light`),
// then fog toward the sky. A flash washes it toward its own colors made brighter, after the
// light, so it reads the same at noon or midnight.

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::shadows::cascade_debug_visualization
#import boxel::sun::{atmosphere, lit, has_maps, view_depth}
#import boxel::atmosphere::{dithered, sky, fog_amount}
#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping::tone_mapping
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> color: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var texture_sampler: sampler;
// Red, green, blue and sky, 0 to 1.
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var<uniform> light: vec4<f32>;
// How much brighter in each channel, then how far toward that.
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var<uniform> flash: vec4<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var own = color;
#ifdef VERTEX_UVS_A
    own *= textureSample(texture, texture_sampler, in.uv);
#endif
    let position = in.world_position.xyz;
    let normal = normalize(in.world_normal);
    let to_eye_far = view.world_position - position;
    let distance = length(to_eye_far);
    let to_eye = to_eye_far / max(distance, 0.0001);

    let parts = atmosphere.parts;
    var rgb = own.rgb;
    if parts.w > 0.5 {
        rgb *= lit(position, normal, light).light;
    }
    rgb = mix(rgb, min(own.rgb * flash.rgb, vec3<f32>(1.0)), flash.a);
    if atmosphere.shadow.w > 0.5 && has_maps() {
        rgb = cascade_debug_visualization(rgb, 0u, -view_depth(position));
    }
    if parts.y > 0.5 {
        rgb = mix(rgb, sky(atmosphere, -to_eye), fog_amount(atmosphere, distance));
    }

    var out = vec4<f32>(rgb, own.a);
#ifdef TONEMAP_IN_SHADER
    out = tone_mapping(out, view.color_grading);
#ifdef DEBAND_DITHER
    out = vec4<f32>(dithered(out.rgb, in.position.xy), out.a);
#endif
#endif
    return out;
}
