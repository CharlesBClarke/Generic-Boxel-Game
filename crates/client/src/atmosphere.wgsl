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
};

// The sky looking along `dir` (a unit vector), with the sun's disc drawn or not. Fog fades to
// it without the disc, so far terrain melts into the sky behind it.
fn sky(a: Atmosphere, dir: vec3<f32>, disc: bool) -> vec3<f32> {
    let up = clamp(dir.y, 0.0, 1.0);
    var color = mix(a.horizon.rgb, a.zenith.rgb, sqrt(up));
    let toward = max(dot(dir, a.sun_direction.xyz), 0.0);
    let sun = a.sun_color.rgb / max(max(a.sun_color.r, a.sun_color.g), max(a.sun_color.b, 0.001));
    color += sun * a.sun_color.w * (0.25 * pow(toward, 8.0) + 0.75 * pow(toward, 256.0));
    if disc {
        let edge = a.sun_direction.w;
        let soft = (1.0 - edge) * 0.15;
        color = mix(color, sun * 2.0, smoothstep(edge - soft, edge + soft, toward));
    }
    return color;
}

// How much fog there is at `distance` meters, from 0 to the fog's thickness.
fn fog_amount(a: Atmosphere, distance: f32) -> f32 {
    let t = clamp((distance - a.fog.x) / max(a.fog.y - a.fog.x, 0.001), 0.0, 1.0);
    return a.fog.z * t * t * (3.0 - 2.0 * t);
}

// Light falling on a surface facing `normal` (unit): the sky's above, the ground's below,
// and the sun.
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
