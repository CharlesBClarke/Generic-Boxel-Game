//! Movement controllers running as WebAssembly components.

use api::{Ground, MoveContext, MoveInput, MoveOutput, MovementController, Tunable, Tunables, Vec2, Vec3};

use crate::{LoadError, Limits, Sandbox, Shared, Startup, Stop};

mod bindings {
    wasmtime::component::bindgen!({ path: "../api/wit", world: "movement-controller" });
}

use bindings::MovementController as Bindings;
use bindings::exports::boxel::api::movement as wit;

/// A movement controller running inside a WebAssembly component. Copies share the component.
/// If it traps or hits one of its [`Limits`], the reason is printed once and it stops
/// steering: velocity passes through unchanged and its tunables disappear. The game carries on.
#[derive(Clone)]
pub struct WasmController(Shared<Bindings>);

/// Loads a movement controller component with the default [`Limits`]. Returns the name it
/// registers under and the controller.
pub fn load_movement(bytes: &[u8]) -> Result<(String, WasmController), LoadError> {
    load_movement_with(bytes, Limits::default())
}

/// Loads a movement controller component with the given limits.
pub fn load_movement_with(bytes: &[u8], limits: Limits) -> Result<(String, WasmController), LoadError> {
    let sandbox = Sandbox::load(
        bytes,
        limits,
        Startup {
            what: "movement controller",
            instantiate: |s, component, linker| Bindings::instantiate(s, component, linker),
            name: |b, s| b.boxel_api_movement().call_name(s),
            warm_up: |b, s| b.boxel_api_movement().call_tunables(s).map(drop),
        },
    )?;
    let name = sandbox.name.clone();
    Ok((name, WasmController(Shared::new(sandbox))))
}

impl WasmController {
    /// Why the component stopped, if it has.
    pub fn stopped(&self) -> Option<Stop> {
        self.0.stopped()
    }

    /// Fuel used by the most recent call into the component.
    pub fn fuel_used(&self) -> u64 {
        self.0.fuel_used()
    }
}

impl MovementController for WasmController {
    fn step(&self, ctx: &MoveContext) -> MoveOutput {
        match self.0.call(|b, s| b.boxel_api_movement().call_step(s, &ctx.clone().into())) {
            Some(out) => out.into(),
            None => MoveOutput { velocity: ctx.velocity, ..Default::default() },
        }
    }
}

impl Tunables for WasmController {
    fn tunables(&self) -> Vec<Tunable> {
        let list = self.0.call(|b, s| b.boxel_api_movement().call_tunables(s));
        list.unwrap_or_default().into_iter().map(Into::into).collect()
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        self.0.call(|b, s| b.boxel_api_movement().call_set_tunable(s, name, value)).unwrap_or(false)
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
            memory: c.memory,
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
            memory: o.memory,
        }
    }
}

impl From<wit::Tunable> for Tunable {
    fn from(t: wit::Tunable) -> Self {
        Tunable { name: t.name, value: t.value, min: t.min, max: t.max }
    }
}
