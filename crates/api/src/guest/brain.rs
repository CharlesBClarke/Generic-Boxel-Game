//! The guest side of the brain boundary. A mod compiled to WebAssembly writes a plain
//! [`Brain`] and exports it with [`export_brain!`](crate::export_brain); this module turns the
//! component calls into trait calls and converts the records both ways.

use std::cell::RefCell;

use crate::{Attack, Brain, BrainContext, BrainOutput, Hurt, Neighbor, Route, Seen, Tunable, Vec3};

wit_bindgen::generate!({
    path: "wit",
    world: "creature-brain",
    pub_export_macro: true,
    export_macro_name: "export_wit_brain",
    default_bindings_module: "api::guest::brain",
});

use exports::boxel::api::brain as wit;

/// Exports a [`Brain`] under a namespaced name, such as `"base:cow"`. The brain type must
/// implement `Default`; the component holds one, made on its first call.
#[macro_export]
macro_rules! export_brain {
    ($name:expr, $brain:ty) => {
        struct __BrainExport;

        fn __make_brain() -> ::std::boxed::Box<dyn $crate::Brain> {
            ::std::boxed::Box::new(<$brain as ::core::default::Default>::default())
        }

        impl $crate::guest::brain::exports::boxel::api::brain::Guest for __BrainExport {
            fn name() -> ::std::string::String {
                ::std::string::String::from($name)
            }

            fn think(
                ctx: $crate::guest::brain::exports::boxel::api::brain::BrainContext,
            ) -> $crate::guest::brain::exports::boxel::api::brain::BrainOutput {
                $crate::guest::brain::think(__make_brain, ctx)
            }

            fn tunables() -> ::std::vec::Vec<$crate::guest::brain::exports::boxel::api::brain::Tunable> {
                $crate::guest::brain::tunables(__make_brain)
            }

            fn set_tunable(name: ::std::string::String, value: f32) -> bool {
                $crate::guest::brain::set_tunable(__make_brain, name, value)
            }
        }

        $crate::guest::brain::export_wit_brain!(__BrainExport with_types_in $crate::guest::brain);
    };
}

thread_local! {
    /// The component's one brain. WebAssembly here is single-threaded.
    static BRAIN: RefCell<Option<Box<dyn Brain>>> = const { RefCell::new(None) };
}

fn with_brain<R>(make: fn() -> Box<dyn Brain>, f: impl FnOnce(&mut dyn Brain) -> R) -> R {
    BRAIN.with_borrow_mut(|brain| f(brain.get_or_insert_with(make).as_mut()))
}

pub fn think(make: fn() -> Box<dyn Brain>, ctx: wit::BrainContext) -> wit::BrainOutput {
    with_brain(make, |brain| brain.think(&ctx.into()).into())
}

pub fn tunables(make: fn() -> Box<dyn Brain>) -> Vec<wit::Tunable> {
    with_brain(make, |brain| brain.tunables().into_iter().map(Into::into).collect())
}

pub fn set_tunable(make: fn() -> Box<dyn Brain>, name: String, value: f32) -> bool {
    with_brain(make, |brain| brain.set_tunable(&name, value))
}

impl From<wit::Vec3> for Vec3 {
    fn from(v: wit::Vec3) -> Self {
        Vec3::new(v.x, v.y, v.z)
    }
}

impl From<Vec3> for wit::Vec3 {
    fn from(v: Vec3) -> Self {
        wit::Vec3 { x: v.x, y: v.y, z: v.z }
    }
}

fn to_wit3(v: [f32; 3]) -> wit::Vec3 {
    wit::Vec3 { x: v[0], y: v[1], z: v[2] }
}

impl From<wit::BrainContext> for BrainContext {
    fn from(c: wit::BrainContext) -> Self {
        BrainContext {
            dt: c.dt,
            id: c.id,
            velocity: c.velocity.into(),
            yaw: c.yaw,
            on_ground: c.on_ground,
            blocked: c.blocked,
            health: c.health,
            full_health: c.full_health,
            player: c.player.map(|p| Seen { offset: p.offset.into(), velocity: p.velocity.into(), clear_run: p.clear_run }),
            hurt: c.hurt.map(|h| Hurt { from: h.from.into(), damage: h.damage }),
            attack_landed: c.attack_landed,
            others: c
                .others
                .into_iter()
                .map(|n| Neighbor {
                    offset: n.offset.into(),
                    velocity: n.velocity.into(),
                    kind: n.kind,
                    id: n.id,
                    signal: n.signal,
                    health: n.health,
                })
                .collect(),
            random: c.random,
            route: c.route.map(|r| Route { next: r.next.into(), end: r.end.into(), length: r.length, reaches: r.reaches, hop: r.hop }),
            memory: c.memory,
        }
    }
}

impl From<BrainOutput> for wit::BrainOutput {
    fn from(o: BrainOutput) -> Self {
        wit::BrainOutput {
            walk: wit::Vec2 { x: o.walk.x, y: o.walk.y },
            yaw: o.yaw,
            edge_guard: o.edge_guard,
            clip: o.clip,
            clip_speed: o.clip_speed,
            signal: o.signal,
            attack: o.attack.map(|a: Attack| wit::Attack {
                min: to_wit3(a.min),
                max: to_wit3(a.max),
                damage: a.damage,
                push: a.push,
                lift: a.lift,
                guarded_damage: a.guarded_damage,
                guarded_push: a.guarded_push,
            }),
            go_to: o.go_to.map(Into::into),
            hop: o.hop,
            memory: o.memory,
        }
    }
}

impl From<Tunable> for wit::Tunable {
    fn from(t: Tunable) -> Self {
        wit::Tunable { name: t.name, value: t.value, min: t.min, max: t.max }
    }
}
