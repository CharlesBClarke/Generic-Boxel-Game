//! `crafting.toml`: which block is the crafting table.
//!
//! ```toml
//! table = "crafting_table"   # a block; short names get the mod's namespace
//! ```
//!
//! A short name must be one of this mod's blocks. A full name may name another mod's, checked
//! by the engine.

use std::path::Path;

use api::block::is_valid_name;
use api::{BlockDef, CraftingDef};
use serde::Deserialize;
use toml::Spanned;

use crate::{LoadError, Source};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CraftingFile {
    table: Spanned<String>,
}

/// What `crafting.toml` in `dir` defines, or nothing if there's no such file. `blocks` are this
/// mod's, for checking a short table name.
pub(crate) fn load(dir: &Path, namespace: &str, blocks: &[BlockDef]) -> Result<Option<CraftingDef>, LoadError> {
    let path = dir.join("crafting.toml");
    if !path.exists() {
        return Ok(None);
    }
    let file = Source::read(path)?;
    let entries: CraftingFile = file.parse()?;
    let short = entries.table.get_ref();
    let table = if short.contains(':') { short.clone() } else { format!("{namespace}:{short}") };
    if !is_valid_name(&table) {
        let message = format!("table {short:?}: use lowercase letters, digits and underscores");
        return Err(file.error_at(&entries.table, message));
    }
    if !short.contains(':') && !blocks.iter().any(|b| b.name == table) {
        return Err(file.error_at(&entries.table, format!("table {short:?}: no block in this mod has that name")));
    }
    Ok(Some(CraftingDef { table }))
}
