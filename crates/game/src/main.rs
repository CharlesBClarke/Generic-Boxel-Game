//! Loads the base mod into the engine and opens the client.

use sim::Simulation;

fn main() {
    let mut sim = Simulation::with_mods(&[&base::BaseMod]).unwrap_or_else(|e| panic!("loading mods failed: {e}"));
    sim.spawn_player().unwrap_or_else(|e| panic!("spawning the player failed: {e}"));
    client::run(sim);
}
