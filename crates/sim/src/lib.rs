//! Engine simulation. Holds the world and advances it in fixed ticks. Knows nothing about
//! rendering, so a headless server is just this crate plus mods.

pub mod blueprint;
pub mod cell;
pub mod combat;
pub mod crafting;
pub mod edit;
pub mod inventory;
pub mod items;
pub mod layer;
pub mod path;
pub mod player;
pub mod raycast;
pub mod registry;
pub mod save;
pub mod signal;
pub mod simulation;
pub mod terrain;
pub mod view;
pub mod world;

pub use combat::{Blow, Slam, StepContext, Strike, Swing, SwingStage, Target, TargetHit, TargetKind, Targets, ToolId};
pub use cell::{CellBox, CellId, CellTable, MAX_MATERIALS, MAX_OCCUPANTS, Occupant, ShareError, can_share, occupant_boxes};
pub use blueprint::{Blueprint, BlueprintEdit, BlueprintId, BlueprintRefusal, Change, Design, Library, Variation};
pub use crafting::{Cells, Craft, CraftPlan, CraftRefusal, Piece, Recipe, Source, Thing};
pub use inventory::Inventory;
pub use items::{Collector, Item, Items};
pub use layer::Layer;
pub use player::{Body, Player, PlayerInput, PushOut, Shape, StepReport, Terrain};
pub use raycast::{Hit, raycast};
pub use registry::{BlockId, BlockRegistry, Derived, Drop, FULL_VOLUME, LookRegistry, MaterialId, volume};
pub use save::SaveData;
pub use simulation::{BlockEdit, LoadReport, PlayerHurt, SimError, Simulation};
pub use terrain::{GenerationStats, WorldSettings};
pub use view::{FACE_CENTER, PlaceTarget, View, face_region};
pub use world::{Chunk, ChunkData, Grid, GridId, Universe};
