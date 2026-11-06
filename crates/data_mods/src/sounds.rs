//! `sounds.toml`: sounds read from Ogg Vorbis or WAV files in the mod folder, for blocks and targets to
//! name.
//!
//! ```toml
//! [sounds.step_stone]
//! files = ["sounds/step_a.ogg", "sounds/step_b.ogg"]  # variations, one picked each play; a
//!                                                     # `*` in a file name matches any text,
//!                                                     # such as "sounds/footstep_concrete_*.ogg"
//! volume = 0.6         # optional, 1 if left out: a gain on the file as recorded
//! pitch = 1.0          # optional, 1 if left out: playback speed, so also pitch
//! pitch_spread = 0.08  # optional, 0 if left out: how much each play's pitch varies, either way
//! looping = false      # optional: repeats until stopped, such as a motor
//! ```
//!
//! Names are short and get the mod's namespace. Blocks name sounds in `blocks.toml` with
//! `sounds = { step = "step_stone", broken = "...", placed = "..." }`, and targets in
//! `targets.toml` with `sounds = { hit = "moo", step = "...", slam = "...", death = "..." }`. A short name
//! there is this mod's own and must be defined here; a full one is checked once every mod has
//! loaded.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use api::block::is_valid_name;
use api::{BlockSounds, SoundDef, TargetSounds};
use serde::Deserialize;
use toml::Spanned;

use crate::{LoadError, Source};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SoundsFile {
    #[serde(default)]
    sounds: BTreeMap<String, Spanned<SoundEntry>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SoundEntry {
    files: Vec<String>,
    #[serde(default = "one")]
    volume: f32,
    #[serde(default = "one")]
    pitch: f32,
    #[serde(default)]
    pitch_spread: f32,
    #[serde(default)]
    looping: bool,
}

/// `sounds = { ... }` on a block.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct BlockSoundsEntry {
    step: Option<Spanned<String>>,
    broken: Option<Spanned<String>>,
    placed: Option<Spanned<String>>,
}

/// `sounds = { ... }` on a target.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetSoundsEntry {
    hit: Option<Spanned<String>>,
    step: Option<Spanned<String>>,
    slam: Option<Spanned<String>>,
    death: Option<Spanned<String>>,
}

fn one() -> f32 {
    1.0
}

/// Every sound `sounds.toml` in `dir` defines, in name order, or none if there's no such file.
pub(crate) fn load(dir: &Path, namespace: &str) -> Result<Vec<SoundDef>, LoadError> {
    let path = dir.join("sounds.toml");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = Source::read(path)?;
    let entries: SoundsFile = file.parse()?;
    let mut sounds = Vec::new();
    for (short, entry) in &entries.sounds {
        let at = |message: String| file.error_at(entry, format!("sound {short:?}: {message}"));
        let name = format!("{namespace}:{short}");
        if !is_valid_name(&name) {
            return Err(at("invalid name: use lowercase letters, digits and underscores".into()));
        }
        let e = entry.get_ref();
        if e.files.is_empty() {
            return Err(at("files is empty".into()));
        }
        let mut variations = Vec::new();
        for pattern in &e.files {
            let paths = matching(dir, pattern).map_err(|err| at(format!("{pattern}: {err}")))?;
            for path in paths {
                variations.push(fs::read(dir.join(&path)).map_err(|err| at(format!("{path}: {err}")))?);
            }
        }
        let def = SoundDef { name, variations, volume: e.volume, pitch: e.pitch, pitch_spread: e.pitch_spread, looping: e.looping };
        def.validate().map_err(|why| file.error_at(entry, why))?;
        sounds.push(def);
    }
    Ok(sounds)
}

/// The files `pattern` names, relative to `dir`: itself, or with a `*` in its file name, every
/// file in that folder it matches, in name order. Matching nothing is an error.
fn matching(dir: &Path, pattern: &str) -> Result<Vec<String>, String> {
    let (folder, name) = pattern.rsplit_once('/').unwrap_or(("", pattern));
    let Some((before, after)) = name.split_once('*') else {
        return Ok(vec![pattern.to_owned()]);
    };
    if after.contains('*') {
        return Err("only one * is allowed".into());
    }
    let listing = fs::read_dir(dir.join(folder)).map_err(|e| e.to_string())?;
    let mut found: Vec<String> = listing
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|f| f.len() >= before.len() + after.len() && f.starts_with(before) && f.ends_with(after))
        .map(|f| if folder.is_empty() { f } else { format!("{folder}/{f}") })
        .collect();
    if found.is_empty() {
        return Err("matches no files".into());
    }
    found.sort();
    Ok(found)
}

/// A sound named in another file: a short name is this mod's own and must be among `defined`;
/// a full one is checked once every mod has loaded.
pub(crate) fn reference(file: &Source, namespace: &str, defined: &[SoundDef], spanned: &Spanned<String>) -> Result<String, LoadError> {
    let short = spanned.get_ref();
    let full = if short.contains(':') { short.clone() } else { format!("{namespace}:{short}") };
    let own = full.strip_prefix(namespace).is_some_and(|rest| rest.starts_with(':'));
    if !is_valid_name(&full) || (own && !defined.iter().any(|s| s.name == full)) {
        return Err(file.error_at(spanned, format!("no sound {short:?}: sounds.toml has no [sounds.{short}]")));
    }
    Ok(full)
}

impl BlockSoundsEntry {
    pub(crate) fn resolve(&self, file: &Source, namespace: &str, defined: &[SoundDef]) -> Result<BlockSounds, LoadError> {
        let get = |s: &Option<Spanned<String>>| s.as_ref().map(|s| reference(file, namespace, defined, s)).transpose();
        Ok(BlockSounds { step: get(&self.step)?, broken: get(&self.broken)?, placed: get(&self.placed)? })
    }
}

impl TargetSoundsEntry {
    pub(crate) fn resolve(&self, file: &Source, namespace: &str, defined: &[SoundDef]) -> Result<TargetSounds, LoadError> {
        let get = |s: &Option<Spanned<String>>| s.as_ref().map(|s| reference(file, namespace, defined, s)).transpose();
        Ok(TargetSounds { hit: get(&self.hit)?, step: get(&self.step)?, slam: get(&self.slam)?, death: get(&self.death)? })
    }
}
