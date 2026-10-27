//! Loads the base mod into the engine and opens the client. `--native-movement` runs the
//! movement controller natively instead of as WebAssembly, for comparing the two.

use game::Movement;

fn main() {
    let movement = match std::env::args().any(|a| a == "--native-movement") {
        true => Movement::Native,
        false => Movement::Wasm,
    };
    eprintln!("movement controller: {movement:?}");
    let sim = game::load(movement).unwrap_or_else(|e| panic!("loading the game failed: {e}"));
    client::run(sim);
}
