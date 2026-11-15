//! The server: owns the simulation and talks to clients only through [`protocol`] messages.
//! Single-player runs one in the same process (see [`LocalConnection`]), so there is one code
//! path whether or not the server is far away.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use api::{Tunable, Tunables};
use protocol::{ClientMsg, Command, Connection, Content, Defs, Event, ServerMsg, Snapshot, TuningSection, Welcome, WorldNote};
use sim::{ChunkStore, PlayerInput, SaveData, Simulation};

pub mod chunk_files;
pub mod worlds;

pub use chunk_files::ChunkFiles;
pub use worlds::{DEFAULT_WORLD, WorldFile};

/// Reads the sounds again from wherever the mods came from.
pub type SoundSource = Box<dyn FnMut() -> Result<Vec<api::SoundDef>, String> + Send>;

pub struct Server {
    sim: Simulation,
    /// How many blocks the client has heard of, so crafting a new shape sends the registry.
    blocks_sent: usize,
    /// The blueprints' revision the client has, so a change sends them.
    blueprints_sent: u64,
    /// Where [`Command::ReloadSounds`] reads from; without one it can't.
    sound_source: Option<SoundSource>,
    /// Why the last regeneration failed, if it did.
    generation_error: Option<String>,
    /// Where the world is saved and loaded from; without one it can't be.
    world: Option<WorldFile>,
    /// The world was loaded from its file when the server started.
    loaded: bool,
    /// The world as the game built it, before any save: what a fresh world starts from.
    fresh: Option<SaveData>,
    /// Every chunk the client has, so it hears of changes only to those, and of every chunk
    /// streaming brings in or drops.
    has: HashSet<api::ChunkPos>,
    /// Minutes between autosaves while playing; 0 for none. Tuned in the `saving` section.
    autosave_minutes: f32,
    /// Ticks since the world was last saved or loaded.
    since_saved: u64,
}

/// Minutes between autosaves unless tuned.
pub const AUTOSAVE_MINUTES: f32 = 5.0;

impl Server {
    /// A server running `sim`, which must already have its mods loaded and its player spawned.
    pub fn new(sim: Simulation) -> Self {
        Self {
            sim,
            blocks_sent: 0,
            blueprints_sent: 0,
            sound_source: None,
            generation_error: None,
            world: None,
            loaded: false,
            fresh: None,
            has: HashSet::new(),
            autosave_minutes: AUTOSAVE_MINUTES,
            since_saved: 0,
        }
    }

    /// Saves and loads the world at `file`. `loaded` says `sim` was loaded from it already.
    /// The simulation must already keep its edited terrain in the world's chunk store (see
    /// [`WorldFile::chunk_files`]), as `game::load_world` sets it up, since terrain loads from
    /// the store before the server exists.
    pub fn with_world(mut self, file: WorldFile, loaded: bool) -> Self {
        self.world = Some(file);
        self.loaded = loaded;
        self
    }

    /// Lets clients have the sounds read again from `source` while the game runs.
    pub fn with_sound_source(mut self, source: SoundSource) -> Self {
        self.sound_source = Some(source);
        self
    }

    /// Lets clients start fresh worlds from `fresh`, the world as the game built it.
    pub fn with_fresh(mut self, fresh: SaveData) -> Self {
        self.fresh = Some(fresh);
        self
    }

    /// Everything a client needs to start.
    pub fn welcome(&mut self) -> Welcome {
        // Changes, loads and drops so far are in the chunks sent now.
        self.sim.take_changed_chunks();
        self.sim.take_loaded_chunks();
        self.sim.take_unloaded_chunks();
        self.sim.take_air_chunks();
        self.sim.take_tops();
        // Far terrain comes after, built afresh for this client.
        self.sim.reset_far();
        self.sim.take_far_unloaded();
        self.blocks_sent = self.sim.blocks().len();
        self.blueprints_sent = self.sim.blueprints_revision();
        let grid = self.sim.universe().main();
        self.has = grid.chunks().map(|(pos, _)| pos).collect();
        let chunks = grid.chunks().map(|(pos, _)| grid.export_chunk(pos)).collect();
        let tops = grid.tops.clone();
        let snapshot = self.snapshot(Vec::new());
        Welcome {
            content: Content {
                looks: self.sim.looks().clone(),
                models: self.sim.models().to_vec(),
                sounds: self.sim.sounds().to_vec(),
            },
            blocks: self.sim.blocks().clone(),
            blueprints: self.sim.blueprints().clone(),
            defs: self.defs(),
            chunks,
            tops,
            air: self.sim.air_chunks().collect(),
            spawn: self.sim.spawn(),
            waypoints: self.sim.waypoints().to_vec(),
            snapshot,
            loaded_look: self.loaded.then(|| self.sim.look()),
            world: self.world.as_ref().map(|w| w.name.clone()),
        }
    }

    /// Acts on a message from the client, and returns what to send back.
    pub fn handle(&mut self, msg: ClientMsg) -> Vec<ServerMsg> {
        match msg {
            ClientMsg::Input(input) => self.tick(&input),
            ClientMsg::Command(Command::ReloadSounds) => vec![ServerMsg::Sounds(self.reload_sounds())],
            ClientMsg::Command(Command::SaveWorld) => vec![ServerMsg::World(self.save_world(false))],
            ClientMsg::Command(Command::LoadWorld) => vec![ServerMsg::World(self.load_world())],
            ClientMsg::Command(Command::SaveWorldAs(name)) => vec![ServerMsg::World(self.save_world_as(&name))],
            ClientMsg::Command(Command::OpenWorld(name)) => vec![ServerMsg::World(self.open_world(&name))],
            ClientMsg::Command(Command::ListWorlds) => {
                vec![ServerMsg::Worlds(self.world.as_ref().map_or_else(Vec::new, WorldFile::saved_worlds))]
            }
            ClientMsg::Command(command) => {
                self.command(command);
                vec![ServerMsg::Defs(Box::new(self.defs()))]
            }
        }
    }

    /// Runs one tick. Sends new blocks first, then changed chunks, then the snapshot.
    fn tick(&mut self, input: &PlayerInput) -> Vec<ServerMsg> {
        self.sim.step(input);
        let mut out = Vec::new();
        self.since_saved += 1;
        let every = (self.autosave_minutes as f64 * 60.0 * Simulation::TICK_RATE) as u64;
        if self.world.is_some() && every > 0 && self.since_saved >= every {
            out.push(ServerMsg::World(self.save_world(true)));
        }
        if self.sim.blocks().len() != self.blocks_sent {
            self.blocks_sent = self.sim.blocks().len();
            out.push(ServerMsg::Blocks(Box::new(self.sim.blocks().clone())));
        }
        if self.sim.blueprints_revision() != self.blueprints_sent {
            self.blueprints_sent = self.sim.blueprints_revision();
            out.push(ServerMsg::Blueprints(Box::new(self.sim.blueprints().clone())));
        }
        out.extend(self.chunk_news());
        let mut events: Vec<Event> = Vec::new();
        events.extend(self.sim.take_strikes().into_iter().map(Event::Strike));
        events.extend(self.sim.take_slams().into_iter().map(Event::Slam));
        events.extend(self.sim.take_player_hurts().into_iter().map(Event::PlayerHurt));
        events.extend(self.sim.take_block_edits().into_iter().map(Event::BlockEdit));
        events.extend(self.sim.take_blasts().into_iter().map(Event::Blast));
        if input.craft.is_some()
            && let Some(result) = self.sim.last_craft()
        {
            events.push(Event::Craft(result));
        }
        if input.blueprint.is_some()
            && let Some(result) = self.sim.last_blueprint()
        {
            events.push(Event::Blueprint(result.clone()));
        }
        out.push(ServerMsg::Tick(Box::new(self.snapshot(events))));
        out
    }

    /// What changed in the chunks since last time, for the client: the tops that moved, chunks
    /// dropped, then chunks brought in, far chunks dropped and built, then changed chunks the
    /// client has, or that are new to the world.
    fn chunk_news(&mut self) -> Vec<ServerMsg> {
        let tops = self.sim.take_tops();
        let unloaded = self.sim.take_unloaded_chunks();
        let loaded = self.sim.take_loaded_chunks();
        let changed = self.sim.take_changed_chunks();
        let far_gone = self.sim.take_far_unloaded();
        let far = self.sim.take_far_loaded();
        let (air, not_air) = self.sim.take_air_chunks();
        let grid = self.sim.universe().main();
        let mut out = Vec::new();
        if !tops.is_empty() {
            out.push(ServerMsg::Tops(tops));
        }
        let gone: Vec<api::ChunkPos> = unloaded.into_iter().filter(|pos| grid.chunk(*pos).is_none() && self.has.remove(pos)).collect();
        if !gone.is_empty() {
            out.push(ServerMsg::ChunksUnloaded(gone));
        }
        let mut fresh = HashSet::new();
        let arrived: Vec<sim::ChunkData> = loaded
            .into_iter()
            .filter(|pos| grid.chunk(*pos).is_some() && fresh.insert(*pos))
            .map(|pos| grid.export_chunk(pos))
            .collect();
        if !arrived.is_empty() {
            self.has.extend(fresh.iter().copied());
            out.push(ServerMsg::ChunksLoaded(arrived));
        }
        if !air.is_empty() || !not_air.is_empty() {
            out.push(ServerMsg::AirChunks { came: air, gone: not_air });
        }
        if !far_gone.is_empty() {
            out.push(ServerMsg::FarUnloaded(far_gone));
        }
        if !far.is_empty() {
            out.push(ServerMsg::FarLoaded(far));
        }
        for pos in changed {
            if fresh.contains(&pos) {
                continue;
            }
            let there = grid.chunk(pos).is_some();
            if there || self.has.contains(&pos) {
                if there {
                    self.has.insert(pos);
                }
                out.push(ServerMsg::Chunk(grid.export_chunk(pos)));
            }
        }
        out
    }

    fn snapshot(&self, events: Vec<Event>) -> Snapshot {
        let sim = &self.sim;
        Snapshot {
            tick: sim.tick(),
            clock: *sim.clock(),
            player: sim.player().map(|p| p.body().clone()),
            health: sim.health(),
            targets: sim.targets().as_slice().to_vec(),
            items: sim.items().iter().cloned().collect(),
            inventory: sim.inventory().clone(),
            swing: sim.swing().copied(),
            guarding: sim.guarding(),
            terrain: sim.stream_stats(),
            far: sim.far_stats(),
            events,
        }
    }

    /// Reads the sounds again and swaps them in, returning every sound.
    fn reload_sounds(&mut self) -> Result<Vec<api::SoundDef>, String> {
        let source = self.sound_source.as_mut().ok_or("this server can't read its sounds again")?;
        let sounds = source()?;
        self.sim.replace_sounds(sounds).map_err(|e| e.to_string())?;
        Ok(self.sim.sounds().to_vec())
    }

    /// Writes the world to its file, and its edited terrain to its chunk store. `auto` says
    /// it's an autosave.
    fn save_world(&mut self, auto: bool) -> WorldNote {
        self.since_saved = 0;
        let Some(file) = &self.world else { return WorldNote::Failed("this server has no world file".into()) };
        match self.sim.save().and_then(|save| file.write(&save)) {
            Ok(()) => WorldNote::Saved { name: file.name.clone(), auto },
            Err(e) => WorldNote::Failed(format!("saving: {e}")),
        }
    }

    /// Saves the world as `name`, and plays on in that one; the world left stays as it was
    /// last saved.
    fn save_world_as(&mut self, name: &str) -> WorldNote {
        let Some(file) = &self.world else { return WorldNote::Failed("this server has no world file".into()) };
        if file.name == name {
            return self.save_world(false);
        }
        let next = match file.sibling(name) {
            Ok(next) => next,
            Err(e) => return WorldNote::Failed(e),
        };
        self.since_saved = 0;
        let saved = self.sim.save_as(Arc::new(next.chunk_files())).and_then(|save| next.write(&save));
        match saved {
            Ok(()) => {
                self.world = Some(next);
                WorldNote::Saved { name: name.to_owned(), auto: false }
            }
            Err(e) => WorldNote::Failed(format!("saving as {name}: {e}")),
        }
    }

    /// Puts the world back as its file has it.
    fn load_world(&mut self) -> WorldNote {
        let Some(file) = self.world.clone() else { return WorldNote::Failed("this server has no world file".into()) };
        match file.read() {
            Ok(Some(save)) => self.load(&file.name, &save, false),
            Ok(None) => WorldNote::Failed(format!("world {} hasn't been saved yet", file.name)),
            Err(e) => WorldNote::Failed(format!("loading: {e}")),
        }
    }

    /// Saves the world, then plays world `name`: as saved, or fresh if it hasn't been saved.
    fn open_world(&mut self, name: &str) -> WorldNote {
        let Some(file) = &self.world else { return WorldNote::Failed("this server has no world file".into()) };
        let next = match file.sibling(name) {
            Ok(next) => next,
            Err(e) => return WorldNote::Failed(e),
        };
        // Read it before leaving this one, so a damaged save changes nothing.
        let save = match next.read() {
            Ok(save) => save,
            Err(e) => return WorldNote::Failed(format!("opening: {e}")),
        };
        if save.is_none() && self.fresh.is_none() {
            return WorldNote::Failed("this server can't start fresh worlds".into());
        }
        if let WorldNote::Failed(why) = self.save_world(false) {
            return WorldNote::Failed(format!("{why}; stayed in this world"));
        }
        let store = Arc::new(next.chunk_files());
        if save.is_none() {
            // A world never saved has no chunks: any left in its folder go.
            if let Err(e) = store.forget().and_then(|()| store.commit()) {
                return WorldNote::Failed(format!("opening {name}: {e}; stayed in this world"));
            }
        }
        self.world = Some(next);
        self.sim.set_chunk_store(store);
        match save {
            Some(save) => self.load(name, &save, false),
            None => {
                // Fresh, but with the terrain as it's tuned now.
                let terrain = self.sim.snapshot().terrain;
                let fresh = SaveData { terrain, ..self.fresh.clone().expect("checked above") };
                self.load(name, &fresh, true)
            }
        }
    }

    /// Puts the world back as `save` has it, as world `name`.
    fn load(&mut self, name: &str, save: &SaveData, fresh: bool) -> WorldNote {
        self.since_saved = 0;
        match self.sim.load(save) {
            Ok(report) => {
                self.generation_error = None;
                for skipped in &report.skipped {
                    eprintln!("server: loading {name}: left out {skipped}");
                }
                WorldNote::Loaded { name: name.to_owned(), look: self.sim.look(), skipped: report.skipped, fresh }
            }
            Err(e) => WorldNote::Failed(format!("loading: {e}")),
        }
    }

    fn command(&mut self, command: Command) {
        let sim = &mut self.sim;
        match command {
            Command::Tune { section, name, value } if section == "saving" => {
                let mut values = Saving { autosave_minutes: self.autosave_minutes };
                if values.set_tunable(&name, value) {
                    self.autosave_minutes = values.autosave_minutes;
                } else {
                    eprintln!("server: no tuning value [{section}] {name}");
                }
            }
            Command::Tune { section, name, value } => {
                if !tune(sim, &section, &name, value) {
                    eprintln!("server: no tuning value [{section}] {name}");
                }
            }
            Command::Heal => sim.set_health(f32::MAX),
            Command::Unkillable(on) => sim.set_unkillable(on),
            Command::BrainsOn(on) => sim.set_brains_on(on),
            Command::SetTime(hours) => sim.clock_mut().set_hours(hours),
            Command::PauseTime(paused) => sim.clock_mut().paused = paused,
            Command::TeleportPlayer(pos) => sim.teleport_player(pos),
            Command::RespawnTargets => sim.respawn_targets(),
            Command::ViewDistance(reach) => sim.set_reach(reach),
            Command::RegenerateTerrain => {
                self.generation_error = sim.generate_terrain().err().map(|e| e.to_string());
                if let Some(e) = &self.generation_error {
                    eprintln!("server: generating the terrain failed: {e}");
                }
            }
            Command::ReloadSounds
            | Command::SaveWorld
            | Command::LoadWorld
            | Command::SaveWorldAs(_)
            | Command::OpenWorld(_)
            | Command::ListWorlds => {}
        }
    }

    fn defs(&mut self) -> Defs {
        let mut tuning = sections(&mut self.sim);
        let saving = Saving { autosave_minutes: self.autosave_minutes };
        tuning.push(TuningSection { name: "saving".into(), values: saving.tunables() });
        let sim = &self.sim;
        Defs {
            player: sim.player_def().cloned(),
            interaction: sim.interaction_def().cloned(),
            push_out: sim.push_out_def().cloned(),
            crowd: sim.crowd_def().cloned(),
            health: sim.health_def().cloned(),
            item_drops: sim.item_drop_def().cloned(),
            inventory: sim.inventory_def().cloned(),
            crafting: sim.crafting_def().cloned(),
            signals: sim.signal_def().cloned(),
            targets: sim.target_defs().to_vec(),
            tools: sim.tools().to_vec(),
            tuning,
            brains_on: sim.brains_on(),
            unkillable: sim.unkillable(),
            terrain_error: self.generation_error.clone().or_else(|| sim.terrain_error().map(|e| e.to_string())),
        }
    }

    /// The simulation, for tests.
    pub fn simulation(&self) -> &Simulation {
        &self.sim
    }
}

/// Every value a client may tune, by section.
fn sections(sim: &mut Simulation) -> Vec<TuningSection> {
    let mut out = Vec::new();
    let mut add = |name: String, values: Vec<api::Tunable>| out.push(TuningSection { name, values });
    if sim.player().is_some()
        && let Some(controller) = sim.player_controller()
    {
        add("movement".into(), controller.tunables());
    }
    if let Some(d) = sim.interaction_def() {
        add("interaction".into(), d.tunables());
    }
    if let Some(d) = sim.push_out_def() {
        add("push_out".into(), d.tunables());
    }
    if let Some(d) = sim.crowd_def() {
        add("crowd".into(), d.tunables());
    }
    if let Some(d) = sim.health_def() {
        add("health".into(), d.tunables());
    }
    if let Some(d) = sim.item_drop_def() {
        add("item_drops".into(), d.tunables());
    }
    if let Some(d) = sim.inventory_def() {
        add("inventory".into(), d.tunables());
    }
    if let Some(d) = sim.signal_def() {
        add("signals".into(), d.tunables());
    }
    for tool in sim.tools() {
        add(format!("tool {}", tool.name), tool.tunables());
    }
    for target in sim.target_defs() {
        add(format!("target {}", target.name), target.tunables());
    }
    let names: Vec<String> = sim.brain_names().map(String::from).collect();
    for (i, name) in names.into_iter().enumerate() {
        let values = sim.brain_mut(i).map_or_else(Vec::new, |b| b.tunables());
        add(format!("brain {name}"), values);
    }
    add("world".into(), sim.world_settings().tunables());
    add("time".into(), sim.clock().tunables());
    let names: Vec<String> = sim.generator_names().map(String::from).collect();
    for (i, name) in names.into_iter().enumerate() {
        add(format!("terrain {name}"), sim.generator_tunables(i));
    }
    out
}

/// The server's own saving values, tuned in the `saving` section.
struct Saving {
    autosave_minutes: f32,
}

/// The tuning panel's groups for [`Saving`]'s values.
const SAVING_GROUPS: &[(&str, &[&str])] = &[
];
/// What each of [`Saving`]'s values does, for the tuning panel.
const SAVING_HELP: &[(&str, &str)] = &[
    ("autosave_minutes", "Minutes between autosaves of the world; 0 never autosaves."),
];

impl Tunables for Saving {
    fn tunables(&self) -> Vec<Tunable> {
        api::tuning::described(vec![Tunable::new("autosave_minutes", self.autosave_minutes, 0.0, 60.0)], SAVING_GROUPS, SAVING_HELP)
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "autosave_minutes" => self.autosave_minutes = value.clamp(0.0, 60.0),
            _ => return false,
        }
        true
    }
}

/// Sets one value in one section (see [`sections`]). False if there's no such value.
fn tune(sim: &mut Simulation, section: &str, name: &str, value: f32) -> bool {
    fn set(t: Option<&mut dyn Tunables>, name: &str, value: f32) -> bool {
        t.is_some_and(|t| t.set_tunable(name, value))
    }
    match section {
        "movement" => set(sim.player_controller_mut().map(|c| c as &mut dyn Tunables), name, value),
        "interaction" => set(sim.interaction_def_mut().map(|d| d as &mut dyn Tunables), name, value),
        "push_out" => set(sim.push_out_def_mut().map(|d| d as &mut dyn Tunables), name, value),
        "crowd" => set(sim.crowd_def_mut().map(|d| d as &mut dyn Tunables), name, value),
        "health" => set(sim.health_def_mut().map(|d| d as &mut dyn Tunables), name, value),
        "item_drops" => set(sim.item_drop_def_mut().map(|d| d as &mut dyn Tunables), name, value),
        "inventory" => set(sim.inventory_def_mut().map(|d| d as &mut dyn Tunables), name, value),
        "signals" => set(sim.signal_def_mut().map(|d| d as &mut dyn Tunables), name, value),
        "world" => sim.world_settings_mut().set_tunable(name, value),
        "time" => sim.clock_mut().set_tunable(name, value),
        _ => {
            if let Some(tool) = section.strip_prefix("tool ") {
                let found = sim.tools_mut().iter_mut().find(|t| t.name == tool);
                set(found.map(|t| t as &mut dyn Tunables), name, value)
            } else if let Some(target) = section.strip_prefix("target ") {
                let found = sim.target_defs_mut().iter_mut().find(|t| t.name == target);
                set(found.map(|t| t as &mut dyn Tunables), name, value)
            } else if let Some(brain) = section.strip_prefix("brain ") {
                let index = sim.brain_names().position(|n| n == brain);
                set(index.and_then(|i| sim.brain_mut(i)).map(|b| b as &mut dyn Tunables), name, value)
            } else if let Some(generator) = section.strip_prefix("terrain ") {
                let index = sim.generator_names().position(|n| n == generator);
                index.is_some_and(|i| sim.tune_generator(i, name, value))
            } else {
                false
            }
        }
    }
}

/// A server on the client's own thread: each input runs a tick right away, so the client's
/// fixed tick drives the simulation's, as before there was a server.
pub struct LocalConnection {
    server: Server,
    inbox: Vec<ServerMsg>,
}

impl LocalConnection {
    pub fn new(mut server: Server) -> Self {
        let welcome = ServerMsg::Welcome(Box::new(server.welcome()));
        Self { server, inbox: vec![welcome] }
    }
}

impl Connection for LocalConnection {
    fn send(&mut self, msg: ClientMsg) {
        let replies = self.server.handle(msg);
        self.inbox.extend(replies);
    }

    fn receive(&mut self) -> Vec<ServerMsg> {
        std::mem::take(&mut self.inbox)
    }
}

/// A server on its own thread, talking over channels. It runs a tick for each input it gets,
/// as the local one does. The thread ends when the connection is dropped.
pub struct ThreadConnection {
    to_server: Option<Sender<ClientMsg>>,
    from_server: Receiver<ServerMsg>,
    thread: Option<JoinHandle<()>>,
}

impl ThreadConnection {
    pub fn spawn(mut server: Server) -> Self {
        let (to_server, inbox) = channel::<ClientMsg>();
        let (outbox, from_server) = channel::<ServerMsg>();
        let thread = std::thread::spawn(move || {
            if outbox.send(ServerMsg::Welcome(Box::new(server.welcome()))).is_err() {
                return;
            }
            for msg in inbox {
                for reply in server.handle(msg) {
                    if outbox.send(reply).is_err() {
                        return;
                    }
                }
            }
        });
        Self { to_server: Some(to_server), from_server, thread: Some(thread) }
    }

    /// Waits for the next message.
    pub fn wait(&mut self) -> Option<ServerMsg> {
        self.from_server.recv().ok()
    }
}

impl Connection for ThreadConnection {
    fn send(&mut self, msg: ClientMsg) {
        if let Some(to_server) = &self.to_server {
            // A server that stopped has nothing more to say; its messages just stop coming.
            let _ = to_server.send(msg);
        }
    }

    fn receive(&mut self) -> Vec<ServerMsg> {
        self.from_server.try_iter().collect()
    }
}

impl Drop for ThreadConnection {
    fn drop(&mut self) {
        self.to_server = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
