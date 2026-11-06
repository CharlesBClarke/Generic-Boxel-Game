//! `targets.toml`: models, and targets drawn through them and heard through sounds from
//! `sounds.toml`.
//!
//! ```toml
//! [models.cow]
//! mesh = "models/cow.glb"       # Z up, meters, feet at the origin, facing +Y: an OBJ file, or
//!                               # a glTF binary with a skeleton and named animations
//! texture = "textures/cow.png"  # a PNG, drawn with hard pixel edges
//! play_once = ["eat", "die"]    # optional: animations that hold their last pose; others loop
//!
//! [[target]]
//! name = "cow"
//! model = "cow"
//! sounds = { hit = "moo", step = "hoof", slam = "thud", death = "splat" }  # optional, each one too: names
//!                               # from sounds.toml
//! brain = "cow"                 # optional: a brain a code mod registers; without one it's a
//!                               # dummy that only moves when hit
//! width = 0.9                   # every number is required unless marked; see api::TargetDef
//! height = 1.3
//! step_height = 0.6             # optional, 0 if left out
//! health = 20
//! weight = 3
//! gravity = 40
//! drag = 9
//! air_drag = 3
//! bounce = 0.25
//! respawn_time = 3
//! respawns = false              # optional, true if left out: comes back on its own
//! slam_speed = 4
//! slam_damage = 2
//!
//! [[target.hit_box]]            # optional: the parts a strike can hit, in the model's space
//! name = "head"
//! min = [-0.28, 1.1, 0.8]
//! max = [0.28, 1.84, 1.45]
//! damage = 2                    # multiplies the tool's damage
//! ```
//!
//! Paths are relative to the mod folder. Model names are short and get the mod's namespace; a
//! target may also name another mod's models and sounds in full.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use api::block::is_valid_name;
use api::{HitBox, ModelDef, SoundDef, TargetDef, Texture};

use crate::sounds::TargetSoundsEntry;
use serde::Deserialize;
use toml::Spanned;

use crate::{LoadError, Source};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetsFile {
    #[serde(default)]
    models: BTreeMap<String, Spanned<ModelEntry>>,
    #[serde(default)]
    target: Vec<TargetEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelEntry {
    mesh: String,
    texture: String,
    #[serde(default)]
    play_once: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetEntry {
    name: Spanned<String>,
    model: Spanned<String>,
    #[serde(default)]
    sounds: TargetSoundsEntry,
    brain: Option<Spanned<String>>,
    width: f32,
    height: f32,
    #[serde(default)]
    step_height: f32,
    health: f32,
    weight: f32,
    gravity: f32,
    drag: f32,
    air_drag: f32,
    bounce: f32,
    respawn_time: f32,
    #[serde(default = "yes")]
    respawns: bool,
    slam_speed: f32,
    slam_damage: f32,
    #[serde(default)]
    hit_box: Vec<HitBoxEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HitBoxEntry {
    name: String,
    min: [f32; 3],
    max: [f32; 3],
    damage: f32,
}

/// Everything `targets.toml` defines.
pub(crate) type Media = (Vec<ModelDef>, Vec<TargetDef>);

/// A brain's full name. Brains come from code, so a short one is only checked for form here;
/// whether it exists is checked once every mod has registered.
fn brain_name(spanned: &Spanned<String>, namespace: &str, file: &Source) -> Result<String, LoadError> {
    let short = spanned.get_ref();
    let full = if short.contains(':') { short.clone() } else { format!("{namespace}:{short}") };
    if is_valid_name(&full) {
        Ok(full)
    } else {
        Err(file.error_at(spanned, format!("invalid brain name {short:?}: use lowercase letters, digits and underscores")))
    }
}

/// An OBJ file's vertex positions, texture coordinates and triangle indices.
type ObjMesh = (Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>);

/// What `targets.toml` in `dir` defines, or nothing if there's no such file. `sounds` are the
/// ones `sounds.toml` defined.
pub(crate) fn load(dir: &Path, namespace: &str, sounds: &[SoundDef]) -> Result<Media, LoadError> {
    let path = dir.join("targets.toml");
    if !path.exists() {
        return Ok(Default::default());
    }
    let file = Source::read(path)?;
    let entries: TargetsFile = file.parse()?;
    let named = |short: &str, span: std::ops::Range<usize>, what: &str| {
        let name = format!("{namespace}:{short}");
        if is_valid_name(&name) {
            Ok(name)
        } else {
            let message = format!("invalid {what} name {short:?}: use lowercase letters, digits and underscores");
            Err(file.error_at(&Spanned::new(span, ()), message))
        }
    };

    let mut models = Vec::new();
    for (short, entry) in &entries.models {
        let name = named(short, entry.span(), "model")?;
        let e = entry.get_ref();
        let at = |message: String| file.error_at(entry, format!("model {short:?}: {message}"));
        let mesh_err = |err: String| at(format!("{}: {err}", e.mesh));
        let bytes = fs::read(dir.join(&e.mesh)).map_err(|err| mesh_err(err.to_string()))?;
        let (positions, uvs, indices, skin) = if e.mesh.ends_with(".glb") {
            crate::glb::read(&bytes, &e.play_once).map_err(mesh_err)?
        } else {
            if !e.play_once.is_empty() {
                return Err(at("play_once needs an animated model (.glb)".into()));
            }
            let obj = String::from_utf8(bytes).map_err(|_| mesh_err("not text".into()))?;
            let (positions, uvs, indices) = parse_obj(&obj).map_err(mesh_err)?;
            (positions, uvs, indices, None)
        };
        let png = fs::read(dir.join(&e.texture)).map_err(|err| at(format!("{}: {err}", e.texture)))?;
        let texture = decode_png(&png).map_err(|err| at(format!("{}: {err}", e.texture)))?;
        let def = ModelDef { name, positions, uvs, indices, texture, skin };
        def.validate().map_err(at)?;
        models.push(def);
    }

    // A short name is this mod's own and must be defined here; a full one is checked once
    // every mod has loaded.
    let reference = |spanned: &Spanned<String>, defined: &[String], what: &str, table: &str| {
        let short = spanned.get_ref();
        let full = if short.contains(':') { short.clone() } else { format!("{namespace}:{short}") };
        let own = full.strip_prefix(namespace).is_some_and(|rest| rest.starts_with(':'));
        if !is_valid_name(&full) || (own && !defined.contains(&full)) {
            return Err(file.error_at(spanned, format!("no {what} {short:?}: there is no [{table}.{short}]")));
        }
        Ok(full)
    };
    let model_names: Vec<String> = models.iter().map(|m| m.name.clone()).collect();
    let mut targets = Vec::new();
    for entry in entries.target {
        let short = entry.name.get_ref();
        let name = named(short, entry.name.span(), "target")?;
        let def = TargetDef {
            name,
            model: reference(&entry.model, &model_names, "model", "models")?,
            sounds: entry.sounds.resolve(&file, namespace, sounds)?,
            brain: entry.brain.as_ref().map(|b| brain_name(b, namespace, &file)).transpose()?,
            width: entry.width,
            height: entry.height,
            step_height: entry.step_height,
            hit_boxes: entry.hit_box.iter().map(|b| HitBox { name: b.name.clone(), min: b.min, max: b.max, damage: b.damage }).collect(),
            health: entry.health,
            weight: entry.weight,
            gravity: entry.gravity,
            drag: entry.drag,
            air_drag: entry.air_drag,
            bounce: entry.bounce,
            respawn_time: entry.respawn_time,
            slam_speed: entry.slam_speed,
            slam_damage: entry.slam_damage,
            respawns: entry.respawns,
        };
        def.validate().map_err(|why| file.error_at(&entry.name, format!("target {short:?}: {why}")))?;
        targets.push(def);
    }
    Ok((models, targets))
}

/// Reads the vertex positions, texture coordinates and faces of an OBJ file into one vertex per
/// distinct position and texture coordinate pair, and three indices per triangle. Faces with
/// more than three corners are split into a fan. Everything else in the file is skipped.
fn parse_obj(text: &str) -> Result<ObjMesh, String> {
    let mut v = Vec::new();
    let mut vt = Vec::new();
    let mut corners: BTreeMap<(usize, usize), u32> = BTreeMap::new();
    let (mut positions, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new());
    for (number, line) in text.lines().enumerate() {
        let at = |message: &str| format!("line {}: {message}", number + 1);
        let mut parts = line.split_whitespace();
        let numbers = |parts: std::str::SplitWhitespace, count: usize| -> Result<Vec<f32>, String> {
            let values: Vec<f32> = parts.take(count).map(str::parse).collect::<Result<_, _>>().map_err(|_| at("expected numbers"))?;
            if values.len() < count { Err(at(&format!("expected {count} numbers"))) } else { Ok(values) }
        };
        match parts.next() {
            Some("v") => {
                let n = numbers(parts, 3)?;
                v.push([n[0], n[1], n[2]]);
            }
            Some("vt") => {
                let n = numbers(parts, 2)?;
                // OBJ counts texture rows from the bottom.
                vt.push([n[0], 1.0 - n[1]]);
            }
            Some("f") => {
                let mut face = Vec::new();
                for corner in parts {
                    let mut refs = corner.split('/');
                    let index = |r: Option<&str>, len: usize| -> Result<usize, String> {
                        let i: usize = r.and_then(|r| r.parse().ok()).ok_or_else(|| at("faces need v/vt corners"))?;
                        if i == 0 || i > len { Err(at(&format!("index {i} out of range"))) } else { Ok(i - 1) }
                    };
                    let key = (index(refs.next(), v.len())?, index(refs.next(), vt.len())?);
                    let vertex = *corners.entry(key).or_insert_with(|| {
                        positions.push(v[key.0]);
                        uvs.push(vt[key.1]);
                        positions.len() as u32 - 1
                    });
                    face.push(vertex);
                }
                if face.len() < 3 {
                    return Err(at("a face needs at least three corners"));
                }
                for i in 1..face.len() - 1 {
                    indices.extend([face[0], face[i], face[i + 1]]);
                }
            }
            _ => {}
        }
    }
    if indices.is_empty() {
        return Err("no faces".into());
    }
    Ok((positions, uvs, indices))
}

/// Decodes a PNG of any color type into sRGB with alpha.
fn decode_png(bytes: &[u8]) -> Result<Texture, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("image too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let pixels = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed colors weren't expanded".into()),
    };
    Ok(Texture { width: info.width, height: info.height, rgba })
}

fn yes() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_obj_sharing_corners_and_splitting_quads() {
        let obj = "# a quad\nv 0 0 0\nv 1 0 0\nv 1 0 1\nv 0 0 1\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nf 1/1 2/2 3/3 4/4\n";
        let (positions, uvs, indices) = parse_obj(obj).unwrap();
        assert_eq!(positions.len(), 4);
        assert_eq!(indices, [0, 1, 2, 0, 2, 3]);
        assert_eq!(uvs[0], [0.0, 1.0], "rows count from the top");
        assert!(parse_obj("v 0 0 0\nf 1/1 1/1 1/1\n").is_err(), "no texture coordinate 1");
        assert!(parse_obj("v 0 0\n").is_err());
        assert!(parse_obj("v 0 0 0\n").is_err(), "no faces");
    }

    #[test]
    fn decodes_a_png() {
        // A 1×1 opaque red PNG.
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&[255, 0, 0]).unwrap();
        }
        let t = decode_png(&bytes).unwrap();
        assert_eq!((t.width, t.height, t.rgba), (1, 1, vec![255, 0, 0, 255]));
        assert!(decode_png(b"not a png").is_err());
    }
}
