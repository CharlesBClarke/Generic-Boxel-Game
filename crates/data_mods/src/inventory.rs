//! `inventory.toml`: what a player carries.
//!
//! ```toml
//! capacity = 64       # boxels' worth, every material together; leave out for no limit
//! throw_speed = 6     # meters per second, along the look
//! pickup_delay = 1.5  # seconds before a thrown item can be picked up again
//!
//! endless = ["copper"] # materials that never run out (optional)
//!
//! [start]             # what a player starts with, in boxels' worth (optional)
//! brick = 16
//! ```
//!
//! Material names are short and get the mod's namespace; a short name must be the material of
//! one of this mod's blocks. A full name may name another mod's, checked by the engine.

use std::collections::BTreeMap;
use std::path::Path;

use api::block::is_valid_name;
use api::{BlockDef, InventoryDef};
use serde::Deserialize;
use toml::Spanned;

use crate::{LoadError, Source};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InventoryFile {
    capacity: Option<Spanned<f32>>,
    throw_speed: f32,
    pickup_delay: f32,
    #[serde(default)]
    start: BTreeMap<Spanned<String>, Spanned<f32>>,
    #[serde(default)]
    endless: Vec<Spanned<String>>,
}

/// What `inventory.toml` in `dir` defines, or nothing if there's no such file. `blocks` are
/// this mod's, for checking short material names.
pub(crate) fn load(dir: &Path, namespace: &str, blocks: &[BlockDef]) -> Result<Option<InventoryDef>, LoadError> {
    let path = dir.join("inventory.toml");
    if !path.exists() {
        return Ok(None);
    }
    let file = Source::read(path)?;
    let entries: InventoryFile = file.parse()?;
    // A material's full name, checked: a short one must be the material of one of this mod's blocks.
    let material = |short: &Spanned<String>, what: &str| {
        let m = short.get_ref();
        let full = if m.contains(':') { m.clone() } else { format!("{namespace}:{m}") };
        if !is_valid_name(&full) {
            let message = format!("{what} material {m:?}: use lowercase letters, digits and underscores");
            return Err(file.error_at(short, message));
        }
        if !m.contains(':') && !blocks.iter().any(|b| b.material.as_deref() == Some(full.as_str())) {
            return Err(file.error_at(short, format!("{what} material {m:?}: no block in this mod is made of it")));
        }
        Ok(full)
    };
    let mut start = Vec::new();
    for (short, amount) in &entries.start {
        let m = short.get_ref();
        let full = material(short, "start")?;
        let amount = *amount.get_ref();
        if !(amount.is_finite() && amount >= 0.0) {
            return Err(file.error_at(short, format!("start amount of {m:?} must be at least 0, got {amount}")));
        }
        start.push((full, amount));
    }
    let def = InventoryDef {
        capacity: entries.capacity.as_ref().map(|c| *c.get_ref()),
        throw_speed: entries.throw_speed,
        pickup_delay: entries.pickup_delay,
        start,
        endless: entries.endless.iter().map(|m| material(m, "endless")).collect::<Result<_, _>>()?,
    };
    // Point at the capacity if there is one; it's what the start is measured against too.
    let at = entries.capacity.clone().unwrap_or_else(|| Spanned::new(0..0, 0.0));
    def.validate().map_err(|why| file.error_at(&at, why))?;
    Ok(Some(def))
}
