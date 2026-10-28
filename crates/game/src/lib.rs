//! Wires the base mod into the engine: its data half from a mod folder, its code half
//! compiled in. The movement controller runs as WebAssembly unless asked to run natively.

use std::error::Error;
use std::path::Path;

use sim::Simulation;

/// The base game's movement controller, compiled to a WebAssembly component by `build.rs`.
pub const BASE_MOVEMENT_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/base_movement.wasm"));

/// Where the movement controller runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Movement {
    Wasm,
    Native,
}

/// The engine with the base mod loaded from the source tree and the player spawned.
pub fn load(movement: Movement) -> Result<Simulation, Box<dyn Error>> {
    load_from(Path::new(base::DATA_DIR), movement)
}

/// The engine with the base mod's data half read from `data` and the player spawned.
pub fn load_from(data: &Path, movement: Movement) -> Result<Simulation, Box<dyn Error>> {
    let data = data_mods::load(data)?;
    let mut sim = match movement {
        Movement::Native => Simulation::with_mods(&[&data, &base::BaseMod, &base_movement::BaseMovementMod])?,
        Movement::Wasm => {
            let mut sim = Simulation::with_mods(&[&data, &base::BaseMod])?;
            let (name, controller) = wasm_host::load_movement(BASE_MOVEMENT_WASM)?;
            sim.add_movement_controller(&name, Box::new(controller))?;
            sim
        }
    };
    sim.spawn_player()?;
    Ok(sim)
}
