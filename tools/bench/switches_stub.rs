//! The bench's switches, for a commit older than the bench (copied in by `tools/bench/
//! backport.sh`): it knows none, so every setting goes to the tuning file, and the build warns
//! about any it doesn't know.

use bevy::prelude::*;

pub fn set(_world: &mut World, _key: &str, _on: bool) -> bool {
    false
}
