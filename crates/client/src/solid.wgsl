// Solid looks: each fragment takes the texel of its block's look volume that it sits in. The
// mesher has already moved every face's sample position half a texel inside the block, so a
// face shows the outermost layer it cuts through. Ambient occlusion is worked out here from how
// full the space in front of the quad is around its corners (see `client::mesher::ao_lit`,
// which this matches). Then light and air (see `client::shading`):
// the lines along the quad's creases and ridges, the sun and sky, a sheen if the look has one,
// and fog toward the sky. The sun's shadow is `sun.wgsl`'s, shared with models.

#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world}
#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::shadows::cascade_debug_visualization
#import bevy_pbr::view_transformations::position_world_to_clip
#import boxel::sun::{atmosphere, lit, has_maps, view_depth}
#import boxel::atmosphere::{dithered, sky, fog_amount, sheen, edge_shade}
#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping::tone_mapping
#endif

// Texels along each side of a look; matches `api::LookDef::SIZE`.
const SIZE: i32 = 16;
// Texels a face's position counts up to; matches `client::mesher::FACE_SCALE`.
const FACE_SCALE: f32 = 512.0;

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> tint: vec4<f32>;
// Every look stacked along Z, one SIZE-deep volume after another.
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var volumes: texture_3d<f32>;
// Each volume's sheen and glow, one texel per volume: strength, sharpness, glow.
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var sheens: texture_2d<f32>;
// Stored light added to every vertex's, 0 to 1: a loose item's, as its mesh carries none.
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var<uniform> light: vec4<f32>;

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
    // How full the space in front of the quad is around each corner: four bytes a corner.
    @location(7) ao: vec4<u32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    // Centroid: with MSAA a pixel on a quad's edge is shaded where it's covered, so the look
    // position stays on the quad and its wrap (see `fragment`) never reaches the next boxel's.
    @location(1) @interpolate(perspective, centroid) solid: vec4<f32>,
    @location(2) world_position: vec3<f32>,
    @location(3) world_normal: vec3<f32>,
    @location(4) @interpolate(flat) edges: vec4<u32>,
    @location(5) face: vec4<f32>,
    @location(6) light: vec4<f32>,
    @location(7) @interpolate(flat) ao: vec4<u32>,
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
    out.ao = vertex.ao;
    return out;
}

// Cube `j` of corner `k`'s word (see `client::mesher::CUBES`), 0 to 1.
fn cube(ao: vec4<u32>, k: i32, j: u32) -> f32 {
    return f32((ao[k] >> (8u * j)) & 0xffu) / 255.0;
}

// How full the space in front of a quad is at lattice point `p` of its `n` cells: the four
// cubes meeting there, each from the cell it lies over. See `client::mesher::lattice_fill`.
fn lattice_fill(ao: vec4<u32>, n: vec2<i32>, p: vec2<i32>) -> f32 {
    var sum = 0.0;
    for (var i = 0; i < 4; i++) {
        let c = p - vec2<i32>(1 - (i & 1), 1 - (i >> 1));
        let in_x = c.x >= 0 && c.x < n.x;
        let in_y = c.y >= 0 && c.y < n.y;
        if in_x && in_y {
            // Which corner of its cell the point is: 0 at its lowest, then round.
            var k = 2;
            if c.x == p.x && c.y == p.y { k = 0; } else if c.y == p.y { k = 1; } else if c.x == p.x { k = 3; }
            sum += cube(ao, k, 0u);
        } else if in_x && c.y < 0 {
            sum += select(cube(ao, 1, 2u), cube(ao, 0, 1u), c.x == p.x);
        } else if in_x {
            sum += select(cube(ao, 3, 2u), cube(ao, 2, 1u), c.x == p.x - 1);
        } else if in_y && c.x >= n.x {
            sum += select(cube(ao, 2, 2u), cube(ao, 1, 1u), c.y == p.y);
        } else if in_y {
            sum += select(cube(ao, 0, 2u), cube(ao, 3, 1u), c.y == p.y - 1);
        } else {
            var k = 2;
            if c.x < 0 && c.y < 0 { k = 0; } else if c.y < 0 { k = 1; } else if c.x < 0 { k = 3; }
            sum += cube(ao, k, 3u);
        }
    }
    return sum / 4.0;
}

// How lit the fragment at `face` (texels from corner 0, then the quad's size) is by ambient
// occlusion: the darkness at its cell's corners, blended across the cell.
fn ao_lit(ao: vec4<u32>, face: vec4<f32>) -> f32 {
    let size = f32(SIZE);
    let n = max(vec2<i32>(floor(face.zw / size + 0.5)), vec2<i32>(1));
    let cell = face.zw / vec2<f32>(n);
    let at = clamp(floor(face.xy / cell), vec2<f32>(0.0), vec2<f32>(n - 1));
    let f = clamp(face.xy / cell - at, vec2<f32>(0.0), vec2<f32>(1.0));
    let p = vec2<i32>(at);
    let a = atmosphere.ao;
    let fills = vec4<f32>(
        lattice_fill(ao, n, p),
        lattice_fill(ao, n, p + vec2<i32>(1, 0)),
        lattice_fill(ao, n, p + vec2<i32>(0, 1)),
        lattice_fill(ao, n, p + vec2<i32>(1, 1)),
    );
    let lit = vec4<f32>(1.0) - a.x * pow(clamp(fills / a.y, vec4<f32>(0.0), vec4<f32>(1.0)), vec4<f32>(a.z));
    return mix(mix(lit.x, lit.y, f.x), mix(lit.z, lit.w, f.x), f.y);
}


@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // A merged quad's look position runs on across its boxels (see `client::greedy`): wrapped,
    // the look repeats every boxel.
    let texel = ((vec3<i32>(floor(in.solid.xyz)) % SIZE) + SIZE) % SIZE;
    let volume = i32(round(in.solid.w));
    let own = textureLoad(volumes, texel + vec3<i32>(0, 0, volume * SIZE), 0) * tint;
    var base = own * in.color;
    if atmosphere.ao.w > 0.5 && any(in.ao != vec4<u32>(0u)) {
        base = vec4<f32>(base.rgb * ao_lit(in.ao, in.face), base.a);
    }
    let shine = textureLoad(sheens, vec2<i32>(volume, 0), 0);
    let normal = normalize(in.world_normal);
    let to_eye_far = view.world_position - in.world_position;
    let distance = length(to_eye_far);
    let to_eye = to_eye_far / max(distance, 0.0001);

    // Each part can be switched off, to measure what it costs.
    let parts = atmosphere.parts;
    var rgb = base.rgb;
    // An item's own stored light comes in through `light`; the world's meshes carry theirs.
    let l = lit(in.world_position, normal, min(in.light + light, vec4<f32>(1.0)));
    let sun = l.sun;
    if parts.w > 0.5 {
        rgb *= l.light;
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
#ifdef DEBAND_DITHER
    color = vec4<f32>(dithered(color.rgb, in.clip_position.xy), color.a);
#endif
#endif
    return color;
}
