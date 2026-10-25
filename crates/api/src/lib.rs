//! The public mod API. The base game uses only this crate, so anything a mod can't
//! express here is a gap to fill, not something to reach around.

pub mod block;
pub mod coords;
pub mod error;
pub mod modding;
pub mod player;

pub use block::BlockDef;
pub use coords::{BoxelPos, CHUNK_SIZE, CHUNK_VOLUME, ChunkPos, LocalPos, WorldPos};
pub use error::ApiError;
pub use glam::{I64Vec3, Vec3};
pub use modding::{Mod, Registrar, WorldEdit};
pub use player::PlayerDef;
