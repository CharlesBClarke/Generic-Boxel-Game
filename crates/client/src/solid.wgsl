// Solid looks: each fragment takes the texel of its block's look volume that it sits in. The
// mesher has already moved every face's sample position half a texel inside the block, so a
// face shows the outermost layer it cuts through. Then light and air (see `client::shading`):
// the lines along the quad's creases and ridges, the sun and sky, a sheen if the look has one,
// and fog toward the sky. The sun is shadowed by Bevy's shadow maps near the camera, and past
// them by a coarse shadow stepped across a height map of the terrain (see `client::shadows`).

#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world}
#import bevy_pbr::mesh_view_bindings::{view, lights}
#import bevy_pbr::mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT
#import bevy_pbr::shadows::{fetch_directional_shadow, cascade_debug_visualization}
#import bevy_pbr::view_transformations::position_world_to_clip
#import boxel::atmosphere::{Atmosphere, sky, fog_amount, ambient_light, sun_light, sheen, edge_shade, sky_reach, block_light}
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
// Each volume's sheen and glow, one texel per volume: strength, sharpness, glow.
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var sheens: texture_2d<f32>;
// The top of each column of boxels around the camera (see `client::heights`), from the height
// in `atmosphere.heights.z`.
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var heights: texture_2d<f32>;

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
    // Stored light: red, green, blue and sky, 0 to 1.
    @location(6) light: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) solid: vec4<f32>,
    @location(2) world_position: vec3<f32>,
    @location(3) world_normal: vec3<f32>,
    @location(4) @interpolate(flat) edges: vec4<u32>,
    @location(5) face: vec4<f32>,
    @location(6) light: vec4<f32>,
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
    out.light = vertex.light;
    return out;
}

// How much of the sun reaches `position` past the shadow maps: a ray toward the sun, in steps
// that spread out with distance, across the height map. Soft at the edges, more so farther
// from what casts the shadow.
fn far_shadow(position: vec3<f32>, normal: vec3<f32>) -> f32 {
    let to_sun = atmosphere.sun_direction.xyz;
    // The height map's terms: east, north, up.
    let dir = vec3<f32>(to_sun.x, -to_sun.z, to_sun.y);
    if dir.z <= 0.0 {
        return 1.0;
    }
    let h = atmosphere.heights;
    let scale = h.w;
    let size = vec2<i32>(textureDimensions(heights));
    let start = vec3<f32>(position.x, -position.z, position.y) + vec3<f32>(normal.x, -normal.z, normal.y) * 0.05;
    // Start past the point's own column, so its own top doesn't shade it.
    let flat = length(dir.xy);
    let reach = atmosphere.far.y;
    if flat < 1e-4 || 1.5 * scale / flat >= reach {
        return 1.0;
    }
    let t0 = 1.5 * scale / flat;
    let steps = max(i32(atmosphere.far.x), 1);
    let soft = atmosphere.far.z * 0.01;
    let bias = atmosphere.far.w;
    var lit = 1.0;
    for (var i = 0; i < steps; i++) {
        let s = f32(i) / f32(max(steps - 1, 1));
        let t = t0 + (reach - t0) * s * s;
        let q = start + dir * t;
        let texel = vec2<i32>(floor((q.xy - h.xy) / scale));
        if any(texel < vec2<i32>(0)) || any(texel >= size) {
            break;
        }
        let top = textureLoad(heights, texel, 0).r + h.z;
        let above = q.z + bias - top;
        lit = min(lit, clamp(0.5 + above / (t * soft + 0.001), 0.0, 1.0));
        if lit <= 0.0 {
            break;
        }
    }
    return lit;
}

// How far along the view `position` is from the camera, in meters (Bevy's view Z, negated).
fn view_depth(position: vec3<f32>) -> f32 {
    return -dot(vec4<f32>(view.view_from_world[0].z, view.view_from_world[1].z, view.view_from_world[2].z, view.view_from_world[3].z), vec4<f32>(position, 1.0));
}

// Whether the sun's shadow maps are being drawn.
fn has_maps() -> bool {
    return lights.n_directional_lights > 0u && (lights.directional_lights[0].flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u;
}

// How much of the sun reaches `position`: the shadow maps near the camera, fading into the far
// shadow, which also needs stored sky light (`reach`) since it can't see caves.
fn sun_shadow(position: vec3<f32>, normal: vec3<f32>, reach: f32) -> f32 {
    let depth = view_depth(position);
    let maps = has_maps();
    let fade = select(1.0, smoothstep(atmosphere.shadow.y, atmosphere.shadow.x, depth), maps);
    var near = 1.0;
    if maps && fade < 1.0 {
        near = fetch_directional_shadow(0u, vec4<f32>(position, 1.0), normal, -depth);
    }
    var far = reach;
    if fade > 0.0 && atmosphere.shadow.z > 0.5 {
        far *= far_shadow(position, normal);
    }
    return mix(near, far, fade);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = clamp(vec3<i32>(floor(in.solid.xyz)), vec3<i32>(0), vec3<i32>(SIZE - 1));
    let volume = i32(round(in.solid.w));
    let own = textureLoad(volumes, texel + vec3<i32>(0, 0, volume * SIZE), 0) * tint;
    let base = own * in.color;
    let shine = textureLoad(sheens, vec2<i32>(volume, 0), 0);
    let normal = normalize(in.world_normal);
    let to_eye_far = view.world_position - in.world_position;
    let distance = length(to_eye_far);
    let to_eye = to_eye_far / max(distance, 0.0001);

    // Each part can be switched off, to measure what it costs.
    let parts = atmosphere.parts;
    var rgb = base.rgb;
    // The sky's light reaches a face only as far as stored sky light does: none in a cave. The
    // sun's only as far as its shadows let it.
    let reach = sky_reach(atmosphere, in.light.a);
    var sun = 0.0;
    if parts.w > 0.5 || parts.z > 0.5 {
        sun = sun_shadow(in.world_position, normal, reach);
    }
    if parts.w > 0.5 {
        rgb *= ambient_light(atmosphere, normal) * reach + sun_light(atmosphere, normal) * sun + block_light(atmosphere, in.light.rgb);
    }
    if parts.x > 0.5 {
        rgb *= edge_shade(atmosphere, in.edges, in.face);
    }
    // A glowing look lights itself, without shade: its own colors, times its glow.
    let glow = shine.z;
    rgb = rgb * (1.0 - min(glow, 1.0)) + own.rgb * glow;
    if parts.z > 0.5 {
        rgb += sheen(atmosphere, normal, to_eye, shine.x, shine.y) * tint.rgb * sun;
    }
    if atmosphere.shadow.w > 0.5 && has_maps() {
        rgb = cascade_debug_visualization(rgb, 0u, -view_depth(in.world_position));
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
