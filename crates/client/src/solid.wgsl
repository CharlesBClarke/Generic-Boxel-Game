// Solid looks: each fragment takes the texel of its block's look volume that it sits in. The
// mesher has already moved every face's sample position half a texel inside the block, so a
// face shows the outermost layer it cuts through. Ambient occlusion is worked out here from how
// full the space in front of the quad is around its corners (see `client::mesher::ao_lit`,
// which this matches). Then light and air (see `client::shading`):
// the lines along the quad's creases and ridges, the sun and sky, a sheen if the look has one,
// and fog toward the sky. The sun is shadowed by Bevy's shadow maps near the camera, and past
// them by a coarse shadow stepped across a height map of the terrain (see `client::shadows`).

#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world}
#import bevy_pbr::mesh_view_bindings::{view, lights}
#import bevy_pbr::mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT
#import bevy_pbr::shadows::{fetch_directional_shadow, cascade_debug_visualization}
#import bevy_pbr::view_transformations::position_world_to_clip
#import boxel::atmosphere::{Atmosphere, dithered, sky, fog_amount, ambient_light, sun_light, moon_light, sheen, edge_shade, sky_reach, block_light}
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

// Where height map `m` lies (0 fine, 1 coarse): see `atmosphere.fine_heights`.
fn map_place(m: i32) -> vec4<f32> {
    if m == 0 {
        return atmosphere.fine_heights;
    }
    return atmosphere.heights;
}

// Texels along each side of one map at `level`; the two sit side by side, fine on the left.
fn map_size(level: i32) -> i32 {
    return i32(textureDimensions(heights, level).y);
}

// The highest top in texel `cell` of map `m` at `level` of its pyramid of maximums.
fn highest(m: i32, cell: vec2<i32>, level: i32) -> f32 {
    return textureLoad(heights, cell + vec2<i32>(m * map_size(level), 0), level).r + map_place(m).z;
}

// Where `p` (east, north) falls in map `m`, in its texels at level 0.
fn map_texels(m: i32, p: vec2<f32>) -> vec2<f32> {
    let h = map_place(m);
    return (p - h.xy) / h.w;
}

fn in_map(uv: vec2<f32>) -> bool {
    return all(uv >= vec2<f32>(0.0)) && all(uv < vec2<f32>(f32(map_size(0))));
}

// How lit a ray point at distance `t` is by a column `top` high: soft at the edges, more so
// farther from what casts the shadow.
fn column_light(q: vec3<f32>, t: f32, top: f32) -> f32 {
    let above = q.z + atmosphere.far.w - top;
    return clamp(0.5 + above / (t * atmosphere.far.z * 0.01 + 0.001), 0.0, 1.0);
}

// How much of the sun reaches `position` past the shadow maps: a ray toward the sun across the
// height maps, fine first, then coarse. It walks each map's pyramid of maximums: where the ray
// clears the highest thing in a big texel it skips that whole texel and tries a bigger one;
// where it doesn't, it looks closer, down to single columns. The point's own column is skipped,
// so its own top doesn't shade it.
fn far_shadow(position: vec3<f32>, normal: vec3<f32>) -> f32 {
    let to_sun = atmosphere.sun_direction.xyz;
    // The height map's terms: east, north, up.
    let dir = vec3<f32>(to_sun.x, -to_sun.z, to_sun.y);
    if dir.z <= 0.0 || length(dir.xy) < 1e-4 {
        return 1.0;
    }
    let start = vec3<f32>(position.x, -position.z, position.y) + vec3<f32>(normal.x, -normal.z, normal.y) * 0.05;
    var m = 0;
    if !in_map(map_texels(0, start.xy)) {
        m = 1;
        if !in_map(map_texels(1, start.xy)) {
            return 1.0;
        }
    }
    let own = vec3<i32>(vec2<i32>(floor(map_texels(m, start.xy))), m);
    let top_level = i32(textureNumLevels(heights)) - 1;
    let reach = atmosphere.far.y;
    let reads = i32(atmosphere.far.x);
    var lit = 1.0;
    var t = 0.0;
    var level = 0;
    for (var i = 0; i < reads; i++) {
        let q = start + dir * t;
        let uv = map_texels(m, q.xy);
        if !in_map(uv) {
            if m == 1 || !in_map(map_texels(1, q.xy)) {
                return lit;
            }
            // Off the fine map: on across the coarse one.
            m = 1;
            level = 0;
            continue;
        }
        // The texel at this level, in level-0 texels, and where the ray leaves it.
        let span = f32(1u << u32(level));
        let cell = floor(uv / span);
        let lo = cell * span;
        let d = dir.xy / map_place(m).w;
        let to_edge = select(uv - lo, lo + span - uv, d >= vec2<f32>(0.0)) / max(abs(d), vec2<f32>(1e-6));
        let exit = t + min(to_edge.x, to_edge.y) + 1e-3;
        // The ray is rising, so it's lowest where it enters.
        let light = column_light(q, t, highest(m, vec2<i32>(cell), level));
        if light >= 1.0 {
            t = exit;
            level = min(level + 1, top_level);
        } else if level > 0 {
            level -= 1;
        } else {
            if !(m == own.z && all(vec2<i32>(cell) == own.xy)) {
                lit = min(lit, light);
                if lit <= 0.0 {
                    return 0.0;
                }
            }
            t = exit;
        }
        if t >= reach {
            return lit;
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
    // The sky's light reaches a face only as far as stored sky light does: none in a cave. The
    // sun's only as far as its shadows let it.
    // The moon's light, which casts no shadows, comes in as far as the sky's does.
    let reach = sky_reach(atmosphere, in.light.a);
    var sun = 0.0;
    // Once the sun is down its light is zero, and its shadows aren't looked up at all.
    let sun_up = max(atmosphere.sun_color.r, max(atmosphere.sun_color.g, atmosphere.sun_color.b)) > 0.0;
    if (parts.w > 0.5 || parts.z > 0.5) && sun_up {
        sun = sun_shadow(in.world_position, normal, reach);
    }
    if parts.w > 0.5 {
        rgb *= ambient_light(atmosphere, normal) * reach
            + sun_light(atmosphere, normal) * sun
            + moon_light(atmosphere, normal) * reach
            + block_light(atmosphere, in.light.rgb);
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
