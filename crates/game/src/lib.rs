//! Wires the base mod into the engine: its data half from a mod folder, its code half compiled
//! in. The movement controller and the cow brain run as WebAssembly unless asked to run natively.

use std::error::Error;
use std::path::Path;

use api::{ApiError, Mod, Registrar};
use sim::{SaveData, Simulation};

/// The base game's movement controller, compiled to a WebAssembly component by `build.rs`.
pub const BASE_MOVEMENT_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/base_movement.wasm"));
/// The base game's cow brain, compiled to a WebAssembly component by `build.rs`.
pub const BASE_COW_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/base_cow.wasm"));

/// Where a piece of the base mod's code runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Where {
    Wasm,
    Native,
}

/// Where each piece of the base mod's code runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Code {
    pub movement: Where,
    pub brain: Where,
}

impl Code {
    /// Everything as WebAssembly, as the game runs.
    pub const WASM: Code = Code { movement: Where::Wasm, brain: Where::Wasm };
    /// Everything native, for comparing and for tests that don't need the sandbox.
    pub const NATIVE: Code = Code { movement: Where::Native, brain: Where::Native };
}

/// The cow brain loaded from its component, registered like a mod would. Target kinds find
/// their brains while mods register, so it has to come in as a mod, not afterwards.
struct WasmCow(wasm_host::WasmBrain, String);

impl Mod for WasmCow {
    fn name(&self) -> &str {
        "base"
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        r.register_brain(&self.1, Box::new(self.0.clone()))
    }
}

/// The engine with the base mod loaded from the source tree, the player spawned and the terrain
/// under them generated (the rest streams in as the game runs).
pub fn load(code: Code) -> Result<Simulation, Box<dyn Error>> {
    load_from(Path::new(base::DATA_DIR), code)
}

/// The engine with the base mod's data half read from `data`, the player spawned and the
/// terrain generated.
pub fn load_from(data: &Path, code: Code) -> Result<Simulation, Box<dyn Error>> {
    load_world(data, code, None, None).map(|(sim, _)| sim)
}

/// The engine with the base mod's data half read from `data` and the player spawned, then
/// either the terrain generated or, given a save, the world put back as it was saved. Also
/// returns the world as the game built it, before the terrain, to start fresh worlds from.
pub fn load_world(
    data: &Path,
    code: Code,
    save: Option<&SaveData>,
    store: Option<std::sync::Arc<dyn sim::ChunkStore>>,
) -> Result<(Simulation, SaveData), Box<dyn Error>> {
    let data = data_mods::load(data)?;
    let wasm_cow = match code.brain {
        Where::Native => None,
        Where::Wasm => {
            let (name, brain) = wasm_host::load_brain(BASE_COW_WASM)?;
            Some(WasmCow(brain, name))
        }
    };
    let mut mods: Vec<&dyn Mod> = vec![&data, &base::BaseMod];
    match &wasm_cow {
        Some(cow) => mods.push(cow),
        None => mods.push(&base_cow::BaseCowMod),
    }
    if code.movement == Where::Native {
        mods.push(&base_movement::BaseMovementMod);
    }
    let mut sim = Simulation::with_mods(&mods)?;
    if code.movement == Where::Wasm {
        let (name, controller) = wasm_host::load_movement(BASE_MOVEMENT_WASM)?;
        sim.add_movement_controller(&name, Box::new(controller))?;
    }
    sim.spawn_player()?;
    if let Some(store) = store {
        sim.set_chunk_store(store);
    }
    let fresh = sim.snapshot();
    match save {
        None => {
            sim.generate_terrain()?;
        }
        Some(save) => {
            for skipped in sim.load(save)?.skipped {
                eprintln!("loading the world: left out {skipped}");
            }
        }
    }
    let stats = sim.stream_stats();
    eprintln!(
        "terrain: {} columns loaded around the player at start ({:.2} s on the workers); the rest streams in",
        stats.columns, stats.seconds
    );
    Ok((sim, fresh))
}
