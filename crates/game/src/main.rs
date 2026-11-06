//! Loads the base mod into the engine, runs it on a local server and opens the client.
//!
//! - `--native-movement` runs the movement controller natively instead of as WebAssembly, for
//!   comparing the two.
//! - `--mods <dir>` reads the base mod's data half from `dir` instead of the source tree.

use std::path::PathBuf;

use game::Movement;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let movement = match args.iter().any(|a| a == "--native-movement") {
        true => Movement::Native,
        false => Movement::Wasm,
    };
    let data = match args.iter().position(|a| a == "--mods") {
        Some(i) => PathBuf::from(args.get(i + 1).unwrap_or_else(|| panic!("--mods needs a folder"))),
        None => PathBuf::from(base::DATA_DIR),
    };
    eprintln!("movement controller: {movement:?}");
    eprintln!("base data: {}", data.display());
    let sim = game::load_from(&data, movement).unwrap_or_else(|e| {
        eprintln!("loading the game failed: {e}");
        std::process::exit(1);
    });
    let sounds = data.clone();
    let server = server::Server::new(sim)
        .with_sound_source(Box::new(move || data_mods::load_sounds(&sounds).map_err(|e| e.to_string())));
    client::run(Box::new(server::LocalConnection::new(server)));
}
