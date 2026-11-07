//! Loads the base mod into the engine, runs it on a local server and opens the client.
//!
//! - `--native-movement` runs the movement controller natively instead of as WebAssembly, for
//!   comparing the two.
//! - `--native-brain` does the same for the cow brain.
//! - `--mods <dir>` reads the base mod's data half from `dir` instead of the source tree.
//! - `--world <name>` plays the saved world `name` (see `server::worlds`), starting it fresh if
//!   it hasn't been saved yet. Without it, the world is `default`.

use std::path::PathBuf;

use game::{Code, Where};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let place = |flag: &str| if args.iter().any(|a| a == flag) { Where::Native } else { Where::Wasm };
    let code = Code { movement: place("--native-movement"), brain: place("--native-brain") };
    let data = match args.iter().position(|a| a == "--mods") {
        Some(i) => PathBuf::from(args.get(i + 1).unwrap_or_else(|| panic!("--mods needs a folder"))),
        None => PathBuf::from(base::DATA_DIR),
    };
    let name = match args.iter().position(|a| a == "--world") {
        Some(i) => args.get(i + 1).unwrap_or_else(|| panic!("--world needs a name")).as_str(),
        None => server::DEFAULT_WORLD,
    };
    let fail = |e: String| -> ! {
        eprintln!("loading the game failed: {e}");
        std::process::exit(1);
    };
    let world = server::WorldFile::named(name).unwrap_or_else(|e| fail(e));
    // A save that can't be read stops the game, so quitting doesn't save over it.
    let save = world.read().unwrap_or_else(|e| fail(format!("{e} (move it aside to start the world fresh)")));
    eprintln!("movement controller: {:?}, cow brain: {:?}", code.movement, code.brain);
    eprintln!("base data: {}", data.display());
    match &save {
        Some(_) => eprintln!("world: {} (loaded from {})", world.name, world.path().display()),
        None => eprintln!("world: {} (new; saves to {})", world.name, world.path().display()),
    }
    let (sim, fresh) = game::load_world(&data, code, save.as_ref()).unwrap_or_else(|e| fail(e.to_string()));
    let sounds = data.clone();
    let server = server::Server::new(sim)
        .with_sound_source(Box::new(move || data_mods::load_sounds(&sounds).map_err(|e| e.to_string())))
        .with_world(world, save.is_some())
        .with_fresh(fresh);
    client::run(Box::new(server::LocalConnection::new(server)));
}
