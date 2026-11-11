//! Messages between the client and the server. The client sends its input every tick and
//! commands now and then; the server sends the world once, then what changes: chunks, blocks
//! crafting added, definitions tuning changed, and a snapshot of every tick with its events.
//!
//! Every message is owned plain data with no references into the simulation, so the server can
//! run anywhere: on the client's thread, on another thread, and later on another machine.

use api::{
    CraftingDef, CrowdDef, HealthDef, InteractionDef, InventoryDef, ItemDropDef, ModelDef, PlayerDef, PushOutDef,
    SoundDef, TargetDef, ToolDef, Tunable, WorldPos,
};
use sim::{
    Blast, BlockEdit, Clock, BlueprintId, BlueprintRefusal, Body, BlockRegistry, ChunkData, Library, CraftRefusal, GenerationStats, Inventory, Item, LookRegistry, MaterialId, PlayerHurt,
    PlayerInput, Slam, Strike, Swing, Target,
};

/// Ticks per second.
pub const TICK_RATE: f64 = sim::Simulation::TICK_RATE;
/// Seconds per tick.
pub const TICK_SECONDS: f32 = sim::Simulation::TICK_SECONDS;

/// From the client to the server.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientMsg {
    /// The player's input for the next tick. The server runs one tick for each.
    Input(Box<PlayerInput>),
    Command(Command),
}

/// Something the client asks for outside play: tuning and debugging.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Sets one value in one of the server's tuning sections (see [`Defs::tuning`]).
    Tune { section: String, name: String, value: f32 },
    /// Fills the player's health.
    Heal,
    /// With it on, running out of health doesn't send the player back.
    Unkillable(bool),
    /// Off, every creature is a dummy that only moves when hit.
    BrainsOn(bool),
    /// Sets the time of day, in hours (wrapped into 0 to 24).
    SetTime(f64),
    /// Stops the time of day where it is, or lets it run again.
    PauseTime(bool),
    TeleportPlayer(WorldPos),
    /// Puts every creature back at its spot, whole.
    RespawnTargets,
    /// Generates the terrain again with the current tuning (the `world` section and each
    /// generator's). The new chunks come with the next tick.
    RegenerateTerrain,
    /// Reads the sounds again, for trying sound files without a restart. The server answers
    /// with [`ServerMsg::Sounds`].
    ReloadSounds,
    /// Saves the world to the server's world file. The server answers with [`ServerMsg::World`].
    SaveWorld,
    /// Puts the world back as it was last saved. The server answers with [`ServerMsg::World`];
    /// the changed blocks and chunks come with the next tick.
    LoadWorld,
    /// Saves the world under another name, and plays on in that one. Answered with
    /// [`ServerMsg::World`].
    SaveWorldAs(String),
    /// Saves the world, then plays the world `name` instead: as it was saved, or fresh if it
    /// hasn't been. Answered with [`ServerMsg::World`].
    OpenWorld(String),
    /// Asks for [`ServerMsg::Worlds`].
    ListWorlds,
}

/// From the server to the client.
pub enum ServerMsg {
    /// Everything, once, first.
    Welcome(Box<Welcome>),
    /// One tick ran.
    Tick(Box<Snapshot>),
    /// A chunk changed, or a neighbor's border did. Comes before the tick it changed in.
    Chunk(ChunkData),
    /// Blocks were added, by crafting a new shape. Comes before the tick they were added in.
    Blocks(Box<BlockRegistry>),
    /// The player's blueprints changed, by an edit or a load. Comes before the tick they
    /// changed in.
    Blueprints(Box<Library>),
    /// Definitions or tuning changed, by a command.
    Defs(Box<Defs>),
    /// Every sound after [`Command::ReloadSounds`], or why reading them again failed (the old
    /// ones stay).
    Sounds(Result<Vec<SoundDef>, String>),
    /// The world was saved or loaded, or that failed.
    World(WorldNote),
    /// Every saved world's name, last saved first.
    Worlds(Vec<String>),
}

/// How saving or loading the world went.
#[derive(Clone, Debug, PartialEq)]
pub enum WorldNote {
    Saved { name: String },
    /// Loaded, with where the player looked when it was saved, and anything a missing mod named
    /// that was left out. A fresh world is loaded too, from how the game started.
    Loaded { name: String, look: (f32, f32), skipped: Vec<String>, fresh: bool },
    Failed(String),
}

/// What a client needs to start: the content, the world and where things stand.
pub struct Welcome {
    pub content: Content,
    pub blocks: BlockRegistry,
    /// The player's blueprints.
    pub blueprints: Library,
    pub defs: Defs,
    /// Every chunk in the world.
    pub chunks: Vec<ChunkData>,
    pub spawn: WorldPos,
    /// Named spots in the world, in the order the mods made them.
    pub waypoints: Vec<(String, WorldPos)>,
    pub snapshot: Snapshot,
    /// The world came from a save: where the player looked when it was saved. Its terrain
    /// values are the save's, so a client shouldn't generate the terrain again with its own.
    pub loaded_look: Option<(f32, f32)>,
    /// The world being played, if the server saves worlds.
    pub world: Option<String>,
}

/// Content that never changes while the game runs.
#[derive(Clone)]
pub struct Content {
    pub looks: LookRegistry,
    pub models: Vec<ModelDef>,
    pub sounds: Vec<SoundDef>,
}

/// The definitions mods registered, as tuned so far, and the server's tuning.
#[derive(Clone, Default)]
pub struct Defs {
    pub player: Option<PlayerDef>,
    pub interaction: Option<InteractionDef>,
    pub push_out: Option<PushOutDef>,
    pub crowd: Option<CrowdDef>,
    pub health: Option<HealthDef>,
    pub item_drops: Option<ItemDropDef>,
    pub inventory: Option<InventoryDef>,
    pub crafting: Option<CraftingDef>,
    /// By [`sim::TargetKind`] index.
    pub targets: Vec<TargetDef>,
    /// By [`sim::ToolId`] index.
    pub tools: Vec<ToolDef>,
    /// Every value the server lets a client tune, by section, in the order the panel shows them.
    pub tuning: Vec<TuningSection>,
    pub brains_on: bool,
    pub unkillable: bool,
    /// How the last terrain generation went, or why it failed.
    pub generation: Option<Result<GenerationStats, String>>,
}

/// One tuning section: the movement controller, a definition, a tool, a target or a brain.
#[derive(Clone, Debug, PartialEq)]
pub struct TuningSection {
    /// Its name in the tuning file, such as `"movement"` or `"brain base:cow"`.
    pub name: String,
    pub values: Vec<Tunable>,
}

/// Where things stand after a tick, and what happened in it.
#[derive(Clone, Default)]
pub struct Snapshot {
    /// Ticks run so far.
    pub tick: u64,
    /// The time of day, how fast it runs and whether it's paused.
    pub clock: Clock,
    pub player: Option<Body>,
    pub health: f32,
    /// Live creatures, broken ones excluded.
    pub targets: Vec<Target>,
    pub items: Vec<Item>,
    pub inventory: Inventory,
    pub swing: Option<Swing>,
    pub guarding: bool,
    /// What happened during the tick, in order.
    pub events: Vec<Event>,
}

/// Something that happened during a tick.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Strike(Strike),
    Slam(Slam),
    PlayerHurt(PlayerHurt),
    /// A block the player broke or placed.
    BlockEdit(BlockEdit),
    /// A crystal blast's beam.
    Blast(Blast),
    /// A craft the input asked for: the part made and how many, or why nothing was.
    Craft(Result<(MaterialId, u32), CraftRefusal>),
    /// An edit to the blueprints the input asked for: the blueprint made or changed, or why
    /// nothing was.
    Blueprint(Result<BlueprintId, BlueprintRefusal>),
}

/// A client's end of its link to a server.
pub trait Connection: Send {
    fn send(&mut self, msg: ClientMsg);
    /// Every message that has arrived since the last call, oldest first.
    fn receive(&mut self) -> Vec<ServerMsg>;
}
