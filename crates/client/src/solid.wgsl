// Solid looks: each fragment takes the texel of its block's look that it sits in. The mesher
// has already moved every face's sample position half a texel inside the block, so a face
// shows the outermost layer it cuts through. Texels are read from the look's face slices (see
// `client::volumes::Volumes::atlas`): exact up close, blended between mip levels as they
// shrink on screen, with a few reads along faces seen at a glance (anisotropic filtering).
// Each boxel in the world shows one of its look's variants, picked by a hash of where it is,
// so a field of blocks doesn't repeat every block (see `variant_of`). Ambient occlusion is worked out here from how
// full the space in front of the quad is around its corners (see `client::mesher::ao_lit`,
// which this matches). Then light and air (see `client::shading`):
// the lines along the quad's creases and ridges, the sun and sky, a sheen if the look has one,
// and fog toward the sky. The sun's shadow is `sun.wgsl`'s, shared with models.

#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world}
#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::shadows::cascade_debug_visualization
#import bevy_pbr::view_transformations::position_world_to_clip
#import boxel::sun::{atmosphere, lit, sun_reach, light_facing, has_maps, view_depth}
#import boxel::atmosphere::{dithered, sky, fog_amount, sheen, edge_shade}
#ifdef TONEMAP_IN_SHADER
#import bevy_core_pipeline::tonemapping::tone_mapping
#endif

// Texels along each side of a look; matches `api::LookDef::SIZE`.
const SIZE: i32 = 16;
// Texels a face's position counts up to; matches `client::mesher::FACE_SCALE`.
const FACE_SCALE: f32 = 512.0;

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> tint: vec4<f32>;
// Every look's face slices, with mip levels: see `client::volumes::Volumes::atlas`.
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var looks: texture_2d<f32>;
// Each volume's sheen and glow, one texel per volume: strength, sharpness, glow.
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var sheens: texture_2d<f32>;
// Stored light added to every vertex's, 0 to 1: a loose item's, as its mesh carries none.
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var<uniform> light: vec4<f32>;
// The render origin's cell (wrapping), then 1 for meshes where they stand in the world, 0 for
// loose items or 2 for far terrain: see `client::solid::SolidMaterial::place`.
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var<uniform> place: vec4<u32>;
// Far terrain: how much of the darkness ambient occlusion puts on a staircase's steps its
// faces take, then where the render origin's cell is in its chunk, in boxels: see
// `client::solid::SolidMaterial::far`.
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var<uniform> far_terrain: vec4<f32>;
// Far terrain: which ordinary chunks around the render origin's are drawn, a texel a chunk,
// the origin's chunk in the middle: see `client::far::mask_image`.
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var far_mask: texture_3d<f32>;

// Chunks along each side of the mask; matches `client::far::MASK_SIDE`.
const FAR_MASK_SIDE: i32 = 64;
// Boxels along each side of a chunk; matches `api::CHUNK_SIZE`.
const CHUNK: f32 = 32.0;

// What a far face's vertex alpha packs its second material's volume by: `client::far::MIX_PACK`.
const FAR_MIX_PACK: f32 = 32.0;

// Meters far terrain looks its shadow up higher: a level-1 cell (see `client::far`).
const FAR_SHADOW_LIFT: f32 = 2.0;

// Mip levels of the face slices; matches `client::volumes::LEVELS`.
const LEVELS: i32 = 5;

// Variants each volume holds in the atlas; matches `client::volumes::VARIANTS`.
const VARIANTS: i32 = 16;

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
    // The rest that change across a quad are centroid too: shaded at a pixel's middle, off the
    // triangle, they'd run past its corners' values, and stored light below 0 turns to NaN in
    // `pow` (black and colored specks where a lamp's light ends).
    @location(2) @interpolate(perspective, centroid) world_position: vec3<f32>,
    @location(3) world_normal: vec3<f32>,
    @location(4) @interpolate(flat) edges: vec4<u32>,
    @location(5) @interpolate(perspective, centroid) face: vec4<f32>,
    @location(6) @interpolate(perspective, centroid) light: vec4<f32>,
    @location(7) @interpolate(flat) ao: vec4<u32>,
    // The look position again, not centroid, for how it changes across the screen: centroid
    // points jump about within pixels on a triangle's edge, so their differences there are
    // wrong, picked the wrong slice or mip level and drew dashes along quads' diagonals.
    @location(8) solid_smooth: vec3<f32>,
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
    out.solid_smooth = vertex.solid.xyz;
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


// Texel `texel` of `volume`'s variant `variant`, from its slice facing along `axis`, at mip
// level `level`.
fn slice_texel(volume: i32, variant: i32, axis: i32, texel: vec3<i32>, level: i32) -> vec4<f32> {
    let pixel = vec2<i32>(
        (variant * SIZE + texel[axis]) * SIZE + texel[(axis + 1) % 3],
        (volume * 3 + axis) * SIZE + texel[(axis + 2) % 3],
    );
    return textureLoad(looks, pixel >> vec2<u32>(u32(level)), level);
}

// `volume`'s variant's texel `texel` (wrapped into the look) blended between mip levels at
// `lod`.
fn mip_texel(volume: i32, variant: i32, axis: i32, texel: vec3<i32>, lod: f32) -> vec3<f32> {
    let low = i32(floor(lod));
    let high = min(low + 1, LEVELS - 1);
    let a = slice_texel(volume, variant, axis, texel, low).rgb;
    let b = slice_texel(volume, variant, axis, texel, high).rgb;
    return mix(a, b, lod - f32(low));
}

fn wrap(cell: vec3<i32>) -> vec3<i32> {
    return ((cell % SIZE) + SIZE) % SIZE;
}

// Which of the first `n` variants the boxel at `cell` shows; matches
// `client::volumes::variant_of`.
fn variant_of(cell: vec3<u32>, n: u32) -> i32 {
    var h = (cell.x * 0x8da6b343u) ^ (cell.y * 0xd8163841u) ^ (cell.z * 0xcb1ab31fu);
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return i32(h % max(n, 1u));
}

// The look's color at a fragment: see the top of this file.
fn look_color(in: VertexOutput, volume: i32) -> vec3<f32> {
    let solid = in.solid.xyz;
    // Across a face the look position changes on its two axes but not along the normal.
    let dx = dpdx(in.solid_smooth);
    let dy = dpdy(in.solid_smooth);
    let change = abs(dx) + abs(dy);
    var axis = 0;
    if change.y < change.x && change.y <= change.z {
        axis = 1;
    } else if change.z < change.x && change.z < change.y {
        axis = 2;
    }
    let u = (axis + 1) % 3;
    let w = (axis + 2) % 3;
    // A pixel's footprint on the face, in texels: long across ground seen at a glance and
    // short the other way. Like anisotropic filtering, detail goes by the short way, with a
    // few reads spread along the long way so it doesn't shimmer.
    let along_x = vec2<f32>(dx[u], dx[w]);
    let along_y = vec2<f32>(dy[u], dy[w]);
    let lx = length(along_x);
    let ly = length(along_y);
    let longest = max(lx, ly);
    let flags = u32(atmosphere.looks.w);
    let mips_on = (flags & 2u) != 0u;
    let most = f32(max(flags >> 2u, 1u));
    let reads = clamp(ceil(longest / max(min(lx, ly), 1e-6)), 1.0, most);
    let footprint = longest / reads;

    // Which boxel a texel is in: a merged quad's look position runs on across its boxels
    // (see `client::greedy`) from the quad's first, whose corner is the fragment's position
    // less its place in the look, rounded (exact, as faces lie on texels). Meshes away from
    // the world show the first variant.
    let in_world = place.w == 1u;
    let n = u32(clamp(round(atmosphere.looks.x), 1.0, f32(VARIANTS)));
    var first = vec3<u32>(0u);
    if in_world {
        let at = vec3<f32>(in.world_position.x, -in.world_position.z, in.world_position.y);
        first = place.xyz + bitcast<vec3<u32>>(vec3<i32>(round(at - solid / f32(SIZE))));
    }
    // Wrapped, the look repeats every boxel.
    let cell = vec3<i32>(floor(solid));
    var variant = 0;
    if in_world {
        variant = variant_of(first + bitcast<vec3<u32>>(cell >> vec3<u32>(4u)), n);
    }
    if !mips_on || footprint <= 1.0 {
        return slice_texel(volume, variant, axis, wrap(cell), 0).rgb;
    }
    let lod = min(log2(footprint), f32(LEVELS - 1));
    let spread = select(along_y, along_x, lx >= ly);
    let count = i32(reads);
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < count; i++) {
        let o = spread * ((f32(i) + 0.5) / f32(count) - 0.5);
        var at = solid;
        at[u] += o.x;
        at[w] += o.y;
        // A read can land in the next boxel of a merged quad, with its own variant.
        let read = vec3<i32>(floor(at));
        var v = 0;
        if in_world {
            v = variant_of(first + bitcast<vec3<u32>>(read >> vec3<u32>(4u)), n);
        }
        sum += mip_texel(volume, v, axis, wrap(read), lod);
    }
    return sum / f32(count);
}

// A unit vector in render space (Y-up) along `axis` of the world's (Z-up: east, north, up),
// forward if `side` is 1 and backward if -1.
fn render_axis(axis: i32, side: f32) -> vec3<f32> {
    if axis == 0 {
        return vec3<f32>(side, 0.0, 0.0);
    }
    if axis == 1 {
        return vec3<f32>(0.0, 0.0, -side);
    }
    return vec3<f32>(0.0, side, 0.0);
}

// The slope a far face carries in its stored light's red and green (see
// `client::slope::pack`, which this undoes), in render space.
fn far_slope(packed: vec2<f32>) -> vec3<f32> {
    let o = (round(packed * 255.0) - vec2<f32>(128.0)) / 124.0;
    let z = 1.0 - abs(o.x) - abs(o.y);
    let fold = max(-z, 0.0);
    let x = o.x + select(fold, -fold, o.x >= 0.0);
    let y = o.y + select(fold, -fold, o.y >= 0.0);
    let n = normalize(vec3<f32>(x, y, z));
    return vec3<f32>(n.x, n.z, -n.y);
}

// Whether a far fragment at `position` (render space) on a face facing `normal` is in an
// ordinary chunk that's drawn: full detail is there, so far terrain is left out (a coarse far
// chunk still stands in around it while finer ones load). Judged just behind the face, so one
// on a chunk's border goes by the cell it's a face of.
fn far_hidden(position: vec3<f32>, normal: vec3<f32>) -> bool {
    let p = position - normal * 0.01;
    // The world's axes (east, north, up), in boxels from the corner of the origin's chunk.
    let at = vec3<f32>(p.x, -p.z, p.y) + far_terrain.yzw;
    let chunk = vec3<i32>(floor(at / CHUNK)) + vec3<i32>(FAR_MASK_SIDE / 2);
    if any(chunk < vec3<i32>(0)) || any(chunk >= vec3<i32>(FAR_MASK_SIDE)) {
        return false;
    }
    return textureLoad(far_mask, chunk, 0).r > 0.5;
}

// A far face's color: its look's alone, then lit; and how much of the sun reaches it.
struct Far {
    own: vec3<f32>,
    shaded: vec3<f32>,
    sun: f32,
}

// Far terrain (see `client::far`). At full detail the ground facing `slope` is a staircase of
// boxel faces, too small to tell apart from here, and what shows is their mix: each of the
// three ways the stairs face, by how much of the slope it takes up and how squarely the eye
// sees it. A far face is drawn as that mix (each way's average color and light, less what
// ambient occlusion takes off steps that size), so a coarse step's top and side come out the
// same and a hillside shades as one surface, not in stripes. `face` is the far face's own
// direction; `stored` its light. A cell of two materials is drawn as their mix: `second`'s
// colors make up `share_second` of it (see `client::far::set_mixes`).
fn far_shade(volume: i32, second: i32, share_second: f32, slope: vec3<f32>, face: vec3<f32>, to_eye: vec3<f32>, position: vec3<f32>, stored: vec4<f32>) -> Far {
    // The world's axes: east, north, up.
    let n = vec3<f32>(slope.x, -slope.z, slope.y);
    let eye = vec3<f32>(to_eye.x, -to_eye.z, to_eye.y);
    let own_way = abs(vec3<f32>(face.x, -face.z, face.y));
    let side = select(vec3<f32>(-1.0), vec3<f32>(1.0), n >= vec3<f32>(0.0));
    // The far face itself is in view, whatever the slope says: never nothing to show.
    var share = abs(n) * max(eye * side, vec3<f32>(0.0)) + own_way * 0.001;
    share /= share.x + share.y + share.z;
    // A step's crease darkens its boxel by half of what a floor against one wall gets at the
    // crease (see `ao_lit`), fading across it.
    let a = atmosphere.ao;
    var crease = 0.0;
    if a.w > 0.5 {
        crease = 0.5 * a.x * pow(clamp(0.5 / a.y, 0.0, 1.0), a.z) * far_terrain.x;
    }
    let total = abs(n.x) + abs(n.y) + abs(n.z);
    var out: Far;
    out.sun = sun_reach(position, slope, stored);
    out.own = vec3<f32>(0.0);
    out.shaded = vec3<f32>(0.0);
    for (var axis = 0; axis < 3; axis++) {
        if share[axis] <= 0.0 {
            continue;
        }
        // The look's outermost layer that way, averaged: one pixel at the deepest level, of
        // its first variant.
        var texel = vec3<i32>(0);
        texel[axis] = select(0, SIZE - 1, side[axis] > 0.0);
        var look = slice_texel(volume, 0, axis, texel, LEVELS - 1).rgb;
        if share_second > 0.0 {
            look = mix(look, slice_texel(second, 0, axis, texel, LEVELS - 1).rgb, share_second);
        }
        let color = look * share[axis];
        // How much of a face that way lies along a crease: all of it where it's the narrow
        // part of the stairs, less the wider it is.
        let creased = min((total - abs(n[axis])) / max(abs(n[axis]), 0.0001), 1.0);
        out.own += color;
        out.shaded += color * light_facing(render_axis(axis, side[axis]), stored, out.sun) * (1.0 - crease * creased);
    }
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let volume = i32(round(in.solid.w));
    let shine = textureLoad(sheens, vec2<i32>(volume, 0), 0);
    var normal = normalize(in.world_normal);
    let to_eye_far = view.world_position - in.world_position;
    let distance = length(to_eye_far);
    let to_eye = to_eye_far / max(distance, 0.0001);
    // Each part can be switched off, to measure what it costs.
    let parts = atmosphere.parts;
    var own: vec4<f32>;
    var base: vec4<f32>;
    var rgb: vec3<f32>;
    var sun: f32;
    if place.w == 2u {
#ifdef FAR_TERRAIN
        if far_hidden(in.world_position, normal) {
            discard;
        }
#endif
        // Far terrain: drawn as the slope it's part of (see `far_shade`), with no ambient
        // occlusion or edges, which are a boxel's, not a cell's. Its stored light holds only
        // the sky's; red and green carry the slope. It looks its shadow up a cell higher
        // (render space is Y-up): its coarse surface can sit up to a cell below the ground the
        // far shadow's height map holds, and would otherwise shade itself.
        let slope = far_slope(in.light.xy);
        let stored = clamp(vec4<f32>(0.0, 0.0, 0.0, in.light.a) + light, vec4<f32>(0.0), vec4<f32>(1.0));
        var shadow_at = in.world_position;
        shadow_at.y += FAR_SHADOW_LIFT;
        // The vertex color's alpha carries the cell's second material and its share; far
        // terrain is opaque.
        let packed = round(in.color.a);
        let second = i32(floor(packed / FAR_MIX_PACK));
        let share_second = (packed - f32(second) * FAR_MIX_PACK) / 16.0;
        let far = far_shade(volume, second, share_second, slope, normal, to_eye, shadow_at, stored);
        own = vec4<f32>(far.own, 1.0) * tint;
        base = own * vec4<f32>(in.color.rgb, 1.0);
        rgb = far.shaded * tint.rgb * in.color.rgb;
        sun = far.sun;
        // A sheen is the material's, so far terrain keeps it, off the slope.
        normal = slope;
    } else {
        own = vec4<f32>(look_color(in, volume), 1.0) * tint;
        base = own * in.color;
        if atmosphere.ao.w > 0.5 && any(in.ao != vec4<u32>(0u)) {
            base = vec4<f32>(base.rgb * ao_lit(in.ao, in.face), base.a);
        }
        rgb = base.rgb;
        // An item's own stored light comes in through `light`; the world's meshes carry theirs.
        let l = lit(in.world_position, normal, clamp(in.light + light, vec4<f32>(0.0), vec4<f32>(1.0)));
        sun = l.sun;
        if parts.w > 0.5 {
            rgb *= l.light;
        }
        if parts.x > 0.5 {
            rgb *= edge_shade(atmosphere, in.edges, in.face);
        }
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
