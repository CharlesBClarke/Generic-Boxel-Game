//! The simulation: loads mods, owns the world and advances it at a fixed rate.

use std::collections::HashMap;
use std::fmt;

use api::block::is_valid_name;
use api::{
    ApiError, BlockDef, BoxelPos, ChunkPos, InteractionDef, Mod, MovementController, PlayerDef, PushOutDef, Registrar,
    Vec3, WorldEdit, WorldPos, look_direction,
};

use crate::edit::{ChangedChunks, Repeater};
use crate::player::{PushOut, Terrain};
use crate::{BlockId, BlockRegistry, Grid, Hit, Player, PlayerInput, Universe, raycast};

#[derive(Debug, PartialEq)]
pub enum SimError {
    Api(ApiError),
    /// No mod registered a player definition.
    NoPlayerDef,
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SimError::Api(e) => write!(f, "{e}"),
            SimError::NoPlayerDef => write!(f, "no mod registered a player definition"),
        }
    }
}

impl std::error::Error for SimError {}

impl From<ApiError> for SimError {
    fn from(e: ApiError) -> Self {
        SimError::Api(e)
    }
}

#[derive(Default)]
pub struct Simulation {
    blocks: BlockRegistry,
    /// Movement controllers by name, as registered. Each body gets its own from `spawn`.
    controllers: HashMap<String, Box<dyn MovementController>>,
    player_def: Option<PlayerDef>,
    interaction_def: Option<InteractionDef>,
    push_out_def: Option<PushOutDef>,
    universe: Universe,
    spawn: WorldPos,
    waypoints: Vec<(String, WorldPos)>,
    player: Option<Player>,
    breaking: Repeater,
    placing: Repeater,
    changed: ChangedChunks,
    tick: u64,
}

impl Simulation {
    /// Ticks per second.
    pub const TICK_RATE: f64 = 60.0;
    /// Seconds per tick.
    pub const TICK_SECONDS: f32 = 1.0 / Self::TICK_RATE as f32;

    /// An empty simulation with no content: no blocks, no player definition.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers every mod's content, then lets every mod build the world.
    pub fn with_mods(mods: &[&dyn Mod]) -> Result<Self, SimError> {
        let mut sim = Self::new();
        for m in mods {
            m.register(&mut ContentRegistrar {
                blocks: &mut sim.blocks,
                controllers: &mut sim.controllers,
                player: &mut sim.player_def,
                interaction: &mut sim.interaction_def,
                push_out: &mut sim.push_out_def,
            })?;
        }
        for m in mods {
            m.build_world(&mut WorldEditor {
                blocks: &sim.blocks,
                grid: sim.universe.main_mut(),
                spawn: &mut sim.spawn,
                waypoints: &mut sim.waypoints,
            })?;
        }
        Ok(sim)
    }

    /// Adds a movement controller the engine loaded itself, such as one from a WebAssembly
    /// component, as if a mod had registered it.
    pub fn add_movement_controller(
        &mut self,
        name: &str,
        controller: Box<dyn MovementController>,
    ) -> Result<(), ApiError> {
        add_controller(&mut self.controllers, name, controller)
    }

    /// Puts the player at the spawn point. Needs a mod to have registered a player definition,
    /// and the movement controller it names.
    pub fn spawn_player(&mut self) -> Result<(), SimError> {
        let def = self.player_def.as_ref().ok_or(SimError::NoPlayerDef)?;
        let controller = self
            .controllers
            .get(&def.controller)
            .ok_or_else(|| ApiError::UnknownController(def.controller.clone()))?;
        self.player = Some(Player::new(def, controller.spawn(), self.spawn));
        Ok(())
    }

    /// Advances one fixed tick. Block edits happen first, from where the player stood at the
    /// end of the last tick, which is what the client aimed from.
    pub fn step(&mut self, input: &PlayerInput) {
        self.edit_blocks(input);
        let Self { blocks, universe, player, .. } = self;
        if let Some(player) = player {
            let terrain = GridTerrain { grid: universe.main(), blocks };
            player.step(input, Self::TICK_SECONDS, &terrain);
        }
        self.tick += 1;
    }

    /// The block the player would break looking along `yaw` and `pitch`, and the face a
    /// placed block would go on. `None` with nothing in reach, no player, or no mod defining
    /// interaction.
    pub fn target(&self, yaw: f32, pitch: f32) -> Option<Hit> {
        let player = self.player.as_ref()?;
        let reach = self.interaction_def.as_ref()?.reach;
        let grid = self.universe.main();
        raycast(player.eye(), look_direction(yaw, pitch), reach, |p| !grid.block(p).is_empty())
    }

    /// Where `block` would go if placed on `hit`'s face, or `None` if that isn't allowed: the
    /// block isn't placeable, the hit has no face, or the cell is filled. A solid block inside
    /// the player is only allowed if a push-out rule is registered and there's room to push
    /// the player out.
    pub fn placement(&self, hit: &Hit, block: BlockId) -> Option<BoxelPos> {
        let placeable = self.blocks.get(block).is_some_and(|def| def.placeable);
        let pos = hit.adjacent();
        let free = hit.normal != api::I64Vec3::ZERO && self.universe.main().block(pos).is_empty();
        if !(placeable && free) {
            return None;
        }
        match &self.player {
            Some(player) if self.blocks.is_solid(block) && player.overlaps(pos) => {
                self.push_out(player, pos).map(|_| pos)
            }
            _ => Some(pos),
        }
    }

    /// How the player would get out of a solid block at `pos`, if there's a rule and room.
    fn push_out(&self, player: &Player, pos: BoxelPos) -> Option<PushOut> {
        let def = self.push_out_def.as_ref()?;
        let terrain = WithSolid { inner: GridTerrain { grid: self.universe.main(), blocks: &self.blocks }, pos };
        player.push_out(pos, def.upward_bias, &terrain)
    }

    /// Chunks whose blocks changed since the last call, including neighbors whose border faces
    /// changed. World building at load time isn't included.
    pub fn take_changed_chunks(&mut self) -> Vec<ChunkPos> {
        self.changed.take()
    }

    /// Breaks or places a block if the input asks for it this tick. Breaking wins when both
    /// buttons act on the same tick.
    fn edit_blocks(&mut self, input: &PlayerInput) {
        let Some(def) = &self.interaction_def else {
            return;
        };
        let interval = 1.0 / def.repeat_rate;
        let dt = Self::TICK_SECONDS;
        let breaking = self.breaking.update(input.break_pressed, input.break_held, interval, dt);
        let placing = self.placing.update(input.place_pressed, input.place_held, interval, dt);
        if !breaking && !placing {
            return;
        }
        let Some(hit) = self.target(input.yaw, input.pitch) else {
            return;
        };
        if breaking {
            self.set_block(hit.pos, BlockId::EMPTY);
        } else if let Some(pos) = self.placement(&hit, input.place_block) {
            self.set_block(pos, input.place_block);
        }
    }

    /// Sets a block. A solid block appearing inside the player pushes it out, if there's a
    /// rule and room.
    fn set_block(&mut self, pos: BoxelPos, id: BlockId) {
        let push = match (&self.player, &self.push_out_def) {
            (Some(player), Some(def)) if self.blocks.is_solid(id) => {
                self.push_out(player, pos).map(|push| (push, def.strength))
            }
            _ => None,
        };
        self.universe.main_mut().set_block(pos, id);
        self.changed.mark(pos);
        if let (Some((push, strength)), Some(player)) = (push, &mut self.player) {
            player.apply_push_out(push, strength);
        }
    }

    pub fn teleport_player(&mut self, pos: WorldPos) {
        if let Some(player) = &mut self.player {
            player.teleport(pos);
        }
    }

    /// Queues an instant velocity change for the player's next tick.
    pub fn apply_player_impulse(&mut self, impulse: Vec3) {
        if let Some(player) = &mut self.player {
            player.apply_impulse(impulse);
        }
    }

    pub fn blocks(&self) -> &BlockRegistry {
        &self.blocks
    }

    pub fn player_def(&self) -> Option<&PlayerDef> {
        self.player_def.as_ref()
    }

    pub fn interaction_def(&self) -> Option<&InteractionDef> {
        self.interaction_def.as_ref()
    }

    /// For engine tools such as the tuning panel.
    pub fn interaction_def_mut(&mut self) -> Option<&mut InteractionDef> {
        self.interaction_def.as_mut()
    }

    pub fn push_out_def(&self) -> Option<&PushOutDef> {
        self.push_out_def.as_ref()
    }

    /// For engine tools such as the tuning panel.
    pub fn push_out_def_mut(&mut self) -> Option<&mut PushOutDef> {
        self.push_out_def.as_mut()
    }

    pub fn universe(&self) -> &Universe {
        &self.universe
    }

    pub fn player(&self) -> Option<&Player> {
        self.player.as_ref()
    }

    /// For engine tools such as the tuning panel.
    pub fn player_mut(&mut self) -> Option<&mut Player> {
        self.player.as_mut()
    }

    pub fn spawn(&self) -> WorldPos {
        self.spawn
    }

    pub fn waypoints(&self) -> &[(String, WorldPos)] {
        &self.waypoints
    }

    /// Ticks run so far.
    pub fn tick(&self) -> u64 {
        self.tick
    }
}

struct GridTerrain<'a> {
    grid: &'a Grid,
    blocks: &'a BlockRegistry,
}

impl Terrain for GridTerrain<'_> {
    fn solid(&self, pos: BoxelPos) -> bool {
        self.blocks.is_solid(self.grid.block(pos))
    }

    fn friction(&self, pos: BoxelPos) -> f32 {
        self.blocks.get(self.grid.block(pos)).map_or(1.0, |def| def.friction)
    }
}

/// Terrain with one extra solid boxel, for checking a block that hasn't been placed yet.
struct WithSolid<T> {
    inner: T,
    pos: BoxelPos,
}

impl<T: Terrain> Terrain for WithSolid<T> {
    fn solid(&self, pos: BoxelPos) -> bool {
        pos == self.pos || self.inner.solid(pos)
    }

    fn friction(&self, pos: BoxelPos) -> f32 {
        self.inner.friction(pos)
    }
}

fn add_controller(
    controllers: &mut HashMap<String, Box<dyn MovementController>>,
    name: &str,
    controller: Box<dyn MovementController>,
) -> Result<(), ApiError> {
    if !is_valid_name(name) {
        return Err(ApiError::InvalidName(name.into()));
    }
    if controllers.contains_key(name) {
        return Err(ApiError::DuplicateName(name.into()));
    }
    controllers.insert(name.into(), controller);
    Ok(())
}

struct ContentRegistrar<'a> {
    blocks: &'a mut BlockRegistry,
    controllers: &'a mut HashMap<String, Box<dyn MovementController>>,
    player: &'a mut Option<PlayerDef>,
    interaction: &'a mut Option<InteractionDef>,
    push_out: &'a mut Option<PushOutDef>,
}

impl Registrar for ContentRegistrar<'_> {
    fn register_block(&mut self, def: BlockDef) -> Result<(), ApiError> {
        self.blocks.register(def).map(|_| ())
    }

    fn register_movement_controller(
        &mut self,
        name: &str,
        controller: Box<dyn MovementController>,
    ) -> Result<(), ApiError> {
        add_controller(self.controllers, name, controller)
    }

    fn register_player(&mut self, def: PlayerDef) -> Result<(), ApiError> {
        if self.player.is_some() {
            return Err(ApiError::DuplicatePlayerDef);
        }
        def.validate().map_err(ApiError::InvalidDef)?;
        *self.player = Some(def);
        Ok(())
    }

    fn register_interaction(&mut self, def: InteractionDef) -> Result<(), ApiError> {
        if self.interaction.is_some() {
            return Err(ApiError::DuplicateInteractionDef);
        }
        def.validate().map_err(ApiError::InvalidDef)?;
        *self.interaction = Some(def);
        Ok(())
    }

    fn register_push_out(&mut self, def: PushOutDef) -> Result<(), ApiError> {
        if self.push_out.is_some() {
            return Err(ApiError::DuplicatePushOutDef);
        }
        def.validate().map_err(ApiError::InvalidDef)?;
        *self.push_out = Some(def);
        Ok(())
    }
}

struct WorldEditor<'a> {
    blocks: &'a BlockRegistry,
    grid: &'a mut Grid,
    spawn: &'a mut WorldPos,
    waypoints: &'a mut Vec<(String, WorldPos)>,
}

impl WorldEdit for WorldEditor<'_> {
    fn set_block(&mut self, pos: BoxelPos, name: &str) -> Result<(), ApiError> {
        let id = self.blocks.id(name).ok_or_else(|| ApiError::UnknownBlock(name.into()))?;
        self.grid.set_block(pos, id);
        Ok(())
    }

    fn clear_block(&mut self, pos: BoxelPos) {
        self.grid.set_block(pos, crate::BlockId::EMPTY);
    }

    fn set_spawn(&mut self, pos: WorldPos) {
        *self.spawn = pos;
    }

    fn add_waypoint(&mut self, name: &str, pos: WorldPos) {
        self.waypoints.push((name.into(), pos));
    }
}
