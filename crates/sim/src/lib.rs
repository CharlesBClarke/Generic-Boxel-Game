//! Engine simulation. Holds the world and advances it in fixed ticks. Knows nothing about
//! rendering, so a headless server is just this crate plus mods.

pub mod edit;
pub mod layer;
pub mod player;
pub mod raycast;
pub mod registry;
pub mod simulation;
pub mod world;

pub use layer::Layer;
pub use player::{Player, PlayerInput, PushOut, Shape, Terrain};
pub use raycast::{Hit, raycast};
pub use registry::{BlockId, BlockRegistry, TextureRegistry};
pub use simulation::{SimError, Simulation};
pub use world::{Chunk, Grid, GridId, Universe};
