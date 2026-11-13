// The sun's shadow, shared by everything lit like the world: blocks (`solid.wgsl`) and models
// (`model.wgsl`). Shadow maps near the camera, past them a coarse shadow stepped across a height
// map of the terrain (see `client::shadows`). A material that imports this puts its atmosphere
// at binding 2 and the height map at binding 4.
#define_import_path boxel::sun

#import bevy_pbr::mesh_view_bindings::{view, lights}
#import bevy_pbr::mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT
#import bevy_pbr::shadows::fetch_directional_shadow
#import boxel::atmosphere::{Atmosphere, ambient_light, sun_light, moon_light, sky_reach, block_light}

@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> atmosphere: Atmosphere;
// The top of each column of boxels around the camera (see `client::heights`), from the height
// in `atmosphere.heights.z`.
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var heights: texture_2d<f32>;

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

// The light falling on a surface, and how much of the sun reaches it (for a sheen).
struct Lit {
    light: vec3<f32>,
    sun: f32,
}

// How much of the sun reaches a surface at `position` facing `normal`, given its stored
// light: only as far as its shadows let it. Once the sun is down its light is zero, and its
// shadows aren't looked up at all.
fn sun_reach(position: vec3<f32>, normal: vec3<f32>, stored: vec4<f32>) -> f32 {
    let parts = atmosphere.parts;
    let sun_up = max(atmosphere.sun_color.r, max(atmosphere.sun_color.g, atmosphere.sun_color.b)) > 0.0;
    if (parts.w > 0.5 || parts.z > 0.5) && sun_up {
        return sun_shadow(position, normal, sky_reach(atmosphere, stored.a));
    }
    return 0.0;
}

// The light on a surface facing `normal` (normalized), from the sun, sky, moon and blocks,
// given the stored light there (red, green, blue and sky, 0 to 1) and how much of the sun
// reaches it (see `sun_reach`). The sky's light reaches a surface only as far as stored sky
// light does: none in a cave. The moon's light, which casts no shadows, comes in as far as the
// sky's does. The light is white when that part is switched off (see `atmosphere.parts`).
fn light_facing(normal: vec3<f32>, stored: vec4<f32>, sun: f32) -> vec3<f32> {
    if atmosphere.parts.w <= 0.5 {
        return vec3<f32>(1.0);
    }
    let reach = sky_reach(atmosphere, stored.a);
    return ambient_light(atmosphere, normal) * reach
        + sun_light(atmosphere, normal) * sun
        + moon_light(atmosphere, normal) * reach
        + block_light(atmosphere, stored.rgb);
}

// The light on a surface at `position` facing `normal` (normalized), given the stored light
// there: see `light_facing` and `sun_reach`.
fn lit(position: vec3<f32>, normal: vec3<f32>, stored: vec4<f32>) -> Lit {
    var out: Lit;
    out.sun = sun_reach(position, normal, stored);
    out.light = light_facing(normal, stored, out.sun);
    return out;
}
