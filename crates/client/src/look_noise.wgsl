#define_import_path boxel::look_noise

// The noise looks generate from the world texel position: the same functions as `api::look`,
// which they must match bit for bit (the client's GPU test checks). Positions are u32 world
// texels with `api::look::WORLD_OFFSET` added; scales are in sixteenths of a texel.

fn rotl(x: u32, r: u32) -> u32 {
    return (x << r) | (x >> (32u - r));
}

// A well-mixed number for a lattice point.
fn hash(p: vec3<u32>, seed: u32) -> u32 {
    var h = (seed * 0x9e3779b9u) ^ (p.x * 0x85ebca6bu);
    h ^= rotl(p.y * 0xc2b2ae35u, 11u);
    h ^= rotl(p.z * 0x27d4eb2fu, 22u);
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    return h ^ (h >> 16u);
}

// A hash as a number from 0 to just below 1.
fn unit(h: u32) -> f32 {
    return f32(h >> 8u) / 16777216.0;
}

fn octave(scale: vec3<u32>, seed: u32, p: vec3<u32>) -> f32 {
    // The texel's center in lattice cells, `16p + 8` over `scale`, worked in pieces so it
    // never overflows.
    let whole = p / scale;
    let inner = 16u * (p % scale) + 8u;
    let base = 16u * whole + inner / scale;
    let f = vec3<f32>(inner % scale) / vec3<f32>(scale);
    let t = f * f * (vec3<f32>(3.0) - 2.0 * f);
    var sum = 0.0;
    for (var corner = 0u; corner < 8u; corner++) {
        let d = vec3<u32>(corner & 1u, (corner >> 1u) & 1u, (corner >> 2u) & 1u);
        let w = select(vec3<f32>(1.0) - t, t, d == vec3<u32>(1u));
        sum += w.x * w.y * w.z * unit(hash(base + d, seed));
    }
    return sum;
}

// Smooth value noise, 0 to 1: two octaves, the finer half the scale but no finer than a texel.
fn value(scale: vec3<u32>, seed: u32, p: vec3<u32>) -> f32 {
    return value_fading(scale, seed, p, 0.0);
}

// The same with the finer octave faded toward its average by `fine_fade` (0 to 1), as it
// shrinks on screen; past 1 it isn't worked out at all.
fn value_fading(scale: vec3<u32>, seed: u32, p: vec3<u32>, fine_fade: f32) -> f32 {
    var fine = 0.5;
    if fine_fade < 1.0 {
        fine = mix(octave(max(scale / 2u, vec3<u32>(16u)), seed + 1u, p), 0.5, fine_fade);
    }
    return 0.65 * octave(scale, seed, p) + 0.35 * fine;
}

// How faded each octave of noise at `scale` is (coarse, fine), 0 to 1, with `footprint`
// texels to a pixel: an octave fades as its blobs shrink from `start` to `end` of a pixel
// (both in blobs per pixel), so detail goes the finest first.
fn octave_fades(scale: vec3<u32>, footprint: f32, start: f32, end: f32) -> vec2<f32> {
    let finest = f32(min(scale.x, min(scale.y, scale.z))) / 16.0;
    let cells = vec2<f32>(finest, max(finest / 2.0, 1.0));
    return smoothstep(vec2<f32>(start), vec2<f32>(end), vec2<f32>(footprint) / cells);
}

// Which of `count` colors a noise value picks: stretched first, as smooth noise bunches
// around the middle.
fn pick(value: f32, count: u32) -> u32 {
    let stretched = clamp((value - 0.5) * 1.8 + 0.5, 0.0, 0.9999);
    return u32(stretched * f32(count));
}

// How much grain shades texel `p` running `direction` (1 X, 2 Y, 3 up) on `board`: around 1.
// `scale` is along the board, across it and through it.
fn grain_factor(scale: vec3<u32>, seed: u32, strength: f32, direction: u32, board: u32, p: vec3<u32>, fine_fade: f32) -> f32 {
    var s = vec3<u32>(scale.y, scale.z, scale.x);
    if direction == 1u {
        s = scale;
    } else if direction == 2u {
        s = vec3<u32>(scale.y, scale.x, scale.z);
    }
    let v = value_fading(s, seed + board * 7919u, p, fine_fade);
    return 1.0 + strength * (2.0 * v - 1.0);
}
