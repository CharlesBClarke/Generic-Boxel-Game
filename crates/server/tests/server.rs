//! The server behind its message boundary: inputs and commands in, the world and snapshots out.
//! A client keeps a copy of the world from the messages alone, so the copy must match.

use api::{BoxelPos, I64Vec3, Mod, Vec3, WorldPos};
use protocol::{ClientMsg, Command, Connection, Event, ServerMsg, Snapshot, WorldNote};
use server::{LocalConnection, Server, ThreadConnection, WorldFile};
use sim::{BlueprintEdit, BlueprintRefusal, Cells, Craft, Grid, MaterialId, Piece, PlayerInput, Recipe, Simulation, Universe};

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
                self.universe.main_mut().tops = w.tops.clone();
                self.blocks = w.blocks.len();
            }
            ServerMsg::Tops(tops) => {
                let known = &mut self.universe.main_mut().tops;
                for &(x, y) in &tops.dropped {
                    known.remove_column(x, y);
                }
                for ((x, y), column) in &tops.set {
                    known.set_column(*x, *y, std::sync::Arc::clone(column));
                }
            }
            ServerMsg::Chunk(chunk) => self.universe.main_mut().import_chunk(chunk),
            ServerMsg::ChunksLoaded(chunks) => {
                for chunk in chunks {
                    self.universe.main_mut().import_chunk(chunk);
                }
            }
            ServerMsg::ChunksUnloaded(gone) => {
                for pos in gone {
                    self.universe.main_mut().remove_chunk(*pos);
                }
            }
            ServerMsg::Blocks(blocks) => self.blocks = blocks.len(),
            ServerMsg::Tick(_)
            | ServerMsg::AirChunks { .. }
            | ServerMsg::FarLoaded(_)
            | ServerMsg::FarUnloaded(_)
            | ServerMsg::Blueprints(_)
            | ServerMsg::Defs(_)
            | ServerMsg::Sounds(_)
            | ServerMsg::World(_)
            | ServerMsg::Worlds(_) => {}
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
    let msgs = server.handle(input(PlayerInput { craft: Some(Craft::grid(table, cells, false, false, 1)), ..Default::default() }));
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
fn blueprint_edits_send_the_library_before_the_tick_and_refusals_in_its_events() {
    let mut server = Server::new(base_sim());
    let welcome = server.welcome();
    assert!(welcome.blueprints.is_empty());
    let brick: MaterialId = server.simulation().blocks().material(base::BRICK).unwrap();
    let mut cells: Cells = Default::default();
    cells[0][0][0] = Some(Piece::new(brick));
    let save = BlueprintEdit::Save { name: "nub".into(), recipe: Recipe::top(cells) };
    let msgs = server.handle(input(PlayerInput { blueprint: Some(save), ..Default::default() }));
    let ServerMsg::Blueprints(library) = &msgs[0] else { panic!("the library comes first") };
    let id = library.find("nub").expect("saved");
    assert!(snapshot(&msgs).events.contains(&Event::Blueprint(Ok(id))));
    // Nothing changed, nothing sent.
    let msgs = server.handle(input(PlayerInput::default()));
    assert!(!msgs.iter().any(|m| matches!(m, ServerMsg::Blueprints(_))));
    // A refusal changes nothing, so only its outcome comes.
    let undo = BlueprintEdit::Undo { id };
    let msgs = server.handle(input(PlayerInput { blueprint: Some(undo), ..Default::default() }));
    assert!(!msgs.iter().any(|m| matches!(m, ServerMsg::Blueprints(_))));
    assert!(snapshot(&msgs).events.contains(&Event::Blueprint(Err(BlueprintRefusal::NoChanges))));
}

/// The base game keeping little terrain loaded, generated, so saving and loading stay quick.
fn small_world() -> Simulation {
    let mut sim = base_sim();
    sim.set_view_distance(32.0, 0.0);
    sim.generate_terrain().unwrap();
    sim
}

#[test]
fn the_copy_follows_terrain_streaming_in_and_out_as_the_player_moves() {
    let mut sim = base_sim();
    sim.set_view_distance(32.0, 0.0);
    let over = |x: i64| WorldPos::at_boxel(base::TERRAIN_CENTER.offset(I64Vec3::new(x, 5, 150)));
    sim.teleport_player(over(0));
    sim.generate_terrain().unwrap();
    let mut server = Server::new(sim);
    let mut copy = Copy { universe: Universe::default(), blocks: 0 };
    copy.apply(&ServerMsg::Welcome(Box::new(server.welcome())));
    let fly = PlayerInput { toggle_fly: true, ..Default::default() };
    let mut news = (0, 0);
    let mut step = |server: &mut Server, copy: &mut Copy, what: PlayerInput| {
        for m in server.handle(input(what)) {
            match &m {
                ServerMsg::ChunksLoaded(c) => {
                    news.0 += c.len();
                    // The tops a chunk is under, and beside, come before it does.
                    let tops = &copy.universe.main().tops;
                    for chunk in c {
                        for (dx, dy) in sim::tops::AROUND {
                            assert!(tops.column(chunk.pos.0.x + dx, chunk.pos.0.y + dy).is_some(), "tops for {:?}", chunk.pos);
                        }
                    }
                }
                ServerMsg::ChunksUnloaded(c) => news.1 += c.len(),
                _ => {}
            }
            copy.apply(&m);
        }
    };
    step(&mut server, &mut copy, fly);
    // Hops east across many chunk borders, a few ticks apart, then waits for the terrain.
    for hop in 1..=8 {
        server.handle(ClientMsg::Command(Command::TeleportPlayer(over(hop * 37))));
        for _ in 0..3 {
            step(&mut server, &mut copy, PlayerInput::default());
        }
    }
    for _ in 0..4000 {
        step(&mut server, &mut copy, PlayerInput::default());
        if server.simulation().stream_stats().waiting == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(server.simulation().stream_stats().waiting, 0, "the terrain caught up");
    assert!(news.0 > 0 && news.1 > 0, "chunks came and went: {news:?}");
    let (ours, theirs) = (copy.universe.main(), server.simulation().universe().main());
    let chunks = |g: &Grid| {
        let mut v: Vec<_> = g.chunks().map(|(p, _)| p.0.to_array()).collect();
        v.sort();
        v
    };
    assert_eq!(chunks(ours), chunks(theirs), "the copy holds what the server holds");
    assert!(same_world(ours, theirs));
    for (pos, chunk) in theirs.chunks() {
        assert_eq!(ours.chunk(pos).unwrap().light, chunk.light, "light in {pos:?}");
    }
    assert!(!theirs.tops.is_empty() && ours.tops == theirs.tops, "and the same tops, with those left behind dropped");
}

#[test]
fn the_air_around_the_test_area_is_told_and_follows_the_player() {
    // At spawn, in the test area: no terrain near, so the air around the floor never comes.
    let mut sim = base_sim();
    sim.set_view_distance(64.0, 0.0);
    sim.generate_terrain().unwrap();
    let mut server = Server::new(sim);
    let mut air: std::collections::HashSet<api::ChunkPos> = server.welcome().air.into_iter().collect();
    let step = |server: &mut Server, air: &mut std::collections::HashSet<api::ChunkPos>| {
        for m in server.handle(input(PlayerInput::default())) {
            if let ServerMsg::AirChunks { came, gone } = m {
                for p in &gone {
                    assert!(air.remove(p), "only what was air goes: {p:?}");
                }
                for p in came {
                    assert!(air.insert(p), "told once: {p:?}");
                }
            }
        }
    };
    step(&mut server, &mut air);
    let grid = server.simulation().universe().main();
    assert!(air.contains(&api::ChunkPos::new(0, 0, 1)), "over the floor");
    assert!(air.iter().all(|&p| grid.chunk(p).is_none()), "never what was built");
    // Off to the far copy: what was air here goes, the same around the far copy comes.
    server.handle(ClientMsg::Command(Command::TeleportPlayer(WorldPos::at_boxel(base::FAR_ORIGIN.offset(I64Vec3::new(0, 0, 2))))));
    step(&mut server, &mut air);
    let far = base::FAR_ORIGIN.0 / 32;
    assert!(!air.contains(&api::ChunkPos::new(0, 0, 1)));
    assert!(air.contains(&api::ChunkPos(far + I64Vec3::new(0, 0, 1))), "over the far copy's floor");
    let expected: std::collections::HashSet<api::ChunkPos> = server.simulation().air_chunks().collect();
    assert_eq!(air, expected, "the client knows what the server does");
}

/// Whether two games' worlds hold the same blocks everywhere, by name: separate games may
/// number the blocks they made differently.
fn same_by_name(a: &Simulation, b: &Simulation) -> bool {
    let named = |s: &Simulation, pos| {
        let data = s.universe().main().export_chunk(pos);
        let lists: Vec<Vec<(String, api::Orientation)>> = data
            .lists
            .iter()
            .map(|l| l.iter().map(|o| (s.blocks().get(o.block).unwrap().name.clone(), o.orientation)).collect())
            .collect();
        (0..api::CHUNK_VOLUME)
            .map(|i| match &data.cells {
                sim::Layer::Uniform(v) => lists[*v as usize].clone(),
                sim::Layer::Dense(values) => lists[values[i] as usize].clone(),
            })
            .collect::<Vec<_>>()
    };
    let (ga, gb) = (a.universe().main(), b.universe().main());
    let chunks: Vec<_> = ga.chunks().chain(gb.chunks()).map(|(pos, _)| pos).collect();
    chunks.into_iter().all(|pos| named(a, pos) == named(b, pos))
}

/// The note a save or load command answered with.
fn note(msgs: Vec<ServerMsg>) -> WorldNote {
    match msgs.into_iter().next() {
        Some(ServerMsg::World(note)) => note,
        _ => panic!("saving and loading answer with a note"),
    }
}

#[test]
fn saving_then_loading_puts_the_world_back_and_the_copy_follows() {
    let dir = std::env::temp_dir().join(format!("boxel-server-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = WorldFile::in_folder(&dir, "test").unwrap();
    let mut sim = small_world();
    sim.set_chunk_store(std::sync::Arc::new(file.chunk_files()));
    let mut server = Server::new(sim).with_world(file.clone(), false);
    let mut copy = Copy { universe: Universe::default(), blocks: 0 };
    copy.apply(&ServerMsg::Welcome(Box::new(server.welcome())));

    let load = || ClientMsg::Command(Command::LoadWorld);
    assert!(matches!(note(server.handle(load())), WorldNote::Failed(_)), "nothing saved yet");
    assert_eq!(note(server.handle(ClientMsg::Command(Command::SaveWorld))), WorldNote::Saved { name: "test".into(), auto: false });
    let mut saved = base_sim();
    saved.load(&file.read().unwrap().unwrap()).unwrap();
    assert!(same_by_name(&saved, server.simulation()));

    // Break and place all around.
    for i in 0..120 {
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
            copy.apply(&msg);
        }
    }
    assert!(!same_by_name(&saved, server.simulation()), "edited since");

    let WorldNote::Loaded { name, skipped, .. } = note(server.handle(load())) else { panic!("loaded") };
    assert_eq!((name.as_str(), skipped.len()), ("test", 0));
    for msg in server.handle(input(PlayerInput::default())) {
        copy.apply(&msg);
    }
    assert!(same_by_name(&saved, server.simulation()), "back as saved");
    assert!(same_world(copy.universe.main(), server.simulation().universe().main()), "the copy follows");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Breaks and places all around for a while.
fn edit(server: &mut Server, ticks: usize) {
    for i in 0..ticks {
        let input = PlayerInput {
            yaw: i as f32 * 0.37,
            pitch: -0.2 - (i % 7) as f32 * 0.15,
            use_pressed: i % 3 == 0,
            alt_pressed: i % 3 == 1,
            place_block: server.simulation().blocks().id(base::BRICK).unwrap(),
            movement: api::Vec2::new(((i / 40) % 2) as f32, 0.0),
            ..Default::default()
        };
        server.handle(ClientMsg::Input(Box::new(input)));
    }
}

fn worlds(server: &mut Server) -> Vec<String> {
    match server.handle(ClientMsg::Command(Command::ListWorlds)).into_iter().next() {
        Some(ServerMsg::Worlds(names)) => names,
        _ => panic!("a list of worlds"),
    }
}

#[test]
fn worlds_are_saved_as_opened_and_started_fresh() {
    let dir = std::env::temp_dir().join(format!("boxel-server-worlds-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = WorldFile::in_folder(&dir, "first").unwrap();
    let mut sim = base_sim();
    sim.set_view_distance(32.0, 0.0);
    let fresh = sim.snapshot();
    sim.set_chunk_store(std::sync::Arc::new(file.chunk_files()));
    sim.generate_terrain().unwrap();
    let mut server = Server::new(sim).with_world(file.clone(), false).with_fresh(fresh);
    let cmd = |c| ClientMsg::Command(c);

    edit(&mut server, 120);
    assert_eq!(note(server.handle(cmd(Command::SaveWorldAs("second".into())))), WorldNote::Saved { name: "second".into(), auto: false });
    assert_eq!(worlds(&mut server), ["second"], "the first was never saved");
    let mut edited = base_sim();
    edited.load(&file.sibling("second").unwrap().read().unwrap().unwrap()).unwrap();
    assert!(!same_by_name(&edited, &small_world()));

    // A new world starts as the game built it, and the one left is saved first.
    edit(&mut server, 30);
    let WorldNote::Loaded { name, fresh: true, .. } = note(server.handle(cmd(Command::OpenWorld("third".into())))) else {
        panic!("a fresh world")
    };
    assert_eq!(name, "third");
    assert!(same_by_name(server.simulation(), &small_world()), "untouched");
    let mut left = base_sim();
    left.load(&file.sibling("second").unwrap().read().unwrap().unwrap()).unwrap();
    assert!(!same_by_name(&left, &edited), "the edits since Save as were saved on leaving");

    // Back to the second, as it was left.
    let WorldNote::Loaded { fresh: false, .. } = note(server.handle(cmd(Command::OpenWorld("second".into())))) else {
        panic!("loaded")
    };
    assert!(same_by_name(server.simulation(), &left));
    assert_eq!(worlds(&mut server), ["third", "second"], "last saved first");

    assert!(matches!(note(server.handle(cmd(Command::OpenWorld("../x".into())))), WorldNote::Failed(_)));
    assert!(matches!(note(server.handle(cmd(Command::SaveWorldAs("".into())))), WorldNote::Failed(_)));
    std::fs::remove_dir_all(&dir).unwrap();
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

/// The base game keeping little terrain loaded, its edited terrain in `file`'s folder.
fn game_on(file: &WorldFile) -> Simulation {
    let mut sim = base_sim();
    sim.set_view_distance(32.0, 0.0);
    sim.set_chunk_store(std::sync::Arc::new(file.chunk_files()));
    sim
}

/// Digs out the block under the player at `(x, y)` from the terrain's center. Returns it.
fn dig(sim: &mut Simulation, x: i64, y: i64) -> BoxelPos {
    let ground = base::terrain::Hills::default().ground(x, y);
    sim.teleport_player(WorldPos::at_boxel(base::TERRAIN_CENTER.offset(I64Vec3::new(x, y, ground))));
    sim.stream_all();
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
    let below = BoxelPos(sim.player().unwrap().pos().cell() - I64Vec3::Z);
    assert!(!sim.universe().main().cell(below).is_empty(), "standing on the ground");
    sim.step(&PlayerInput { pitch: -89.0f32.to_radians(), use_pressed: true, use_held: true, ..Default::default() });
    assert!(sim.universe().main().cell(below).is_empty(), "dug out");
    below
}

/// Whether a new game on `file`'s world, standing over `hole`, finds it dug out.
fn dug_in_saved(file: &WorldFile, hole: BoxelPos) -> bool {
    let mut sim = game_on(file);
    sim.load(&file.read().unwrap().unwrap()).unwrap();
    sim.teleport_player(WorldPos::at_boxel(hole.offset(I64Vec3::new(0, 0, 3))));
    sim.stream_all();
    sim.universe().main().cell(hole).is_empty()
}

fn files_in(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |d| d.count())
}

#[test]
fn edited_terrain_goes_to_the_world_folder_and_comes_back_in_a_new_game() {
    let dir = std::env::temp_dir().join(format!("boxel-server-stored-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = WorldFile::in_folder(&dir, "stored").unwrap();
    let mut sim = game_on(&file);
    sim.generate_terrain().unwrap();
    let hole = dig(&mut sim, 5, 7);
    // Flown away from, the hole's column unloads and the chunk goes to disk, unsaved.
    sim.teleport_player(WorldPos::at_boxel(base::TERRAIN_CENTER.offset(I64Vec3::new(600, 0, 200))));
    sim.player_mut().unwrap().set_flying(true);
    sim.stream_all();
    assert!(sim.universe().main().chunk(hole.split().0).is_none(), "unloaded");
    let mut server = Server::new(sim).with_world(file.clone(), false);
    // Written by the store's own thread, soon after.
    let unsaved = dir.join("stored").join("unsaved");
    let start = std::time::Instant::now();
    while files_in(&unsaved) == 0 && start.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(files_in(&unsaved) > 0, "written out unsaved");

    assert_eq!(note(server.handle(ClientMsg::Command(Command::SaveWorld))), WorldNote::Saved { name: "stored".into(), auto: false });
    assert_eq!(files_in(&dir.join("stored").join("unsaved")), 0, "saved, the unsaved ones move over");
    assert!(files_in(&dir.join("stored").join("chunks")) > 0);
    drop(server);
    assert!(dug_in_saved(&file, hole), "a new game finds the hole");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn save_as_copies_the_stored_terrain_and_leaves_the_old_world_as_last_saved() {
    let dir = std::env::temp_dir().join(format!("boxel-server-save-as-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = WorldFile::in_folder(&dir, "a").unwrap();
    let mut sim = game_on(&file);
    sim.generate_terrain().unwrap();
    let first = dig(&mut sim, 5, 7);
    let mut server = Server::new(sim).with_world(file.clone(), false);
    note(server.handle(ClientMsg::Command(Command::SaveWorld)));
    drop(server);

    // Back in it: a second hole, then saved as b.
    let mut sim = game_on(&file);
    sim.load(&file.read().unwrap().unwrap()).unwrap();
    let second = dig(&mut sim, 90, -60);
    let mut server = Server::new(sim).with_world(file.clone(), true);
    assert_eq!(note(server.handle(ClientMsg::Command(Command::SaveWorldAs("b".into())))), WorldNote::Saved { name: "b".into(), auto: false });
    drop(server);

    let b = file.sibling("b").unwrap();
    assert!(dug_in_saved(&b, first) && dug_in_saved(&b, second), "b has both holes");
    assert!(dug_in_saved(&file, first), "a keeps its saved hole");
    assert!(!dug_in_saved(&file, second), "but not the one dug after");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_world_saves_itself_now_and_then() {
    let dir = std::env::temp_dir().join(format!("boxel-server-autosave-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = WorldFile::in_folder(&dir, "auto").unwrap();
    let mut server = Server::new(game_on(&file)).with_world(file.clone(), false);
    let tune = ClientMsg::Command(Command::Tune { section: "saving".into(), name: "autosave_minutes".into(), value: 0.001 });
    server.handle(tune);
    let mut saved = Vec::new();
    for _ in 0..20 {
        for msg in server.handle(input(PlayerInput::default())) {
            if let ServerMsg::World(note) = msg {
                saved.push(note);
            }
        }
    }
    // 0.001 minutes is 3.6 ticks: every 3rd tick.
    assert_eq!(saved.len(), 6, "{saved:?}");
    assert!(saved.iter().all(|n| *n == WorldNote::Saved { name: "auto".into(), auto: true }));
    assert!(file.read().unwrap().is_some());
    server.handle(ClientMsg::Command(Command::Tune { section: "saving".into(), name: "autosave_minutes".into(), value: 0.0 }));
    for _ in 0..20 {
        let msgs = server.handle(input(PlayerInput::default()));
        assert!(!msgs.iter().any(|m| matches!(m, ServerMsg::World(_))), "none when off");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
