//! The client's copy of the world, kept from the server's messages: the content, every chunk,
//! and the latest snapshot. Everything the client draws or asks about comes from here; the
//! client never sees the simulation itself.
//!
//! Two snapshots are kept: the latest, and the one shown. They're the same except during a
//! hit-stop, when the shown one holds still while the simulation carries on; when the freeze
//! ends the view snaps to the latest.

use std::sync::Mutex;

use api::{ChunkPos, HealthDef, InteractionDef, InventoryDef, ItemDropDef, ModelDef, SoundDef, TargetDef, ToolDef, WorldPos};
use bevy::prelude::*;
use protocol::{ClientMsg, Command, Connection, Defs, Event, ServerMsg, Snapshot, Welcome};
use sim::{
    Body, BlockRegistry, Craft, CraftPlan, CraftRefusal, Hit, Inventory, Item, LookRegistry, MaterialId, PlaceTarget,
    PlayerHurt, Slam, Strike, Swing, Target, TargetKind, ToolId, Universe, View,
};

#[derive(Resource)]
pub struct ClientWorld {
    looks: LookRegistry,
    models: Vec<ModelDef>,
    sounds: Vec<SoundDef>,
    blocks: BlockRegistry,
    defs: Defs,
    universe: Universe,
    spawn: WorldPos,
    waypoints: Vec<(String, WorldPos)>,
    latest: Snapshot,
    shown: Snapshot,
    /// Chunks the server changed since the last [`ClientWorld::take_changed_chunks`].
    changed: Vec<ChunkPos>,
    strikes: Vec<Strike>,
    slams: Vec<Slam>,
    hurts: Vec<PlayerHurt>,
    craft: Option<Result<(MaterialId, u32), CraftRefusal>>,
    /// Every event since the last [`ClientWorld::take_heard`], for sound.
    heard: Vec<Event>,
    /// The outcome of reading the sounds again, until [`ClientWorld::take_sound_reload`].
    sound_reload: Option<Result<(), String>>,
}

impl ClientWorld {
    pub fn new(welcome: Welcome) -> Self {
        let mut universe = Universe::default();
        for chunk in &welcome.chunks {
            universe.main_mut().import_chunk(chunk);
        }
        let mut world = Self {
            looks: welcome.content.looks,
            models: welcome.content.models,
            sounds: welcome.content.sounds,
            blocks: welcome.blocks,
            defs: welcome.defs,
            universe,
            spawn: welcome.spawn,
            waypoints: welcome.waypoints,
            latest: Snapshot::default(),
            shown: Snapshot::default(),
            changed: Vec::new(),
            strikes: Vec::new(),
            slams: Vec::new(),
            hurts: Vec::new(),
            craft: None,
            heard: Vec::new(),
            sound_reload: None,
        };
        world.take_snapshot(welcome.snapshot);
        world.show_latest();
        world
    }

    /// Takes in a message from the server.
    pub fn apply(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Welcome(welcome) => *self = Self::new(*welcome),
            ServerMsg::Tick(snapshot) => self.take_snapshot(*snapshot),
            ServerMsg::Chunk(chunk) => {
                self.universe.main_mut().import_chunk(&chunk);
                if !self.changed.contains(&chunk.pos) {
                    self.changed.push(chunk.pos);
                }
            }
            ServerMsg::Blocks(blocks) => self.blocks = *blocks,
            ServerMsg::Defs(defs) => self.defs = *defs,
            ServerMsg::Sounds(Ok(sounds)) => {
                self.sounds = sounds;
                self.sound_reload = Some(Ok(()));
            }
            ServerMsg::Sounds(Err(why)) => self.sound_reload = Some(Err(why)),
        }
    }

    fn take_snapshot(&mut self, mut snapshot: Snapshot) {
        for event in snapshot.events.drain(..) {
            self.heard.push(event.clone());
            match event {
                Event::Strike(s) => self.strikes.push(s),
                Event::Slam(s) => self.slams.push(s),
                Event::PlayerHurt(h) => self.hurts.push(h),
                Event::Craft(c) => self.craft = Some(c),
                Event::BlockEdit(_) => {}
            }
        }
        self.latest = snapshot;
    }

    /// Shows the latest snapshot. Not called during a hit-stop, so the view holds still.
    pub fn show_latest(&mut self) {
        if self.shown.tick != self.latest.tick || self.shown.player.is_none() {
            self.shown = self.latest.clone();
        }
    }

    /// A read-only look at the world as shown, for the questions in [`View`].
    pub fn view(&self) -> View<'_> {
        let defs = &self.defs;
        View {
            blocks: &self.blocks,
            grid: self.universe.main(),
            player: self.shown.player.as_ref(),
            targets: &self.shown.targets,
            target_defs: &defs.targets,
            interaction: defs.interaction.as_ref(),
            push_out: defs.push_out.as_ref(),
            crafting: defs.crafting.as_ref(),
            inventory: &self.shown.inventory,
            inventory_def: defs.inventory.as_ref(),
        }
    }

    pub fn target(&self, yaw: f32, pitch: f32) -> Option<Hit> {
        self.view().target(yaw, pitch)
    }

    pub fn placement(&self, hit: &Hit, block: sim::BlockId, yaw: f32, pitch: f32, precise: bool) -> Option<PlaceTarget> {
        self.view().placement(hit, block, yaw, pitch, precise)
    }

    pub fn is_table(&self, hit: &Hit) -> bool {
        self.view().is_table(hit)
    }

    pub fn craft_plan(&self, craft: &Craft) -> Result<CraftPlan, CraftRefusal> {
        self.view().craft_plan(craft)
    }

    pub fn capacity(&self) -> Option<u32> {
        self.view().capacity()
    }

    pub fn blocks(&self) -> &BlockRegistry {
        &self.blocks
    }

    pub fn looks(&self) -> &LookRegistry {
        &self.looks
    }

    pub fn models(&self) -> &[ModelDef] {
        &self.models
    }

    pub fn sounds(&self) -> &[SoundDef] {
        &self.sounds
    }

    pub fn defs(&self) -> &Defs {
        &self.defs
    }

    pub fn universe(&self) -> &Universe {
        &self.universe
    }

    pub fn spawn(&self) -> WorldPos {
        self.spawn
    }

    pub fn waypoints(&self) -> &[(String, WorldPos)] {
        &self.waypoints
    }

    /// Ticks run, as shown.
    pub fn tick(&self) -> u64 {
        self.shown.tick
    }

    pub fn player(&self) -> Option<&Body> {
        self.shown.player.as_ref()
    }

    pub fn health(&self) -> f32 {
        self.shown.health
    }

    pub fn targets(&self) -> &[Target] {
        &self.shown.targets
    }

    pub fn items(&self) -> &[Item] {
        &self.shown.items
    }

    pub fn inventory(&self) -> &Inventory {
        &self.shown.inventory
    }

    pub fn swing(&self) -> Option<&Swing> {
        self.shown.swing.as_ref()
    }

    pub fn guarding(&self) -> bool {
        self.shown.guarding
    }

    pub fn health_def(&self) -> Option<&HealthDef> {
        self.defs.health.as_ref()
    }

    pub fn interaction_def(&self) -> Option<&InteractionDef> {
        self.defs.interaction.as_ref()
    }

    pub fn item_drop_def(&self) -> Option<&ItemDropDef> {
        self.defs.item_drops.as_ref()
    }

    pub fn inventory_def(&self) -> Option<&InventoryDef> {
        self.defs.inventory.as_ref()
    }

    pub fn target_def(&self, kind: TargetKind) -> &TargetDef {
        &self.defs.targets[kind.index()]
    }

    pub fn tools(&self) -> &[ToolDef] {
        &self.defs.tools
    }

    pub fn tool(&self, id: ToolId) -> &ToolDef {
        &self.defs.tools[id.index()]
    }

    pub fn tool_ids(&self) -> impl Iterator<Item = ToolId> + use<> {
        ToolId::first(self.defs.tools.len())
    }

    /// Chunks the server changed since the last call.
    pub fn take_changed_chunks(&mut self) -> Vec<ChunkPos> {
        std::mem::take(&mut self.changed)
    }

    /// Tool strikes since the last call, oldest first.
    pub fn take_strikes(&mut self) -> Vec<Strike> {
        std::mem::take(&mut self.strikes)
    }

    /// Creatures slammed into walls since the last call, oldest first.
    pub fn take_slams(&mut self) -> Vec<Slam> {
        std::mem::take(&mut self.slams)
    }

    /// Blows and falls that hurt the player since the last call, oldest first.
    pub fn take_player_hurts(&mut self) -> Vec<PlayerHurt> {
        std::mem::take(&mut self.hurts)
    }

    /// Whether the sounds were read again since the last call: they're new in
    /// [`ClientWorld::sounds`], or here's why not.
    pub fn take_sound_reload(&mut self) -> Option<Result<(), String>> {
        self.sound_reload.take()
    }

    /// Every event since the last call, oldest first, for sound to hear.
    pub fn take_heard(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.heard)
    }

    /// The outcome of the last craft since the last call.
    pub fn take_craft(&mut self) -> Option<Result<(MaterialId, u32), CraftRefusal>> {
        self.craft.take()
    }
}

/// The client's link to the server.
#[derive(Resource)]
pub struct Link(Mutex<Box<dyn Connection>>);

impl Link {
    pub fn new(connection: Box<dyn Connection>) -> Self {
        Self(Mutex::new(connection))
    }

    fn connection(&mut self) -> &mut dyn Connection {
        &mut **self.0.get_mut().unwrap_or_else(|e| e.into_inner())
    }

    /// Sends a message, then takes in whatever has come back.
    pub fn send(&mut self, msg: ClientMsg, world: &mut ClientWorld) {
        self.connection().send(msg);
        self.receive(world);
    }

    pub fn command(&mut self, command: Command, world: &mut ClientWorld) {
        self.send(ClientMsg::Command(command), world);
    }

    /// Takes in every message that has arrived.
    pub fn receive(&mut self, world: &mut ClientWorld) {
        for msg in self.connection().receive() {
            world.apply(msg);
        }
    }

    /// Waits for the server's welcome and builds the world from it. Messages before it are
    /// dropped; there shouldn't be any.
    pub fn connect(&mut self) -> Option<ClientWorld> {
        loop {
            let msgs = self.connection().receive();
            let mut world: Option<ClientWorld> = None;
            for msg in msgs {
                match (&mut world, msg) {
                    (_, ServerMsg::Welcome(w)) => world = Some(ClientWorld::new(*w)),
                    (Some(world), msg) => world.apply(msg),
                    (None, _) => {}
                }
            }
            if world.is_some() {
                return world;
            }
            std::thread::yield_now();
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! A client world on a local server, for tests.
    #![allow(clippy::disallowed_types)]

    use super::*;
    use server::{LocalConnection, Server};
    use sim::Simulation;

    /// The base game's simulation, with its native movement controller and no player yet.
    pub fn base_sim() -> Simulation {
        let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
        Simulation::with_mods(&[&data, &base::BaseMod, &base_cow::BaseCowMod, &base_movement::BaseMovementMod]).unwrap()
    }

    /// A client connected to a local server running `sim`.
    pub fn connect(sim: Simulation) -> (Link, ClientWorld) {
        let mut link = Link::new(Box::new(LocalConnection::new(Server::new(sim))));
        let world = link.connect().expect("a local server welcomes at once");
        (link, world)
    }

    /// Runs one tick with `input` and shows it.
    pub fn step(link: &mut Link, world: &mut ClientWorld, input: &sim::PlayerInput) {
        link.send(ClientMsg::Input(Box::new(input.clone())), world);
        world.show_latest();
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use sim::PlayerInput;

    #[test]
    fn a_broken_block_shows_in_the_copy_and_is_listed_for_remeshing() {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        let (mut link, mut world) = connect(sim);
        // Look straight down at the floor.
        let down = -89f32.to_radians();
        let hit = world.target(0.0, down).expect("the floor is in reach");
        let before = world.universe().main().cell(hit.pos).len();
        world.take_changed_chunks();
        step(&mut link, &mut world, &PlayerInput { pitch: down, use_pressed: true, ..Default::default() });
        assert_eq!(world.universe().main().cell(hit.pos).len(), before - 1);
        assert!(world.take_changed_chunks().contains(&hit.pos.split().0));
        assert!(world.take_changed_chunks().is_empty(), "each change is listed once");
    }

    #[test]
    fn the_shown_snapshot_holds_until_shown() {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        let (mut link, mut world) = connect(sim);
        let start = world.tick();
        link.send(ClientMsg::Input(Box::default()), &mut world);
        assert_eq!(world.tick(), start, "held, as in a hit-stop");
        world.show_latest();
        assert_eq!(world.tick(), start + 1);
    }

    #[test]
    fn a_tuning_command_comes_back_in_the_defs() {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        let (mut link, mut world) = connect(sim);
        let reach = world.interaction_def().unwrap().reach;
        let command = Command::Tune { section: "interaction".into(), name: "reach".into(), value: reach + 1.0 };
        link.command(command, &mut world);
        assert_eq!(world.interaction_def().unwrap().reach, reach + 1.0);
        let section = world.defs().tuning.iter().find(|s| s.name == "interaction").unwrap();
        assert_eq!(section.values.iter().find(|t| t.name == "reach").unwrap().value, reach + 1.0);
    }
}
