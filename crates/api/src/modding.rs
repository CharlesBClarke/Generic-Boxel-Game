//! The traits a mod implements and the engine services it is handed.

use crate::{
    ApiError, BlockDef, Brain, BoxelPos, CraftingDef, CrowdDef, HealthDef, InteractionDef, InventoryDef, ItemDropDef, LookDef, ModelDef,
    Generator, MovementController, Orientation, PlayerDef, PushOutDef, ShapeGrid, SignalDef, SoundDef, TargetDef, TerrainDef, ToolDef,
    WorldPos,
};

/// A mod. The engine calls `register` on every mod first, then `build_world` on every mod.
pub trait Mod {
    /// The mod's namespace, such as `"base"`.
    fn name(&self) -> &str;

    /// Registers content: looks, blocks, movement controllers, brains, the player, interaction, push-out,
    /// crowd, health, item drop, inventory and crafting definitions, models, sounds, targets and tools.
    fn register(&self, registrar: &mut dyn Registrar) -> Result<(), ApiError>;

    /// Places blocks in the world once all content is registered.
    fn build_world(&self, _world: &mut dyn WorldEdit) -> Result<(), ApiError> {
        Ok(())
    }
}

/// Where mods register their content.
pub trait Registrar {
    /// Registers a look for blocks to name. Blocks may name looks registered later, even by
    /// another mod; every name is checked once all mods have registered.
    fn register_look(&mut self, def: LookDef) -> Result<(), ApiError>;
    fn register_block(&mut self, def: BlockDef) -> Result<(), ApiError>;
    /// Registers a movement controller under a namespaced name, for player definitions to
    /// refer to. Native mods only: a WebAssembly mod exports its controller instead (see
    /// `export_movement_controller!`).
    fn register_movement_controller(
        &mut self,
        name: &str,
        controller: Box<dyn MovementController>,
    ) -> Result<(), ApiError>;
    /// Registers a brain under a namespaced name, for target definitions to refer to. Checked
    /// once all mods have registered, so a target may name a brain another mod registers.
    fn register_brain(&mut self, name: &str, brain: Box<dyn Brain>) -> Result<(), ApiError>;
    /// Registers a terrain generator under a namespaced name, for terrain placements to refer to.
    fn register_generator(&mut self, name: &str, generator: Box<dyn Generator>) -> Result<(), ApiError>;
    fn register_player(&mut self, def: PlayerDef) -> Result<(), ApiError>;
    fn register_interaction(&mut self, def: InteractionDef) -> Result<(), ApiError>;
    fn register_push_out(&mut self, def: PushOutDef) -> Result<(), ApiError>;
    /// Registers how creatures share space: seeing each other and being pushed apart.
    fn register_crowd(&mut self, def: CrowdDef) -> Result<(), ApiError>;
    /// Registers the player's health: without it, nothing can hurt them.
    fn register_health(&mut self, def: HealthDef) -> Result<(), ApiError>;
    fn register_item_drops(&mut self, def: ItemDropDef) -> Result<(), ApiError>;
    /// Registers what players carry. Its starting materials are checked once every mod has
    /// registered its blocks.
    fn register_inventory(&mut self, def: InventoryDef) -> Result<(), ApiError>;
    /// Registers the crafting table. Its block is checked once every mod has registered its
    /// blocks.
    fn register_crafting(&mut self, def: CraftingDef) -> Result<(), ApiError>;
    /// Registers how signals behave: without it, triggers do nothing.
    fn register_signals(&mut self, def: SignalDef) -> Result<(), ApiError>;
    /// Registers a model for content to name. Like looks, names are checked once all mods have
    /// registered, so a target may name a model another mod registers later.
    fn register_model(&mut self, def: ModelDef) -> Result<(), ApiError>;
    /// Registers a sound for content to name, checked the same way as models.
    fn register_sound(&mut self, def: SoundDef) -> Result<(), ApiError>;
    fn register_target(&mut self, def: TargetDef) -> Result<(), ApiError>;
    /// Registers a tool. Players can pick every registered tool, in registration order, ahead
    /// of the blocks they can place.
    fn register_tool(&mut self, def: ToolDef) -> Result<(), ApiError>;
}

/// Edits the world by block name.
pub trait WorldEdit {
    /// Fills the boxel with just this block, unturned, replacing whatever was there.
    fn set_block(&mut self, pos: BoxelPos, name: &str) -> Result<(), ApiError>;
    /// Adds a block to what's already in the boxel, turned by `orientation`. The same rules as
    /// a player placing it decide whether it may share the boxel.
    fn add_block(&mut self, pos: BoxelPos, name: &str, orientation: Orientation) -> Result<(), ApiError>;
    /// Adds a block of `material` in `shape`, turned by `orientation`, to what's already in the
    /// boxel: the same block crafting that shape makes. The same rules as a player placing it
    /// decide whether it may share the boxel. A full shape is the material's own block.
    fn add_shaped(
        &mut self,
        pos: BoxelPos,
        material: &str,
        shape: &ShapeGrid,
        orientation: Orientation,
    ) -> Result<(), ApiError>;
    /// Adds a bundle of `pieces`, each a material and the texels it fills, turned alike by
    /// `orientation`, to what's already in the boxel: the same bundle crafting those pieces
    /// makes, placed whole. The same rules as a player placing it decide whether it fits, and
    /// a piece that starts hidden is placed hidden.
    fn add_bundle(
        &mut self,
        pos: BoxelPos,
        pieces: &[(&str, ShapeGrid)],
        orientation: Orientation,
    ) -> Result<(), ApiError>;
    /// Empties the boxel.
    fn clear_block(&mut self, pos: BoxelPos);

    /// Where the player's feet start.
    fn set_spawn(&mut self, pos: WorldPos);

    /// A named place the debug teleport can jump to.
    fn add_waypoint(&mut self, name: &str, pos: WorldPos);

    /// Places a registered target with its feet at `pos`, facing `yaw` (radians counterclockwise
    /// from +X around +Z). That's its spot: it comes back there after breaking.
    fn add_target(&mut self, name: &str, pos: WorldPos, yaw: f32) -> Result<(), ApiError>;

    /// Places generated terrain. The engine fills it once every mod has built its world, so
    /// blocks set here are covered where the terrain reaches.
    fn add_terrain(&mut self, def: TerrainDef) -> Result<(), ApiError>;

    /// Fills the box from `min` to `max`, both inclusive.
    fn fill(&mut self, min: BoxelPos, max: BoxelPos, name: &str) -> Result<(), ApiError> {
        for z in min.0.z..=max.0.z {
            for y in min.0.y..=max.0.y {
                for x in min.0.x..=max.0.x {
                    self.set_block(BoxelPos::new(x, y, z), name)?;
                }
            }
        }
        Ok(())
    }

    /// Clears the box from `min` to `max`, both inclusive.
    fn clear(&mut self, min: BoxelPos, max: BoxelPos) {
        for z in min.0.z..=max.0.z {
            for y in min.0.y..=max.0.y {
                for x in min.0.x..=max.0.x {
                    self.clear_block(BoxelPos::new(x, y, z));
                }
            }
        }
    }
}
