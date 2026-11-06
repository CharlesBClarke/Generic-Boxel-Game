//! The server behind its message boundary: inputs and commands in, the world and snapshots out.
//! A client keeps a copy of the world from the messages alone, so the copy must match.

use api::{BoxelPos, I64Vec3, Mod, Vec3, WorldPos};
use protocol::{ClientMsg, Command, Connection, Event, ServerMsg, Snapshot};
use server::{LocalConnection, Server, ThreadConnection};
use sim::{Cells, Craft, Grid, MaterialId, Piece, PlayerInput, Simulation, Universe};

fn base_sim() -> Simulation {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mods: [&dyn Mod; 4] = [&data, &base::BaseMod, &base_cow::BaseCowMod, &base_movement::BaseMovementMod];
    let mut sim = Simulation::with_mods(&mods).expect("base mod loads");
    sim.spawn_player().expect("base registers a player");
    sim
}

fn input(input: PlayerInput) -> ClientMsg {
    ClientMsg::Input(Box::new(input))
}

/// The last snapshot among `msgs`.
fn snapshot(msgs: &[ServerMsg]) -> &Snapshot {
    msgs.iter()
        .rev()
        .find_map(|m| match m {
            ServerMsg::Tick(s) => Some(&**s),
            _ => None,
        })
        .expect("a tick sends a snapshot")
}

/// A client's copy of the world, kept from messages alone.
struct Copy {
    universe: Universe,
    blocks: usize,
}

impl Copy {
    fn apply(&mut self, msg: &ServerMsg) {
        match msg {
            ServerMsg::Welcome(w) => {
                for chunk in &w.chunks {
                    self.universe.main_mut().import_chunk(chunk);
                }
                self.blocks = w.blocks.len();
            }
            ServerMsg::Chunk(chunk) => self.universe.main_mut().import_chunk(chunk),
            ServerMsg::Blocks(blocks) => self.blocks = blocks.len(),
            ServerMsg::Tick(_) | ServerMsg::Defs(_) | ServerMsg::Sounds(_) => {}
        }
    }
}

/// Whether every boxel in every chunk either grid has reads the same in both.
fn same_world(a: &Grid, b: &Grid) -> bool {
    let chunks: Vec<_> = a.chunks().chain(b.chunks()).map(|(pos, _)| pos).collect();
    chunks.into_iter().all(|pos| a.export_chunk(pos).lists_equal(&b.export_chunk(pos)))
}

trait ListsEqual {
    fn lists_equal(&self, other: &Self) -> bool;
}

impl ListsEqual for sim::ChunkData {
    /// The same occupants in every boxel, however the lists are numbered.
    fn lists_equal(&self, other: &Self) -> bool {
        (0..api::CHUNK_VOLUME).all(|i| {
            let at = |c: &sim::ChunkData| {
                let index = match &c.cells {
                    sim::Layer::Uniform(v) => *v,
                    sim::Layer::Dense(values) => values[i],
                };
                c.lists[index as usize].clone()
            };
            at(self) == at(other)
        })
    }
}

#[test]
fn input_moves_the_player_in_the_snapshot() {
    let mut server = Server::new(base_sim());
    let start = server.welcome().snapshot.player.unwrap().pos();
    let mut last = Vec::new();
    for _ in 0..30 {
        last = server.handle(input(PlayerInput { movement: api::Vec2::Y, ..Default::default() }));
    }
    let snap = snapshot(&last);
    let moved = start.delta_to(snap.player.as_ref().unwrap().pos());
    // Yaw 0 faces east, so forward is +X.
    assert!(moved.x > 1.0, "walked east: {moved}");
    assert_eq!(snap.tick, 30);
}

#[test]
fn the_copy_matches_the_server_through_edits_and_crafts() {
    let mut server = Server::new(base_sim());
    let mut copy = Copy { universe: Universe::default(), blocks: 0 };
    copy.apply(&ServerMsg::Welcome(Box::new(server.welcome())));
    assert!(same_world(copy.universe.main(), server.simulation().universe().main()), "after the welcome");

    // Break and place all around, from a few spots, looking every way.
    let mut sent_chunks = 0;
    for i in 0..200 {
        let input = PlayerInput {
            yaw: i as f32 * 0.37,
            pitch: -0.2 - (i % 7) as f32 * 0.15,
            use_pressed: i % 3 == 0,
            alt_pressed: i % 3 == 1,
            place_block: server.simulation().blocks().id(base::BRICK).unwrap(),
            movement: api::Vec2::new(((i / 40) % 2) as f32, 0.0),
            ..Default::default()
        };
        for msg in server.handle(ClientMsg::Input(Box::new(input))) {
            sent_chunks += matches!(msg, ServerMsg::Chunk(_)) as usize;
            copy.apply(&msg);
        }
    }
    assert!(sent_chunks > 0, "edits send chunks");
    assert!(same_world(copy.universe.main(), server.simulation().universe().main()), "after edits");

    // Crafting a new shape adds a block, and the client hears of it.
    let (x, y, z) = base::TABLE_SPOT;
    let table = BoxelPos::new(x, y, z);
    let at = WorldPos::new(I64Vec3::new(x, y - 2, 0), Vec3::new(0.5, 0.5, 0.0));
    copy.apply(&server.handle(ClientMsg::Command(Command::TeleportPlayer(at))).remove(0));
    let brick: MaterialId = server.simulation().blocks().material(base::BRICK).unwrap();
    // One short stack in a corner: a shape nothing else makes.
    let mut cells: Cells = Default::default();
    cells[0][0][0] = Some(Piece::new(brick));
    let before = server.simulation().blocks().len();
    let msgs = server.handle(input(PlayerInput { craft: Some(Craft { table, cells, flip: false, count: 1 }), ..Default::default() }));
    for msg in &msgs {
        copy.apply(msg);
    }
    assert!(matches!(msgs[0], ServerMsg::Blocks(_)), "new blocks come before the tick");
    assert!(server.simulation().blocks().len() > before);
    assert_eq!(copy.blocks, server.simulation().blocks().len());
    let crafted = snapshot(&msgs).events.iter().any(|e| matches!(e, Event::Craft(Ok(_))));
    assert!(crafted, "the craft's outcome is in the tick's events");
}

#[test]
fn commands_tune_and_debug() {
    let mut server = Server::new(base_sim());
    let welcome = server.welcome();
    let reach = welcome.defs.interaction.as_ref().unwrap().reach;
    let tune = |section: &str, name: &str, value| {
        ClientMsg::Command(Command::Tune { section: section.into(), name: name.into(), value })
    };
    let defs = |msgs: Vec<ServerMsg>| match msgs.into_iter().next() {
        Some(ServerMsg::Defs(d)) => *d,
        _ => panic!("a command answers with the definitions"),
    };

    let d = defs(server.handle(tune("interaction", "reach", reach + 2.0)));
    assert_eq!(d.interaction.unwrap().reach, reach + 2.0);
    assert_eq!(server.simulation().interaction_def().unwrap().reach, reach + 2.0);

    // Every section the server lists can be tuned by its name.
    for section in &d.tuning {
        let t = section.values.first().expect("each section has values");
        let changed = defs(server.handle(tune(&section.name, &t.name, t.max)));
        let after = changed.tuning.iter().find(|s| s.name == section.name).unwrap();
        assert_eq!(after.values[0].value, t.max, "[{}] {}", section.name, t.name);
    }
    for name in ["movement", "health", "tool base:club", "brain base:cow"] {
        assert!(d.tuning.iter().any(|s| s.name == name), "{name} is listed");
    }
    // A wrong name changes nothing.
    let before = d.tuning.clone();
    let d = defs(server.handle(tune("nothing", "reach", 1.0)));
    assert_eq!(d.tuning.len(), before.len());

    let d = defs(server.handle(ClientMsg::Command(Command::Unkillable(true))));
    assert!(d.unkillable);
    let d = defs(server.handle(ClientMsg::Command(Command::BrainsOn(false))));
    assert!(!d.brains_on);
    let to = WorldPos::new(I64Vec3::new(10, 10, 0), Vec3::new(0.5, 0.5, 0.0));
    server.handle(ClientMsg::Command(Command::TeleportPlayer(to)));
    assert_eq!(server.simulation().player().unwrap().pos(), to);
    server.handle(ClientMsg::Command(Command::RespawnTargets));
    assert_eq!(server.simulation().targets().len(), welcome.snapshot.targets.len());
}

/// The inputs a player might give over a few seconds: walking, turning, jumping, swinging.
fn script() -> Vec<PlayerInput> {
    (0..240)
        .map(|i| PlayerInput {
            movement: api::Vec2::new(((i / 60) % 2) as f32, 1.0),
            yaw: i as f32 * 0.02,
            pitch: -0.3,
            jump_pressed: i % 50 == 0,
            jump_held: i % 50 < 10,
            use_pressed: i % 30 == 0,
            ..Default::default()
        })
        .collect()
}

#[test]
fn the_server_runs_the_simulation_bit_for_bit() {
    let mut direct = base_sim();
    let mut server = Server::new(base_sim());
    server.welcome();
    for input in script() {
        direct.step(&input);
        let msgs = server.handle(ClientMsg::Input(Box::new(input)));
        let snap = snapshot(&msgs);
        let (a, b) = (direct.player().unwrap(), snap.player.as_ref().unwrap());
        assert_eq!((a.pos(), a.velocity()), (b.pos(), b.velocity()), "tick {}", snap.tick);
        let targets: Vec<_> = direct.targets().iter().map(|t| (t.id, t.pos)).collect();
        assert_eq!(targets, snap.targets.iter().map(|t| (t.id, t.pos)).collect::<Vec<_>>());
    }
}

#[test]
fn a_server_on_its_own_thread_says_the_same() {
    let mut local = LocalConnection::new(Server::new(base_sim()));
    let mut thread = ThreadConnection::spawn(Server::new(base_sim()));
    let ticks = |msgs: Vec<ServerMsg>| -> Vec<(u64, WorldPos)> {
        msgs.into_iter()
            .filter_map(|m| match m {
                ServerMsg::Tick(s) => Some((s.tick, s.player.unwrap().pos())),
                ServerMsg::Welcome(w) => Some((w.snapshot.tick, w.snapshot.player.unwrap().pos())),
                _ => None,
            })
            .collect()
    };
    let script = script();
    for input in &script {
        local.send(ClientMsg::Input(Box::new(input.clone())));
        thread.send(ClientMsg::Input(Box::new(input.clone())));
    }
    let from_local = ticks(local.receive());
    // One welcome and one tick for each input, however long the thread takes.
    let mut from_thread = Vec::new();
    while from_thread.len() < script.len() + 1 {
        from_thread.extend(ticks(vec![thread.wait().expect("the thread is running")]));
    }
    assert_eq!(from_local.len(), script.len() + 1);
    assert_eq!(from_local, from_thread);
}

/// The sound reply to a reload.
fn reload(server: &mut Server) -> Result<Vec<api::SoundDef>, String> {
    let msgs = server.handle(ClientMsg::Command(Command::ReloadSounds));
    match msgs.into_iter().next() {
        Some(ServerMsg::Sounds(result)) => result,
        _ => panic!("a reload answers with the sounds"),
    }
}

#[test]
fn sounds_read_again_replace_the_old_ones_by_name() {
    let sim = base_sim();
    let moo = sim.sounds().iter().find(|s| s.name == "base:moo").unwrap().clone();
    let louder = api::SoundDef { volume: 0.25, ..moo.clone() };
    let fresh = api::SoundDef::new("base:new_one", b"OggS".to_vec());
    let reloaded = vec![louder.clone(), fresh.clone()];
    let mut server = Server::new(sim).with_sound_source(Box::new(move || Ok(reloaded.clone())));
    let sounds = reload(&mut server).expect("reloads");
    assert!(sounds.contains(&louder) && sounds.contains(&fresh), "replaced and added");
    assert!(!sounds.contains(&moo), "the old moo is gone");
    assert_eq!(server.simulation().sounds(), &sounds[..], "the server keeps what it sent");
}

#[test]
fn a_failed_reload_keeps_the_old_sounds() {
    let before = base_sim().sounds().to_vec();
    let mut server = Server::new(base_sim()).with_sound_source(Box::new(|| Err("sounds.toml:3: bad".into())));
    assert_eq!(reload(&mut server), Err("sounds.toml:3: bad".into()));
    assert_eq!(server.simulation().sounds(), &before[..]);

    let silent = api::SoundDef { variations: Vec::new(), ..before[0].clone() };
    let mut server = Server::new(base_sim()).with_sound_source(Box::new(move || Ok(vec![silent.clone()])));
    assert!(reload(&mut server).unwrap_err().contains("no audio"), "an invalid sound changes nothing");
    assert_eq!(server.simulation().sounds(), &before[..]);

    let mut server = Server::new(base_sim());
    assert!(reload(&mut server).is_err(), "no source to read from");
}
