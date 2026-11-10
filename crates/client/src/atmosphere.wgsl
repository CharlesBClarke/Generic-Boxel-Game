// Light and air, shared by the block shader and the sky: see `client::shading`. Directions are
// in render space (Y up), colors linear.
#define_import_path boxel::atmosphere

// Matches `client::shading::Atmosphere`.
struct Atmosphere {
    // Toward the sun; then the cosine of the sun disc's radius.
    sun_direction: vec4<f32>,
    // The sun's light on a face turned to it; then the glow around its disc.
    sun_color: vec4<f32>,
    // Light on faces looking straight up, and straight down, blended between.
    sky_light: vec4<f32>,
    ground_light: vec4<f32>,
    zenith: vec4<f32>,
    horizon: vec4<f32>,
    // Starting distance, distance where it's thickest, how thick there.
    fog: vec4<f32>,
    // Crease darkness, ridge brightness, width in texels.
    edges: vec4<f32>,
    // Which parts are drawn, 1 or 0: edges, fog, sheen, light.
    parts: vec4<f32>,
    // The sun's rays: how many, how far past the disc (in disc radii), how wide at the disc (a
    // share of the gap between rays, up to 0.45), how strong.
    rays: vec4<f32>,
    // How much the rays' lengths vary, how much they fade toward their tips, how fast they turn
    // (turns a minute), and the disc's radius in radians.
    ray_shape: vec4<f32>,
    // Stored light: block light's strength, sky light's falloff power, block light's, and the
    // least light.
    stored: vec4<f32>,
    // Stored light on (1) or off (0).
    stored_on: vec4<f32>,
};

// The sun's color at full brightness, whatever its strength.
fn sun_hue(a: Atmosphere) -> vec3<f32> {
    return a.sun_color.rgb / max(max(a.sun_color.r, a.sun_color.g), max(a.sun_color.b, 0.001));
}

// The sky looking along `dir` (a unit vector), with its glow around the sun but not the sun
// itself. Fog fades to it, so far terrain melts into the sky behind it.
fn sky(a: Atmosphere, dir: vec3<f32>) -> vec3<f32> {
    let up = clamp(dir.y, 0.0, 1.0);
    let color = mix(a.horizon.rgb, a.zenith.rgb, sqrt(up));
    let toward = max(dot(dir, a.sun_direction.xyz), 0.0);
    return color + sun_hue(a) * a.sun_color.w * (0.25 * pow(toward, 8.0) + 0.75 * pow(toward, 256.0));
}

// A small hash of a ray's number, 0 to 1.
fn ray_hash(i: f32) -> f32 {
    return fract(sin(i * 12.9898 + 4.1414) * 43758.5453);
}

// The sun drawn over `color` (the sky looking along `dir`): a crisp disc and flat, pointed rays
// out from it, like a drawn sun. `time` is in seconds, for turning the rays.
fn sun(a: Atmosphere, dir: vec3<f32>, color: vec3<f32>, time: f32) -> vec3<f32> {
    let s = a.sun_direction.xyz;
    // How far from the sun's middle, in radians; one pixel's worth of that, for soft edges.
    let away = atan2(length(cross(dir, s)), dot(dir, s));
    let pixel = max(fwidth(away), 1e-6);
    let radius = a.ray_shape.w;
    let hue = sun_hue(a) * 2.0;
    var out = color;

    let count = round(a.rays.x);
    let reach = a.rays.y * radius;
    if count >= 1.0 && reach > 0.0 && a.rays.w > 0.0 && away > radius - pixel && away < radius + reach + pixel {
        // Which way around the sun, from a side direction across it.
        var side = cross(s, vec3<f32>(0.0, 1.0, 0.0));
        if length(side) < 1e-3 {
            side = vec3<f32>(1.0, 0.0, 0.0);
        }
        side = normalize(side);
        let other = cross(side, s);
        let turns = a.ray_shape.z * time / 60.0;
        let around = (atan2(dot(dir, other), dot(dir, side)) / 6.2831853 + turns) * count;
        let i = floor(around);
        let ray = (i % count + count) % count;
        let off = abs(fract(around) - 0.5);
        // This ray's length, then how far along it this pixel is: 0 at the disc, 1 at the tip.
        let span = reach * (1.0 - a.ray_shape.x * ray_hash(ray));
        let t = clamp((away - radius) / max(span, 1e-6), 0.0, 1.0);
        // Across the ray, in radians: positive inside its wedge.
        let width = min(a.rays.z, 0.45) * (1.0 - t);
        let inside = (width - off) / count * 6.2831853 * sin(away);
        let along = span + radius - away;
        let cover = smoothstep(-pixel, pixel, inside) * smoothstep(-pixel, pixel, along);
        let fade = 1.0 - a.ray_shape.y * t;
        out = mix(out, hue, clamp(cover * fade * a.rays.w, 0.0, 1.0));
    }

    return mix(out, hue, smoothstep(-pixel, pixel, radius - away));
}

// How much fog there is at `distance` meters, from 0 to the fog's thickness.
fn fog_amount(a: Atmosphere, distance: f32) -> f32 {
    let t = clamp((distance - a.fog.x) / max(a.fog.y - a.fog.x, 0.001), 0.0, 1.0);
    return a.fog.z * t * t * (3.0 - 2.0 * t);
}

// Light falling on a surface facing `normal` (unit): the sky's above, the ground's below,
// and the sun.
// How much of the sun and sky reach a face, from its stored sky light (0 to 1): 1 in the open,
// less where they come in only sideways, through an opening.
fn sky_reach(a: Atmosphere, stored_sky: f32) -> f32 {
    if a.stored_on.x < 0.5 {
        return 1.0;
    }
    return pow(stored_sky, a.stored.y);
}

// Light from blocks that give light, from the face's stored red, green and blue (0 to 1), plus
// the least light any face gets.
fn block_light(a: Atmosphere, stored_rgb: vec3<f32>) -> vec3<f32> {
    if a.stored_on.x < 0.5 {
        return vec3<f32>(0.0);
    }
    return pow(stored_rgb, vec3<f32>(a.stored.z)) * a.stored.x + vec3<f32>(a.stored.w);
}

fn surface_light(a: Atmosphere, normal: vec3<f32>) -> vec3<f32> {
    let ambient = mix(a.ground_light.rgb, a.sky_light.rgb, normal.y * 0.5 + 0.5);
    return ambient + a.sun_color.rgb * max(dot(normal, a.sun_direction.xyz), 0.0);
}

// The sun's reflection toward the eye (Blinn-Phong), for a look with a sheen.
fn sheen(a: Atmosphere, normal: vec3<f32>, to_eye: vec3<f32>, strength: f32, sharpness: f32) -> vec3<f32> {
    let to_sun = a.sun_direction.xyz;
    if strength <= 0.0 || dot(normal, to_sun) <= 0.0 {
        return vec3<f32>(0.0);
    }
    let half_way = normalize(to_sun + to_eye);
    return a.sun_color.rgb * strength * pow(max(dot(normal, half_way), 0.0), sharpness);
}

// How one edge shades a texel `along` texels down it and `distance` texels in from it: each
// texel one flat value, so the line stays crisp and blocky. Kinds as `client::mesher`: 0 flat,
// 1 crease, 2 ridge.
fn edge_factor(a: Atmosphere, kinds: u32, along: f32, distance: f32) -> f32 {
    let texel = u32(clamp(floor(along), 0.0, 15.0));
    let kind = (kinds >> (2u * texel)) & 3u;
    let row = floor(max(distance, 0.0));
    let width = a.edges.z;
    if kind == 0u || row >= width {
        return 1.0;
    }
    let weight = 1.0 - row / width;
    if kind == 1u {
        return 1.0 - a.edges.x * weight;
    }
    return 1.0 + a.edges.y * weight;
}

// All four edges of a quad on a texel: `face` is where on the quad, in texels from corner 0
// along edge 0 and then edge 3 backwards, and the quad's size along those.
fn edge_shade(a: Atmosphere, kinds: vec4<u32>, face: vec4<f32>) -> f32 {
    let u = face.x;
    let v = face.y;
    let w = face.z;
    let h = face.w;
    return edge_factor(a, kinds.x, u, v)
        * edge_factor(a, kinds.y, v, w - u)
        * edge_factor(a, kinds.z, w - u, h - v)
        * edge_factor(a, kinds.w, h - v, u);
}
