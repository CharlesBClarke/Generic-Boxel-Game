//! The engine side of the WebAssembly boundary. Loads a mod's components and wraps each in the
//! plain trait it exports ([`MovementController`](api::MovementController) in [`movement`],
//! [`Brain`](api::Brain) in [`brain`]), so the simulation and the tuning panel can't tell them
//! from native ones.
//!
//! A component imports nothing, so it can only compute: no files, no clock, no network. It
//! keeps no state between calls either; the engine passes each body's or creature's memory in.
//! [`Limits`] keep it from hanging the game or taking all its memory.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, ResourceLimiter, Store, Trap};

pub mod brain;
pub mod movement;

pub use brain::{WasmBrain, load_brain, load_brain_with};
pub use movement::{WasmController, load_movement, load_movement_with};

/// The most table elements a mod may have. Tables hold function references, so real mods
/// need few; this only stops a runaway.
const TABLE_ELEMENTS: usize = 1 << 20;

/// What a mod may use. Fuel is given afresh for every call, so one call's cost never depends
/// on another's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Fuel for each call into the mod, about one unit per WebAssembly instruction. Counted,
    /// not timed, so a mod runs out at the same point on every machine.
    pub fuel: u64,
    /// The most linear memory the mod may have, in bytes.
    pub memory: usize,
    /// The most WebAssembly stack one call may use, in bytes.
    pub stack: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { fuel: 10_000_000, memory: 256 << 20, stack: 512 << 10 }
    }
}

/// Why a mod stopped. Once stopped it is never called again: it may have been cut off
/// halfway through changing its own state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// It trapped on its own, for example by panicking.
    Trap,
    /// It used up a call's fuel, most likely stuck in a loop.
    OutOfFuel,
    /// It tried to grow past its memory limit.
    OutOfMemory,
    /// It used up its stack, most likely recursing without end.
    StackOverflow,
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Stop::Trap => "trapped",
            Stop::OutOfFuel => "ran out of fuel",
            Stop::OutOfMemory => "hit its memory limit",
            Stop::StackOverflow => "overflowed its stack",
        })
    }
}

/// Why a component couldn't be loaded.
#[derive(Debug)]
pub struct LoadError {
    /// What was being loaded, such as "movement controller".
    pub what: &'static str,
    /// Set when the component's own code failed or hit a limit while starting up.
    pub stop: Option<Stop>,
    pub source: wasmtime::Error,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.stop {
            Some(stop) => write!(f, "the {} {stop} while loading: {:?}", self.what, self.source),
            None => write!(f, "the {} couldn't be loaded: {:?}", self.what, self.source),
        }
    }
}

impl std::error::Error for LoadError {}

/// Per-store state: the memory limits, and whether a call tried to grow past them.
struct Caps {
    memory: usize,
    memory_refused: bool,
}

impl ResourceLimiter for Caps {
    fn memory_growing(&mut self, _current: usize, desired: usize, _maximum: Option<usize>) -> wasmtime::Result<bool> {
        let allow = desired <= self.memory;
        self.memory_refused |= !allow;
        Ok(allow)
    }

    fn table_growing(&mut self, _current: usize, desired: usize, _maximum: Option<usize>) -> wasmtime::Result<bool> {
        Ok(desired <= TABLE_ELEMENTS)
    }
}

/// One loaded component: its store and its bindings `B`, shared by every handle to it.
struct Sandbox<B> {
    store: Store<Caps>,
    bindings: B,
    /// What it is and its registered name, for messages.
    what: &'static str,
    name: String,
    fuel: u64,
    /// Fuel the most recent call used.
    fuel_used: u64,
    /// Set when a call fails. The instance may be left inconsistent, so it is never called again.
    stopped: Option<Stop>,
}

/// Runs one call into a component with a full tank of fuel, noting what it used and, if it
/// fails, why.
fn run<R>(
    store: &mut Store<Caps>,
    fuel: u64,
    f: impl FnOnce(&mut Store<Caps>) -> wasmtime::Result<R>,
) -> (Result<R, (Stop, wasmtime::Error)>, u64) {
    store.data_mut().memory_refused = false;
    store.set_fuel(fuel).expect("fuel is enabled");
    let result = f(store);
    let used = fuel - store.get_fuel().expect("fuel is enabled");
    let result = result.map_err(|e| {
        let stop = match e.downcast_ref::<Trap>() {
            Some(Trap::OutOfFuel) => Stop::OutOfFuel,
            Some(Trap::StackOverflow) => Stop::StackOverflow,
            // A guest whose allocation is refused usually aborts with a trap of its own.
            _ if store.data().memory_refused => Stop::OutOfMemory,
            _ => Stop::Trap,
        };
        (stop, e)
    });
    (result, used)
}

/// How a kind of component starts up: instantiating it, asking its name, and one call that
/// makes the thing it exports (its constructor runs on the first call), so a component that
/// can't start fails to load rather than later.
struct Startup<B> {
    what: &'static str,
    instantiate: fn(&mut Store<Caps>, &Component, &Linker<Caps>) -> wasmtime::Result<B>,
    name: fn(&B, &mut Store<Caps>) -> wasmtime::Result<String>,
    warm_up: fn(&B, &mut Store<Caps>) -> wasmtime::Result<()>,
}

impl<B> Sandbox<B> {
    fn load(bytes: &[u8], limits: Limits, startup: Startup<B>) -> Result<Self, LoadError> {
        let what = startup.what;
        let fail = |stop, source| LoadError { what, stop, source };
        let mut config = Config::new();
        config.wasm_component_model(true);
        // Floats must give the same bits on every machine, for replays and multiplayer.
        config.cranelift_nan_canonicalization(true);
        config.relaxed_simd_deterministic(true);
        config.consume_fuel(true);
        config.max_wasm_stack(limits.stack);
        let engine = Engine::new(&config).map_err(|e| fail(None, e))?;
        let component = Component::new(&engine, bytes).map_err(|e| fail(None, e))?;
        let linker = Linker::new(&engine);
        let mut store = Store::new(&engine, Caps { memory: limits.memory, memory_refused: false });
        store.limiter(|caps| caps);
        // Starting up runs the component's own code too, so each step gets fuel.
        fn started<R>(what: &'static str, r: Result<R, (Stop, wasmtime::Error)>) -> Result<R, LoadError> {
            r.map_err(|(stop, source)| LoadError { what, stop: Some(stop), source })
        }
        let bindings = started(what, run(&mut store, limits.fuel, |s| (startup.instantiate)(s, &component, &linker)).0)?;
        let name = started(what, run(&mut store, limits.fuel, |s| (startup.name)(&bindings, s)).0)?;
        let (warm, fuel_used) = run(&mut store, limits.fuel, |s| (startup.warm_up)(&bindings, s));
        started(what, warm)?;
        Ok(Sandbox { store, bindings, what, name, fuel: limits.fuel, fuel_used, stopped: None })
    }

    /// Calls into the component, unless it has stopped. A failure stops it for good and is
    /// printed once.
    fn call<R>(&mut self, f: impl FnOnce(&B, &mut Store<Caps>) -> wasmtime::Result<R>) -> Option<R> {
        if self.stopped.is_some() {
            return None;
        }
        let bindings = &self.bindings;
        let (result, used) = run(&mut self.store, self.fuel, |s| f(bindings, s));
        self.fuel_used = used;
        match result {
            Ok(r) => Some(r),
            Err((stop, e)) => {
                self.stopped = Some(stop);
                eprintln!("{} {:?} {stop} and has stopped: {e:?}", self.what, self.name);
                None
            }
        }
    }
}

/// A handle to a loaded component, shared by every copy.
struct Shared<B>(Arc<Mutex<Sandbox<B>>>);

impl<B> Clone for Shared<B> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<B> Shared<B> {
    fn new(sandbox: Sandbox<B>) -> Self {
        Self(Arc::new(Mutex::new(sandbox)))
    }

    fn lock(&self) -> MutexGuard<'_, Sandbox<B>> {
        // A panic while holding the lock can't leave the sandbox half-updated from our side;
        // the component's own state is covered by `stopped`.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn call<R>(&self, f: impl FnOnce(&B, &mut Store<Caps>) -> wasmtime::Result<R>) -> Option<R> {
        self.lock().call(f)
    }

    fn stopped(&self) -> Option<Stop> {
        self.lock().stopped
    }

    fn fuel_used(&self) -> u64 {
        self.lock().fuel_used
    }
}
