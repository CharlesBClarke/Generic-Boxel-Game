//! The traits a mod implements and the engine services it is handed.

use crate::{
    ApiError, BlockDef, BoxelPos, InteractionDef, ItemDropDef, MovementController, Orientation, PlayerDef, PushOutDef, LookDef,
    WorldPos,
};

/// A mod. The engine calls `register` on every mod first, then `build_world` on every mod.
pub trait Mod {
    /// The mod's namespace, such as `"base"`.
    fn name(&self) -> &str;

    /// Registers content: looks, blocks, movement controllers, the player, interaction, push-out
    /// and item drop definitions.
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
    fn register_player(&mut self, def: PlayerDef) -> Result<(), ApiError>;
    fn register_interaction(&mut self, def: InteractionDef) -> Result<(), ApiError>;
    fn register_push_out(&mut self, def: PushOutDef) -> Result<(), ApiError>;
    fn register_item_drops(&mut self, def: ItemDropDef) -> Result<(), ApiError>;
}

/// Edits the world by block name.
pub trait WorldEdit {
    /// Fills the boxel with just this block, unturned, replacing whatever was there.
    fn set_block(&mut self, pos: BoxelPos, name: &str) -> Result<(), ApiError>;
    /// Adds a block to what's already in the boxel, turned by `orientation`. The same rules as
    /// a player placing it decide whether it may share the boxel.
    fn add_block(&mut self, pos: BoxelPos, name: &str, orientation: Orientation) -> Result<(), ApiError>;
    /// Empties the boxel.
    fn clear_block(&mut self, pos: BoxelPos);

    /// Where the player's feet start.
    fn set_spawn(&mut self, pos: WorldPos);

    /// A named place the debug teleport can jump to.
    fn add_waypoint(&mut self, name: &str, pos: WorldPos);

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
