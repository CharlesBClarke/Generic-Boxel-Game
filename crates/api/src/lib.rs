//! The public mod API. The base game uses only this crate, so anything a mod can't
//! express here is a gap to fill, not something to reach around.
//!
//! The API is plain Rust for now but will become a WebAssembly boundary, so calls exchange
//! plain data: no references into engine or mod memory, no closures, no generic traits.

pub mod block;
pub mod brain;
pub mod combat;
pub mod crafting;
pub mod coords;
pub mod crowd;
pub mod error;
pub mod health;
#[cfg(target_arch = "wasm32")]
pub mod guest;
pub mod interaction;
pub mod item;
pub mod media;
pub mod modding;
pub mod orientation;
pub mod movement;
pub mod player;
pub mod shape;
pub mod terrain;
pub mod tuning;

pub use block::{BlockBox, BlockDef, LookDef, Sheen, Turning};
pub use brain::{Attack, Brain, BrainContext, BrainOutput, Hurt, MAX_NEIGHBORS, Neighbor, Seen};
pub use combat::{HitBox, TargetDef, ToolDef};
pub use crafting::{CraftError, CraftingDef, Layer, Stack, Stacks, craft_shape};
pub use coords::{BoxelPos, CHUNK_SIZE, CHUNK_VOLUME, ChunkPos, LocalPos, WorldPos, look_direction};
pub use crowd::CrowdDef;
pub use error::ApiError;
pub use health::HealthDef;
pub use interaction::{InteractionDef, PushOutDef};
pub use item::{InventoryDef, ItemDropDef};
pub use media::{
    BlockSounds, Bone, BonePose, Clip, ModelDef, PlayerSounds, Skin, SoundDef, TargetSounds, Texture, ToolSounds, Track,
};
pub use glam::{I64Vec3, Vec2, Vec3};
pub use modding::{Mod, Registrar, WorldEdit};
pub use orientation::Orientation;
pub use movement::{Ground, MoveContext, MoveInput, MoveOutput, MovementController};
pub use player::PlayerDef;
pub use shape::ShapeGrid;
pub use terrain::{ChunkFill, Generator, TerrainDef};
pub use tuning::{Tunable, Tunables};
