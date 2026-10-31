//! Engine simulation. Holds the world and advances it in fixed ticks. Knows nothing about
//! rendering, so a headless server is just this crate plus mods.

pub mod cell;
pub mod combat;
pub mod edit;
pub mod items;
pub mod layer;
pub mod player;
pub mod raycast;
pub mod registry;
pub mod simulation;
pub mod world;

pub use combat::{Slam, Strike, Swing, SwingStage, Target, TargetHit, TargetKind, Targets, ToolId};
pub use cell::{CellBox, CellId, CellTable, MAX_OCCUPANTS, Occupant, ShareError, can_share, occupant_boxes};
pub use items::{Collector, Item, Items};
pub use layer::Layer;
pub use player::{Player, PlayerInput, PushOut, Shape, StepReport, Terrain};
pub use raycast::{Hit, raycast};
pub use registry::{BlockId, BlockRegistry, Drop, FULL_VOLUME, LookRegistry, MaterialId, volume};
pub use simulation::{FACE_CENTER, PlaceReport, PlaceTarget, SimError, Simulation, face_region};
pub use world::{Chunk, Grid, GridId, Universe};
