//! The public mod API. The base game uses only this crate, so anything a mod can't
//! express here is a gap to fill, not something to reach around.
//!
//! The API is plain Rust for now but will become a WebAssembly boundary, so calls exchange
//! plain data: no references into engine or mod memory, no closures, no generic traits.

pub mod block;
pub mod coords;
pub mod error;
#[cfg(target_arch = "wasm32")]
pub mod guest;
pub mod interaction;
pub mod modding;
pub mod orientation;
pub mod movement;
pub mod player;
pub mod tuning;

pub use block::{BlockBox, BlockDef, FaceTextures, TextureDef, Turning};
pub use coords::{BoxelPos, CHUNK_SIZE, CHUNK_VOLUME, ChunkPos, LocalPos, WorldPos, look_direction};
pub use error::ApiError;
pub use interaction::{InteractionDef, PushOutDef};
pub use glam::{I64Vec3, Vec2, Vec3};
pub use modding::{Mod, Registrar, WorldEdit};
pub use orientation::Orientation;
pub use movement::{Ground, MoveContext, MoveInput, MoveOutput, MovementController};
pub use player::PlayerDef;
pub use tuning::{Tunable, Tunables};
