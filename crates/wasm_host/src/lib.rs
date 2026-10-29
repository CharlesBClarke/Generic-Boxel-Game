//! The engine side of the WebAssembly boundary. Loads a mod's movement controller component
//! and wraps it in the plain [`MovementController`] trait, so the simulation and the tuning
//! panel can't tell it from a native one.
//!
//! The component imports nothing, so it can only compute: no files, no clock, no network.
//! [`Limits`] keep it from hanging the game or taking all its memory.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use api::{Ground, MoveContext, MoveInput, MoveOutput, MovementController, Tunable, Tunables, Vec2, Vec3};
use wasmtime::component::{Component, Linker, ResourceAny};
use wasmtime::{Config, Engine, ResourceLimiter, Store, Trap};

mod bindings {
    wasmtime::component::bindgen!({ path: "../api/wit", world: "movement-controller" });
}

use bindings::MovementController as Bindings;
use bindings::exports::boxel::api::movement as wit;

/// The most table elements a mod may have. Tables hold function references, so real mods
/// need few; this only stops a runaway.
const TABLE_ELEMENTS: usize = 1 << 20;

/// What a mod may use. A mod's controllers share one instance, so they share its memory;
/// fuel is given afresh for every call, so one call's cost never depends on another's.
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

/// One loaded component: its store and instance, shared by every controller spawned from it.
struct Instance {
    store: Store<Caps>,
    bindings: Bindings,
    /// The component's controller name, for messages.
    name: String,
    fuel: u64,
    /// Fuel the most recent call used.
    fuel_used: u64,
    /// Set when a call fails. The instance may be left inconsistent, so it is never called again.
    stopped: Option<Stop>,
}

impl Instance {
    /// Stops the instance for good, printing why once.
    fn stop(&mut self, stop: Stop, e: &wasmtime::Error) {
        self.stopped = Some(stop);
        eprintln!("movement controller {:?} {stop} and has stopped: {e:?}", self.name);
    }

    /// Runs one call into the component with a full tank of fuel, noting what it used and,
    /// if it fails, why.
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
}

/// A movement controller running inside a WebAssembly component. If the component traps or
/// hits one of its [`Limits`], the reason is printed once and the controller stops steering:
/// velocity passes through unchanged and its tunables disappear. The game carries on.
pub struct WasmController {
    instance: Arc<Mutex<Instance>>,
    /// The controller resource inside the component; `None` once stopped.
    handle: Option<ResourceAny>,
}

/// Why a component couldn't be loaded.
#[derive(Debug)]
pub struct LoadError {
    /// Set when the component's own code failed or hit a limit while starting up.
    pub stop: Option<Stop>,
    pub source: wasmtime::Error,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.stop {
            Some(stop) => write!(f, "the movement controller {stop} while loading: {:?}", self.source),
            None => write!(f, "the movement controller couldn't be loaded: {:?}", self.source),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<wasmtime::Error> for LoadError {
    fn from(source: wasmtime::Error) -> Self {
        LoadError { stop: None, source }
    }
}

impl From<(Stop, wasmtime::Error)> for LoadError {
    fn from((stop, source): (Stop, wasmtime::Error)) -> Self {
        LoadError { stop: Some(stop), source }
    }
}

/// Loads a movement controller component with the default [`Limits`]. Returns the name it
/// registers under and the controller with its default settings; bodies get theirs from its
/// `spawn`.
pub fn load_movement(bytes: &[u8]) -> Result<(String, WasmController), LoadError> {
    load_movement_with(bytes, Limits::default())
}

/// Loads a movement controller component with the given limits.
pub fn load_movement_with(bytes: &[u8], limits: Limits) -> Result<(String, WasmController), LoadError> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    // Floats must give the same bits on every machine, for replays and multiplayer.
    config.cranelift_nan_canonicalization(true);
    config.relaxed_simd_deterministic(true);
    config.consume_fuel(true);
    config.max_wasm_stack(limits.stack);
    let engine = Engine::new(&config)?;
    let component = Component::new(&engine, bytes)?;
    let linker = Linker::new(&engine);
    let mut store = Store::new(&engine, Caps { memory: limits.memory, memory_refused: false });
    store.limiter(|caps| caps);
    // Starting up runs the component's own code too, so each step gets fuel.
    let bindings = Instance::run(&mut store, limits.fuel, |s| Bindings::instantiate(s, &component, &linker)).0?;
    let name = Instance::run(&mut store, limits.fuel, |s| bindings.boxel_api_movement().call_name(s)).0?;
    let (handle, fuel_used) = Instance::run(&mut store, limits.fuel, |s| bindings.boxel_api_movement().call_create(s));
    let handle = handle?;
    let instance = Instance { store, bindings, name: name.clone(), fuel: limits.fuel, fuel_used, stopped: None };
    Ok((name, WasmController { instance: Arc::new(Mutex::new(instance)), handle: Some(handle) }))
}

impl WasmController {
    /// Why the component stopped, if it has. All controllers from one component stop together.
    pub fn stopped(&self) -> Option<Stop> {
        self.lock().stopped
    }

    /// Fuel used by the most recent call into the component, by any of its controllers.
    pub fn fuel_used(&self) -> u64 {
        self.lock().fuel_used
    }

    fn lock(&self) -> MutexGuard<'_, Instance> {
        // A panic while holding the lock can't leave the instance half-updated from our side;
        // the component's own state is covered by `stopped`.
        self.instance.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Calls into the component, unless it has stopped. A failure stops the whole instance
    /// and is printed once.
    fn call<R>(
        &self,
        f: impl FnOnce(&mut Store<Caps>, wit::GuestController<'_>, ResourceAny) -> wasmtime::Result<R>,
    ) -> Option<R> {
        let handle = self.handle?;
        let mut guard = self.lock();
        let instance = &mut *guard;
        if instance.stopped.is_some() {
            return None;
        }
        let controller = instance.bindings.boxel_api_movement().controller();
        let (result, used) = Instance::run(&mut instance.store, instance.fuel, |s| f(s, controller, handle));
        instance.fuel_used = used;
        match result {
            Ok(r) => Some(r),
            Err((stop, e)) => {
                instance.stop(stop, &e);
                None
            }
        }
    }
}

impl MovementController for WasmController {
    fn spawn(&self) -> Box<dyn MovementController> {
        let handle = self.call(|store, c, handle| c.call_spawn(store, handle));
        Box::new(WasmController { instance: self.instance.clone(), handle })
    }

    fn step(&mut self, ctx: &MoveContext) -> MoveOutput {
        match self.call(|store, c, handle| c.call_step(store, handle, ctx.clone().into())) {
            Some(out) => out.into(),
            None => MoveOutput { velocity: ctx.velocity, ..Default::default() },
        }
    }
}

impl Tunables for WasmController {
    fn tunables(&self) -> Vec<Tunable> {
        let list = self.call(|store, c, handle| c.call_tunables(store, handle));
        list.unwrap_or_default().into_iter().map(Into::into).collect()
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        self.call(|store, c, handle| c.call_set_tunable(store, handle, name, value)).unwrap_or(false)
    }
}

impl Drop for WasmController {
    fn drop(&mut self) {
        let Some(handle) = self.handle else { return };
        let mut guard = self.lock();
        let instance = &mut *guard;
        if instance.stopped.is_none() {
            // Freeing runs the component's own code, so it gets fuel like any other call.
            // Failing only leaks the controller inside the component, but it still stops it.
            let (result, _) = Instance::run(&mut instance.store, instance.fuel, |s| handle.resource_drop(s));
            if let Err((stop, e)) = result {
                instance.stop(stop, &e);
            }
        }
    }
}

impl From<Vec3> for wit::Vec3 {
    fn from(v: Vec3) -> Self {
        wit::Vec3 { x: v.x, y: v.y, z: v.z }
    }
}

impl From<wit::Vec3> for Vec3 {
    fn from(v: wit::Vec3) -> Self {
        Vec3::new(v.x, v.y, v.z)
    }
}

impl From<Vec2> for wit::Vec2 {
    fn from(v: Vec2) -> Self {
        wit::Vec2 { x: v.x, y: v.y }
    }
}

impl From<MoveInput> for wit::MoveInput {
    fn from(i: MoveInput) -> Self {
        wit::MoveInput {
            movement: i.movement.into(),
            yaw: i.yaw,
            jump_pressed: i.jump_pressed,
            jump_held: i.jump_held,
            sprint: i.sprint,
            crouch: i.crouch,
        }
    }
}

impl From<MoveContext> for wit::MoveContext {
    fn from(c: MoveContext) -> Self {
        wit::MoveContext {
            dt: c.dt,
            velocity: c.velocity.into(),
            height: c.height,
            standing_height: c.standing_height,
            input: c.input.into(),
            ground: c.ground.map(|Ground { friction }| wit::Ground { friction }),
        }
    }
}

impl From<wit::MoveOutput> for MoveOutput {
    fn from(o: wit::MoveOutput) -> Self {
        MoveOutput {
            velocity: o.velocity.into(),
            impulse: o.impulse.into(),
            height: o.height,
            edge_guard: o.edge_guard,
        }
    }
}

impl From<wit::Tunable> for Tunable {
    fn from(t: wit::Tunable) -> Self {
        Tunable { name: t.name, value: t.value, min: t.min, max: t.max }
    }
}
