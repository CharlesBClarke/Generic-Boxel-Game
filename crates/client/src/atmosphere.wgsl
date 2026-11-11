// Light and air, shared by the block shader and the sky: see `client::shading`. Directions are
// in render space (Y up), colors linear.
#define_import_path boxel::atmosphere

// Matches `client::shading::Atmosphere`.
struct Atmosphere {
    // Toward the sun; then the cosine of the sun disc's radius.
    sun_direction: vec4<f32>,
    // The sun's light on a face turned to it (zero once it's down); then the glow's strength.
    sun_color: vec4<f32>,
    // The sun's hue at full brightness, for its disc and glow.
    sun_hue: vec4<f32>,
    // Toward the moon; then the cosine of its disc's radius.
    moon_direction: vec4<f32>,
    // The moon's light on a face turned to it; then how much it shows, 0 by day, 1 by night.
    moon_color: vec4<f32>,
    // Light on faces looking straight up, and straight down, blended between.
    sky_light: vec4<f32>,
    ground_light: vec4<f32>,
    zenith: vec4<f32>,
    horizon: vec4<f32>,
    // How bright the stars are, and how far they've turned (radians).
    night: vec4<f32>,
    // Starting distance, distance where it's thickest, how thick there.
    fog: vec4<f32>,
    // Crease darkness, ridge brightness, width in texels.
    edges: vec4<f32>,
    // Which parts are drawn, 1 or 0: edges, fog, sheen, light.
    parts: vec4<f32>,
    // Stored light: block light's strength, sky light's falloff power, block light's, and the
    // least light.
    stored: vec4<f32>,
    // Stored light on (1) or off (0).
    stored_on: vec4<f32>,
    // Shadows: how far the maps reach, where they start fading into the far shadow (along the
    // view), the far shadow on (1) or off (0), maps tinted by cascade (1) or not.
    shadow: vec4<f32>,
    // The far shadow: most height-map reads toward the sun, how far it looks, how soft, how far
    // a column must rise above a ray to shade it.
    far: vec4<f32>,
    // The coarse height map, relative to the render origin: X and north of texel (0, 0)'s
    // corner, the height its texels are measured from, columns per texel.
    heights: vec4<f32>,
    // The same for the fine height map, a texel to each column.
    fine_heights: vec4<f32>,
};

// The sky looking along `dir` (a unit vector): the horizon's color blending up into the
// zenith's, and a glow toward the sun lowest down. From the stylised-sky block (Eanpa Sky's
// sky, MIT). Fog fades to it, so far terrain melts into the sky behind it.
fn sky(a: Atmosphere, dir: vec3<f32>) -> vec3<f32> {
    let up = pow(clamp(dir.y * 1.35 + 0.02, 0.0, 1.0), 0.82);
    var color = mix(a.horizon.rgb, a.zenith.rgb, up);
    let toward = max(dot(dir, a.sun_direction.xyz), 0.0);
    color += a.sun_hue.rgb * pow(toward, 6.0) * 0.35 * (1.0 - up) * a.sun_color.w;
    return color;
}

// A small hash of a cell, 0 to 1.
fn hash3(p: vec3<f32>) -> f32 {
    var h = fract(p * vec3<f32>(0.1031, 0.103, 0.0973));
    h += dot(h, h.yzx + 33.33);
    return fract((h.x + h.y) * h.z);
}

// The sun, moon and stars over `color` (the sky looking along `dir`), as the stylised-sky
// block draws them: a small hot disc with a halo, a moon lit from the sun's side, and a star at
// a random point of some cells of a grid that turns once a day.
fn sky_bodies(a: Atmosphere, dir: vec3<f32>, color: vec3<f32>) -> vec3<f32> {
    var out = color;
    let night = a.moon_color.w;

    // How much of this pixel the moon covers: it hides the stars behind it.
    let m = a.moon_direction.xyz;
    let cos_a = dot(dir, m);
    let edge = a.moon_direction.w;
    var moon_cover = 0.0;
    if m.y > -0.05 {
        let px = max(fwidth(cos_a), 1e-7);
        moon_cover = smoothstep(edge - px, edge + px, cos_a);
    }

    // Stars, only where the sky is dark enough to show them.
    if a.night.x > 0.002 && moon_cover < 1.0 {
        let turn = a.night.y;
        let d = vec3<f32>(dir.x * cos(turn) - dir.z * sin(turn), dir.y, dir.x * sin(turn) + dir.z * cos(turn)) * 190.0;
        let f = fract(d) - 0.5;
        let bright = smoothstep(0.93, 1.0, hash3(floor(d))) * smoothstep(0.42, 0.0, length(f));
        out += vec3<f32>(0.85, 0.92, 1.0) * bright * bright * a.night.x * 1.8 * smoothstep(0.0, 0.12, dir.y) * (1.0 - moon_cover);
    }

    // The moon: a disc, lit on the side toward the sun, with a faint, broad shading for seas.
    if moon_cover > 0.0 {
        let radius = sqrt(max(1.0 - edge * edge, 1e-8));
        let right = normalize(vec3<f32>(m.z, 0.0, -m.x) + vec3<f32>(1e-5, 0.0, 0.0));
        let up = cross(right, m);
        let lx = dot(dir, right) / radius;
        let ly = dot(dir, up) / radius;
        let nz = sqrt(max(1.0 - lx * lx - ly * ly, 0.0));
        let n = right * lx + up * ly + m * nz;
        let lit = max(dot(n, a.sun_direction.xyz), 0.04);
        let seas = 0.5 + 0.5 * sin(lx * 2.1 + 1.3) * sin(ly * 1.7 - 0.4);
        let moon = vec3<f32>(0.82, 0.86, 0.95) * mix(0.85, 1.05, seas) * lit * mix(0.5, 1.15, night);
        // The sky's own color shows a little through it by day, never the stars.
        out = mix(out, moon + color * 0.15, moon_cover);
    }

    // The sun: a crisp disc and a tight halo, then a wider glow.
    let s = a.sun_direction.xyz;
    let mu = dot(dir, s);
    let sun_edge = a.sun_direction.w;
    let sun_px = max(fwidth(mu), 1e-7);
    let disc = smoothstep(sun_edge - sun_px, sun_edge + sun_px, mu);
    let glow = a.sun_color.w;
    let toward = max(mu, 0.0);
    out += a.sun_hue.rgb * (disc * 48.0 + (pow(toward, 900.0) * 3.0 + pow(toward, 60.0) * 0.22) * glow);
    return out;
}

// A tone-mapped color with Bevy's screen-space dither added, as its own materials do: a dark
// gradient (the night sky) spans only a few of the screen's 256 steps, and without the noise
// those show as bands. `frag` is the pixel's position on screen.
fn dithered(color: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    let x = (vec3<f32>(dot(vec2<f32>(171.0, 231.0), frag)) / vec3<f32>(103.0, 71.0, 97.0));
    // Dithered in the screen's gamma, then back: the output texture is sRGB. A channel within
    // half a step of black gets less noise, so it's never pushed below black and clipped:
    // clipping only ever lifts, which would grey a dark blue sky.
    let gamma = pow(max(color, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    let amount = min(gamma * 510.0, vec3<f32>(1.0));
    let dither = (fract(x) - 0.5) / 255.0 * amount;
    return pow(max(gamma + dither, vec3<f32>(0.0)), vec3<f32>(2.2));
}

// How much fog there is at `distance` meters, from 0 to the fog's thickness.
fn fog_amount(a: Atmosphere, distance: f32) -> f32 {
    let t = clamp((distance - a.fog.x) / max(a.fog.y - a.fog.x, 0.001), 0.0, 1.0);
    return a.fog.z * t * t * (3.0 - 2.0 * t);
}

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

// Light from the sky and ground on a surface facing `normal` (unit): the sky's above, the
// ground's below, blended between.
fn ambient_light(a: Atmosphere, normal: vec3<f32>) -> vec3<f32> {
    return mix(a.ground_light.rgb, a.sky_light.rgb, normal.y * 0.5 + 0.5);
}

// The sun's light on a surface facing `normal` (unit), before any shadow.
fn sun_light(a: Atmosphere, normal: vec3<f32>) -> vec3<f32> {
    return a.sun_color.rgb * max(dot(normal, a.sun_direction.xyz), 0.0);
}

// The moon's light on a surface facing `normal` (unit). It casts no shadows.
fn moon_light(a: Atmosphere, normal: vec3<f32>) -> vec3<f32> {
    return a.moon_color.rgb * max(dot(normal, a.moon_direction.xyz), 0.0);
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
fn edge_factor(a: Atmosphere, kinds: u32, along: f32, length: f32, distance: f32) -> f32 {
    // A merged quad's edge holds one boxel's pattern, repeated along it (see `client::greedy`).
    let texel = u32(clamp(floor(along), 0.0, max(length - 1.0, 0.0))) % 16u;
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
    return edge_factor(a, kinds.x, u, w, v)
        * edge_factor(a, kinds.y, v, h, w - u)
        * edge_factor(a, kinds.z, w - u, w, h - v)
        * edge_factor(a, kinds.w, h - v, h, u);
}
