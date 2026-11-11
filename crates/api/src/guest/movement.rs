//! The guest side of the movement boundary. See [`export_movement_controller!`](crate::export_movement_controller).

use std::cell::RefCell;

use crate::{Ground, MoveContext, MoveInput, MoveOutput, MovementController, Tunable, Vec2, Vec3};

wit_bindgen::generate!({
    path: "wit",
    world: "movement-controller",
    pub_export_macro: true,
    export_macro_name: "export_wit_movement",
    default_bindings_module: "api::guest::movement",
});

use exports::boxel::api::movement as wit;

/// Exports a [`MovementController`] under a namespaced name, such as `"base:quake"`. The
/// controller type must implement `Default`; the component holds one, made on its first call.
#[macro_export]
macro_rules! export_movement_controller {
    ($name:expr, $controller:ty) => {
        struct __MovementExport;

        fn __make_controller() -> ::std::boxed::Box<dyn $crate::MovementController> {
            ::std::boxed::Box::new(<$controller as ::core::default::Default>::default())
        }

        impl $crate::guest::movement::exports::boxel::api::movement::Guest for __MovementExport {
            fn name() -> ::std::string::String {
                ::std::string::String::from($name)
            }

            fn step(
                ctx: $crate::guest::movement::exports::boxel::api::movement::MoveContext,
            ) -> $crate::guest::movement::exports::boxel::api::movement::MoveOutput {
                $crate::guest::movement::step(__make_controller, ctx)
            }

            fn tunables() -> ::std::vec::Vec<$crate::guest::movement::exports::boxel::api::movement::Tunable> {
                $crate::guest::movement::tunables(__make_controller)
            }

            fn set_tunable(name: ::std::string::String, value: f32) -> bool {
                $crate::guest::movement::set_tunable(__make_controller, name, value)
            }
        }

        $crate::guest::movement::export_wit_movement!(__MovementExport with_types_in $crate::guest::movement);
    };
}

thread_local! {
    /// The component's one controller. WebAssembly here is single-threaded.
    static CONTROLLER: RefCell<Option<Box<dyn MovementController>>> = const { RefCell::new(None) };
}

fn with_controller<R>(make: fn() -> Box<dyn MovementController>, f: impl FnOnce(&mut dyn MovementController) -> R) -> R {
    CONTROLLER.with_borrow_mut(|c| f(c.get_or_insert_with(make).as_mut()))
}

pub fn step(make: fn() -> Box<dyn MovementController>, ctx: wit::MoveContext) -> wit::MoveOutput {
    with_controller(make, |c| c.step(&ctx.into()).into())
}

pub fn tunables(make: fn() -> Box<dyn MovementController>) -> Vec<wit::Tunable> {
    with_controller(make, |c| c.tunables().into_iter().map(Into::into).collect())
}

pub fn set_tunable(make: fn() -> Box<dyn MovementController>, name: String, value: f32) -> bool {
    with_controller(make, |c| c.set_tunable(&name, value))
}

impl From<wit::Vec2> for Vec2 {
    fn from(v: wit::Vec2) -> Self {
        Vec2::new(v.x, v.y)
    }
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

impl From<wit::MoveContext> for MoveContext {
    fn from(c: wit::MoveContext) -> Self {
        MoveContext {
            dt: c.dt,
            velocity: c.velocity.into(),
            height: c.height,
            standing_height: c.standing_height,
            input: MoveInput {
                movement: c.input.movement.into(),
                yaw: c.input.yaw,
                jump_pressed: c.input.jump_pressed,
                jump_held: c.input.jump_held,
                sprint: c.input.sprint,
                crouch: c.input.crouch,
            },
            ground: c.ground.map(|g| Ground { friction: g.friction }),
            memory: c.memory,
        }
    }
}

impl From<MoveOutput> for wit::MoveOutput {
    fn from(o: MoveOutput) -> Self {
        wit::MoveOutput {
            velocity: o.velocity.into(),
            impulse: o.impulse.into(),
            height: o.height,
            edge_guard: o.edge_guard,
            memory: o.memory,
        }
    }
}

impl From<Tunable> for wit::Tunable {
    fn from(t: Tunable) -> Self {
        wit::Tunable { name: t.name, value: t.value, min: t.min, max: t.max, group: t.group, help: t.help }
    }
}
