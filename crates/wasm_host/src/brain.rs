//! Brains running as WebAssembly components.

use api::{Attack, Brain, BrainContext, BrainOutput, Tunable, Tunables, Vec2, Vec3};

use crate::{LoadError, Limits, Sandbox, Shared, Startup, Stop};

mod bindings {
    wasmtime::component::bindgen!({ path: "../api/wit", world: "creature-brain" });
}

use bindings::CreatureBrain as Bindings;
use bindings::exports::boxel::api::brain as wit;

/// A brain running inside a WebAssembly component. Copies share the component, so one can be
/// handed to the engine and another kept for its fuel readout. If it traps or hits one of its
/// [`Limits`], the reason is printed once and every creature it drives stands still, keeping
/// its memory; its tunables disappear. The game carries on.
#[derive(Clone)]
pub struct WasmBrain(Shared<Bindings>);

/// Loads a brain component with the default [`Limits`]. Returns the name it registers under
/// and the brain.
pub fn load_brain(bytes: &[u8]) -> Result<(String, WasmBrain), LoadError> {
    load_brain_with(bytes, Limits::default())
}

/// Loads a brain component with the given limits.
pub fn load_brain_with(bytes: &[u8], limits: Limits) -> Result<(String, WasmBrain), LoadError> {
    let sandbox = Sandbox::load(
        bytes,
        limits,
        Startup {
            what: "brain",
            instantiate: |s, component, linker| Bindings::instantiate(s, component, linker),
            name: |b, s| b.boxel_api_brain().call_name(s),
            warm_up: |b, s| b.boxel_api_brain().call_tunables(s).map(drop),
        },
    )?;
    let name = sandbox.name.clone();
    Ok((name, WasmBrain(Shared::new(sandbox))))
}

impl WasmBrain {
    /// Why the component stopped, if it has.
    pub fn stopped(&self) -> Option<Stop> {
        self.0.stopped()
    }

    /// Fuel used by the most recent call into the component.
    pub fn fuel_used(&self) -> u64 {
        self.0.fuel_used()
    }
}

impl Brain for WasmBrain {
    fn think(&self, ctx: &BrainContext) -> BrainOutput {
        match self.0.call(|b, s| b.boxel_api_brain().call_think(s, &ctx.into())) {
            Some(out) => out.into(),
            None => BrainOutput { yaw: ctx.yaw, clip_speed: 1.0, memory: ctx.memory.clone(), ..Default::default() },
        }
    }
}

impl Tunables for WasmBrain {
    fn tunables(&self) -> Vec<Tunable> {
        let list = self.0.call(|b, s| b.boxel_api_brain().call_tunables(s));
        list.unwrap_or_default().into_iter().map(|t| Tunable { name: t.name, value: t.value, min: t.min, max: t.max, group: t.group, help: t.help }).collect()
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        self.0.call(|b, s| b.boxel_api_brain().call_set_tunable(s, name, value)).unwrap_or(false)
    }
}

fn vec3(v: Vec3) -> wit::Vec3 {
    wit::Vec3 { x: v.x, y: v.y, z: v.z }
}

fn from_vec3(v: wit::Vec3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

impl From<&BrainContext> for wit::BrainContext {
    fn from(c: &BrainContext) -> Self {
        wit::BrainContext {
            dt: c.dt,
            id: c.id,
            velocity: vec3(c.velocity),
            yaw: c.yaw,
            on_ground: c.on_ground,
            blocked: c.blocked,
            health: c.health,
            full_health: c.full_health,
            player: c.player.as_ref().map(|p| wit::Seen {
                offset: vec3(p.offset),
                velocity: vec3(p.velocity),
                clear_run: p.clear_run,
            }),
            hurt: c.hurt.as_ref().map(|h| wit::Hurt { from: vec3(h.from), damage: h.damage }),
            attack_landed: c.attack_landed,
            others: c
                .others
                .iter()
                .map(|n| wit::Neighbor {
                    offset: vec3(n.offset),
                    velocity: vec3(n.velocity),
                    kind: n.kind.clone(),
                    id: n.id,
                    signal: n.signal.clone(),
                    health: n.health,
                })
                .collect(),
            random: c.random,
            route: c.route.as_ref().map(|r| wit::Route {
                next: vec3(r.next),
                end: vec3(r.end),
                length: r.length,
                reaches: r.reaches,
                hop: r.hop,
            }),
            memory: c.memory.clone(),
        }
    }
}

impl From<wit::BrainOutput> for BrainOutput {
    fn from(o: wit::BrainOutput) -> Self {
        let array = |v: wit::Vec3| [v.x, v.y, v.z];
        BrainOutput {
            walk: Vec2::new(o.walk.x, o.walk.y),
            yaw: o.yaw,
            edge_guard: o.edge_guard,
            clip: o.clip,
            clip_speed: o.clip_speed,
            signal: o.signal,
            attack: o.attack.map(|a| Attack {
                min: array(a.min),
                max: array(a.max),
                damage: a.damage,
                push: a.push,
                lift: a.lift,
                guarded_damage: a.guarded_damage,
                guarded_push: a.guarded_push,
            }),
            go_to: o.go_to.map(from_vec3),
            hop: o.hop,
            memory: o.memory,
        }
    }
}
