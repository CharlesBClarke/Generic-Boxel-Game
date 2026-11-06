//! The server: owns the simulation and talks to clients only through [`protocol`] messages.
//! Single-player runs one in the same process (see [`LocalConnection`]), so there is one code
//! path whether or not the server is far away.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use api::Tunables;
use protocol::{ClientMsg, Command, Connection, Content, Defs, Event, ServerMsg, Snapshot, TuningSection, Welcome};
use sim::{PlayerInput, Simulation};

/// Reads the sounds again from wherever the mods came from.
pub type SoundSource = Box<dyn FnMut() -> Result<Vec<api::SoundDef>, String> + Send>;

pub struct Server {
    sim: Simulation,
    /// How many blocks the client has heard of, so crafting a new shape sends the registry.
    blocks_sent: usize,
    /// Where [`Command::ReloadSounds`] reads from; without one it can't.
    sound_source: Option<SoundSource>,
}

impl Server {
    /// A server running `sim`, which must already have its mods loaded and its player spawned.
    pub fn new(sim: Simulation) -> Self {
        Self { sim, blocks_sent: 0, sound_source: None }
    }

    /// Lets clients have the sounds read again from `source` while the game runs.
    pub fn with_sound_source(mut self, source: SoundSource) -> Self {
        self.sound_source = Some(source);
        self
    }

    /// Everything a client needs to start.
    pub fn welcome(&mut self) -> Welcome {
        // Changes so far are in the chunks sent now.
        self.sim.take_changed_chunks();
        self.blocks_sent = self.sim.blocks().len();
        let grid = self.sim.universe().main();
        let chunks = grid.chunks().map(|(pos, _)| grid.export_chunk(pos)).collect();
        let snapshot = self.snapshot(Vec::new());
        Welcome {
            content: Content {
                looks: self.sim.looks().clone(),
                models: self.sim.models().to_vec(),
                sounds: self.sim.sounds().to_vec(),
            },
            blocks: self.sim.blocks().clone(),
            defs: self.defs(),
            chunks,
            spawn: self.sim.spawn(),
            waypoints: self.sim.waypoints().to_vec(),
            snapshot,
        }
    }

    /// Acts on a message from the client, and returns what to send back.
    pub fn handle(&mut self, msg: ClientMsg) -> Vec<ServerMsg> {
        match msg {
            ClientMsg::Input(input) => self.tick(&input),
            ClientMsg::Command(Command::ReloadSounds) => vec![ServerMsg::Sounds(self.reload_sounds())],
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
        if self.sim.blocks().len() != self.blocks_sent {
            self.blocks_sent = self.sim.blocks().len();
            out.push(ServerMsg::Blocks(Box::new(self.sim.blocks().clone())));
        }
        let changed = self.sim.take_changed_chunks();
        let grid = self.sim.universe().main();
        out.extend(changed.into_iter().map(|pos| ServerMsg::Chunk(grid.export_chunk(pos))));
        let mut events: Vec<Event> = Vec::new();
        events.extend(self.sim.take_strikes().into_iter().map(Event::Strike));
        events.extend(self.sim.take_slams().into_iter().map(Event::Slam));
        events.extend(self.sim.take_player_hurts().into_iter().map(Event::PlayerHurt));
        events.extend(self.sim.take_block_edits().into_iter().map(Event::BlockEdit));
        if input.craft.is_some()
            && let Some(result) = self.sim.last_craft()
        {
            events.push(Event::Craft(result));
        }
        out.push(ServerMsg::Tick(Box::new(self.snapshot(events))));
        out
    }

    fn snapshot(&self, events: Vec<Event>) -> Snapshot {
        let sim = &self.sim;
        Snapshot {
            tick: sim.tick(),
            player: sim.player().map(|p| p.body().clone()),
            health: sim.health(),
            targets: sim.targets().as_slice().to_vec(),
            items: sim.items().iter().cloned().collect(),
            inventory: sim.inventory().clone(),
            swing: sim.swing().copied(),
            guarding: sim.guarding(),
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

    fn command(&mut self, command: Command) {
        let sim = &mut self.sim;
        match command {
            Command::Tune { section, name, value } => {
                if !tune(sim, &section, &name, value) {
                    eprintln!("server: no tuning value [{section}] {name}");
                }
            }
            Command::Heal => sim.set_health(f32::MAX),
            Command::Unkillable(on) => sim.set_unkillable(on),
            Command::BrainsOn(on) => sim.set_brains_on(on),
            Command::TeleportPlayer(pos) => sim.teleport_player(pos),
            Command::RespawnTargets => sim.respawn_targets(),
            Command::ReloadSounds => {}
        }
    }

    fn defs(&mut self) -> Defs {
        let tuning = sections(&mut self.sim);
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
            targets: sim.target_defs().to_vec(),
            tools: sim.tools().to_vec(),
            tuning,
            brains_on: sim.brains_on(),
            unkillable: sim.unkillable(),
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
    if let Some(player) = sim.player() {
        add("movement".into(), player.controller().tunables());
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
    out
}

/// Sets one value in one section (see [`sections`]). False if there's no such value.
fn tune(sim: &mut Simulation, section: &str, name: &str, value: f32) -> bool {
    fn set(t: Option<&mut dyn Tunables>, name: &str, value: f32) -> bool {
        t.is_some_and(|t| t.set_tunable(name, value))
    }
    match section {
        "movement" => set(sim.player_mut().map(|p| p.controller_mut() as &mut dyn Tunables), name, value),
        "interaction" => set(sim.interaction_def_mut().map(|d| d as &mut dyn Tunables), name, value),
        "push_out" => set(sim.push_out_def_mut().map(|d| d as &mut dyn Tunables), name, value),
        "crowd" => set(sim.crowd_def_mut().map(|d| d as &mut dyn Tunables), name, value),
        "health" => set(sim.health_def_mut().map(|d| d as &mut dyn Tunables), name, value),
        "item_drops" => set(sim.item_drop_def_mut().map(|d| d as &mut dyn Tunables), name, value),
        "inventory" => set(sim.inventory_def_mut().map(|d| d as &mut dyn Tunables), name, value),
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
