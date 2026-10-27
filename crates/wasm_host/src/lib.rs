//! The engine side of the WebAssembly boundary. Loads a mod's movement controller component
//! and wraps it in the plain [`MovementController`] trait, so the simulation and the tuning
//! panel can't tell it from a native one.
//!
//! The component imports nothing, so it can only compute: no files, no clock, no network.

use std::sync::{Arc, Mutex, MutexGuard};

use api::{Ground, MoveContext, MoveInput, MoveOutput, MovementController, Tunable, Tunables, Vec2, Vec3};
use wasmtime::component::{Component, Linker, ResourceAny};
use wasmtime::{Config, Engine, Store};

mod bindings {
    wasmtime::component::bindgen!({ path: "../api/wit", world: "movement-controller" });
}

use bindings::MovementController as Bindings;
use bindings::exports::boxel::api::movement as wit;

/// One loaded component: its store and instance, shared by every controller spawned from it.
struct Instance {
    store: Store<()>,
    bindings: Bindings,
    /// The component's controller name, for messages.
    name: String,
    /// Set after a trap. The instance may be left inconsistent, so it is never called again.
    trapped: bool,
}

/// A movement controller running inside a WebAssembly component. If the component traps,
/// the error is printed once and the controller stops steering: velocity passes through
/// unchanged and its tunables disappear. The game carries on.
pub struct WasmController {
    instance: Arc<Mutex<Instance>>,
    /// The controller resource inside the component; `None` once trapped.
    handle: Option<ResourceAny>,
}

/// Loads a movement controller component. Returns the name it registers under and the
/// controller with its default settings; bodies get theirs from its `spawn`.
pub fn load_movement(bytes: &[u8]) -> wasmtime::Result<(String, WasmController)> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    // Floats must give the same bits on every machine, for replays and multiplayer.
    config.cranelift_nan_canonicalization(true);
    config.relaxed_simd_deterministic(true);
    let engine = Engine::new(&config)?;
    let component = Component::new(&engine, bytes)?;
    let linker = Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    let bindings = Bindings::instantiate(&mut store, &component, &linker)?;
    let name = bindings.boxel_api_movement().call_name(&mut store)?;
    let handle = bindings.boxel_api_movement().call_create(&mut store)?;
    let instance = Instance { store, bindings, name: name.clone(), trapped: false };
    Ok((name, WasmController { instance: Arc::new(Mutex::new(instance)), handle: Some(handle) }))
}

impl WasmController {
    /// Whether the component has trapped and this controller has stopped.
    pub fn trapped(&self) -> bool {
        self.lock().trapped
    }

    fn lock(&self) -> MutexGuard<'_, Instance> {
        // A panic while holding the lock can't leave the instance half-updated from our side;
        // the component's own state is covered by `trapped`.
        self.instance.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Calls into the component, unless it has trapped. A trap marks the whole instance as
    /// trapped and is printed once.
    fn call<R>(
        &self,
        f: impl FnOnce(&mut Store<()>, wit::GuestController<'_>, ResourceAny) -> wasmtime::Result<R>,
    ) -> Option<R> {
        let handle = self.handle?;
        let mut guard = self.lock();
        let instance = &mut *guard;
        if instance.trapped {
            return None;
        }
        match f(&mut instance.store, instance.bindings.boxel_api_movement().controller(), handle) {
            Ok(r) => Some(r),
            Err(e) => {
                instance.trapped = true;
                eprintln!("movement controller {:?} trapped and has stopped: {e:?}", instance.name);
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
        let mut instance = self.lock();
        if !instance.trapped {
            // Failing to free it only leaks the controller inside the component.
            let _ = handle.resource_drop(&mut instance.store);
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
