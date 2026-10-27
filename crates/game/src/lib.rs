//! Wires the base mod into the engine. The movement controller runs as WebAssembly unless
//! asked to run natively.

use std::error::Error;

use sim::Simulation;

/// The base game's movement controller, compiled to a WebAssembly component by `build.rs`.
pub const BASE_MOVEMENT_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/base_movement.wasm"));

/// Where the movement controller runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Movement {
    Wasm,
    Native,
}

/// The engine with the base mod loaded and the player spawned.
pub fn load(movement: Movement) -> Result<Simulation, Box<dyn Error>> {
    let mut sim = match movement {
        Movement::Native => Simulation::with_mods(&[&base::BaseMod, &base_movement::BaseMovementMod])?,
        Movement::Wasm => {
            let mut sim = Simulation::with_mods(&[&base::BaseMod])?;
            let (name, controller) = wasm_host::load_movement(BASE_MOVEMENT_WASM)?;
            sim.add_movement_controller(&name, Box::new(controller))?;
            sim
        }
    };
    sim.spawn_player()?;
    Ok(sim)
}
