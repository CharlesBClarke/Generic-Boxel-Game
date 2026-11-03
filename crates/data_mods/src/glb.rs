//! Animated models from glTF binary (`.glb`) files: the mesh, its skeleton and its named clips.
//!
//! The file must already be in the API's space: Z up, meters, feet at the origin, front facing
//! +Y. Its buffers must be inside the file, and its texture comes from `targets.toml`, not the
//! file. Clips may move bones and turn them; scaling bones and morph targets aren't supported.

use std::collections::HashMap;

use api::{Bone, BonePose, Clip, Skin, Track};
use gltf::animation::{Interpolation, Property};
use gltf::animation::util::ReadOutputs;

/// A model's vertex positions, texture coordinates, triangle indices and skin.
pub(crate) type GlbMesh = (Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>, Option<Skin>);

/// Reads every triangle of the file's first mesh and, if a node gives it a skin, the skin and
/// every animation in the file as clips. Clips loop unless named in `play_once`.
pub(crate) fn read(bytes: &[u8], play_once: &[String]) -> Result<GlbMesh, String> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| format!("not a glTF file: {e}"))?;
    let blob = gltf.blob.as_deref();
    let buffer = |b: gltf::Buffer| match b.source() {
        gltf::buffer::Source::Bin => blob,
        gltf::buffer::Source::Uri(_) => None,
    };
    let doc = &gltf.document;
    if doc.buffers().any(|b| matches!(b.source(), gltf::buffer::Source::Uri(_))) {
        return Err("buffers must be inside the file (export as .glb)".into());
    }

    let mesh = doc.meshes().next().ok_or("no mesh")?;
    let (mut positions, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new());
    let (mut joints, mut weights) = (Vec::new(), Vec::new());
    for primitive in mesh.primitives() {
        if primitive.mode() != gltf::mesh::Mode::Triangles {
            return Err("only triangles are supported".into());
        }
        let reader = primitive.reader(buffer);
        let base = positions.len() as u32;
        let p: Vec<[f32; 3]> = reader.read_positions().ok_or("a mesh without positions")?.collect();
        let count = p.len();
        positions.extend(p);
        match reader.read_tex_coords(0) {
            Some(t) => uvs.extend(t.into_f32()),
            None => uvs.extend(std::iter::repeat_n([0.0; 2], count)),
        }
        match reader.read_indices() {
            Some(i) => indices.extend(i.into_u32().map(|i| base + i)),
            None => indices.extend(base..base + count as u32),
        }
        if let (Some(j), Some(w)) = (reader.read_joints(0), reader.read_weights(0)) {
            joints.extend(j.into_u16());
            weights.extend(w.into_f32());
        }
    }
    if uvs.len() != positions.len() {
        return Err("texture coordinates don't match the vertices".into());
    }

    let Some(skin) = doc.nodes().find(|n| n.mesh().is_some_and(|m| m.index() == mesh.index())).and_then(|n| n.skin()) else {
        return Ok((positions, uvs, indices, None));
    };
    if joints.len() != positions.len() || weights.len() != positions.len() {
        return Err("a skinned mesh needs bones and weights for every vertex".into());
    }
    for (v, w) in weights.iter_mut().enumerate() {
        let total: f32 = w.iter().sum();
        if total.is_nan() || total <= 0.0 {
            return Err(format!("vertex {v} isn't bound to any bone"));
        }
        w.iter_mut().for_each(|x| *x /= total);
    }

    // Bones in the skin's order; each node's parent, to find the bones'.
    let bone_of: HashMap<usize, u16> = skin.joints().enumerate().map(|(i, n)| (n.index(), i as u16)).collect();
    let mut parent_of = HashMap::new();
    for node in doc.nodes() {
        for child in node.children() {
            parent_of.insert(child.index(), node.clone());
        }
    }
    let binds: Vec<[[f32; 4]; 4]> = match skin.reader(buffer).read_inverse_bind_matrices() {
        Some(m) => m.collect(),
        None => vec![[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]; bone_of.len()],
    };
    let mut bones = Vec::new();
    for (i, node) in skin.joints().enumerate() {
        let name = node.name().unwrap_or("").to_string();
        let parent = match parent_of.get(&node.index()) {
            Some(p) => match bone_of.get(&p.index()) {
                Some(&b) => Some(b),
                None if is_identity(p) && !parent_of.contains_key(&p.index()) => None,
                None => {
                    return Err(format!("bone {name:?} hangs from {:?}, which isn't a bone and moves it", p.name().unwrap_or("")));
                }
            },
            None => None,
        };
        let (translation, rotation, scale) = node.transform().decomposed();
        let inverse_bind = binds.get(i).ok_or("too few inverse bind matrices")?.concat().try_into().unwrap();
        bones.push(Bone { name, parent, rest: BonePose { translation, rotation, scale }, inverse_bind });
    }

    let mut clips = Vec::new();
    for animation in doc.animations() {
        let name = animation.name().ok_or("every animation needs a name")?.to_string();
        let mut tracks: Vec<Track> = Vec::new();
        let (mut start, mut end) = (f32::INFINITY, f32::NEG_INFINITY);
        for channel in animation.channels() {
            let at = |why: &str| format!("animation {name:?}: {why}");
            let Some(&bone) = bone_of.get(&channel.target().node().index()) else {
                return Err(at("moves a node that isn't a bone"));
            };
            if channel.sampler().interpolation() != Interpolation::Linear {
                return Err(at("only linear blending between keys is supported"));
            }
            let reader = channel.reader(buffer);
            let times: Vec<f32> = reader.read_inputs().ok_or_else(|| at("keys without times"))?.collect();
            if let (Some(first), Some(last)) = (times.first(), times.last()) {
                start = start.min(*first);
                end = end.max(*last);
            }
            let index = match tracks.iter().position(|t| t.bone == bone) {
                Some(i) => i,
                None => {
                    tracks.push(Track { bone, translations: Vec::new(), rotations: Vec::new() });
                    tracks.len() - 1
                }
            };
            let track = &mut tracks[index];
            match (channel.target().property(), reader.read_outputs()) {
                (Property::Translation, Some(ReadOutputs::Translations(v))) => track.translations = times.iter().copied().zip(v).collect(),
                (Property::Rotation, Some(ReadOutputs::Rotations(v))) => track.rotations = times.iter().copied().zip(v.into_f32()).collect(),
                (Property::Scale, _) => return Err(at("scaling bones isn't supported")),
                (Property::MorphTargetWeights, _) => return Err(at("morph targets aren't supported")),
                _ => return Err(at("keys without values")),
            }
        }
        // Clips start at 0, wherever the file's keys begin.
        for track in &mut tracks {
            track.translations.iter_mut().for_each(|k| k.0 -= start);
            track.rotations.iter_mut().for_each(|k| k.0 -= start);
        }
        let looping = !play_once.contains(&name);
        clips.push(Clip { length: end - start, looping, tracks, name });
    }
    if let Some(missing) = play_once.iter().find(|n| !clips.iter().any(|c| &c.name == *n)) {
        return Err(format!("play_once names {missing:?}, but there's no such animation"));
    }
    let skin = Skin { joints, weights, bones, clips };
    Ok((positions, uvs, indices, Some(skin)))
}

fn is_identity(node: &gltf::Node) -> bool {
    let (t, r, s) = node.transform().decomposed();
    t == [0.0; 3] && r == [0.0, 0.0, 0.0, 1.0] && s == [1.0; 3]
}
