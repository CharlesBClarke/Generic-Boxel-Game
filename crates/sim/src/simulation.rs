//! The simulation: loads mods, owns the world and advances it at a fixed rate.

use std::fmt;

use api::{ApiError, BlockDef, BoxelPos, Mod, PlayerDef, Registrar, Vec3, WorldEdit, WorldPos};

use crate::player::Terrain;
use crate::{BlockRegistry, Grid, Player, PlayerInput, Universe};

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
    player_def: Option<PlayerDef>,
    universe: Universe,
    spawn: WorldPos,
    waypoints: Vec<(String, WorldPos)>,
    player: Option<Player>,
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
            m.register(&mut ContentRegistrar { blocks: &mut sim.blocks, player: &mut sim.player_def })?;
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

    /// Puts the player at the spawn point. Needs a mod to have registered a player definition.
    pub fn spawn_player(&mut self) -> Result<(), SimError> {
        let def = self.player_def.as_ref().ok_or(SimError::NoPlayerDef)?;
        self.player = Some(Player::new(def, self.spawn));
        Ok(())
    }

    /// Advances one fixed tick.
    pub fn step(&mut self, input: &PlayerInput) {
        let Self { blocks, universe, player, .. } = self;
        if let Some(player) = player {
            let terrain = GridTerrain { grid: universe.main(), blocks };
            player.step(input, Self::TICK_SECONDS, &terrain);
        }
        self.tick += 1;
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

struct ContentRegistrar<'a> {
    blocks: &'a mut BlockRegistry,
    player: &'a mut Option<PlayerDef>,
}

impl Registrar for ContentRegistrar<'_> {
    fn register_block(&mut self, def: BlockDef) -> Result<(), ApiError> {
        self.blocks.register(def).map(|_| ())
    }

    fn register_player(&mut self, def: PlayerDef) -> Result<(), ApiError> {
        if self.player.is_some() {
            return Err(ApiError::DuplicatePlayerDef);
        }
        def.validate().map_err(ApiError::InvalidDef)?;
        *self.player = Some(def);
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
