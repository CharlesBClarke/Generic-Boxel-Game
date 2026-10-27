//! The guest side of the WebAssembly boundary. A mod compiled to WebAssembly writes a plain
//! [`MovementController`] and exports it with [`export_movement_controller!`]; this module
//! turns the component calls into trait calls and converts the records both ways.

use std::cell::RefCell;

use crate::{Ground, MoveContext, MoveInput, MoveOutput, MovementController, Tunable, Vec2, Vec3};

wit_bindgen::generate!({
    path: "wit",
    world: "movement-controller",
    pub_export_macro: true,
    export_macro_name: "export_wit_movement",
    default_bindings_module: "api::guest",
});

use exports::boxel::api::movement as wit;

/// Exports a [`MovementController`] under a namespaced name, such as `"base:quake"`. The
/// controller type must implement `Default`; the engine spawns every body's controller from
/// the default one.
#[macro_export]
macro_rules! export_movement_controller {
    ($name:expr, $controller:ty) => {
        struct __MovementExport;

        impl $crate::guest::exports::boxel::api::movement::Guest for __MovementExport {
            type Controller = $crate::guest::Controller;

            fn name() -> ::std::string::String {
                ::std::string::String::from($name)
            }

            fn create() -> $crate::guest::exports::boxel::api::movement::Controller {
                $crate::guest::Controller::wrap(::std::boxed::Box::new(<$controller as ::core::default::Default>::default()))
            }
        }

        $crate::guest::export_wit_movement!(__MovementExport with_types_in $crate::guest);
    };
}

/// One body's controller behind a component resource. Resource calls take `&self`, so the
/// controller sits in a `RefCell`; WebAssembly here is single-threaded.
pub struct Controller(RefCell<Box<dyn MovementController>>);

impl Controller {
    pub fn wrap(controller: Box<dyn MovementController>) -> wit::Controller {
        wit::Controller::new(Self(RefCell::new(controller)))
    }
}

impl wit::GuestController for Controller {
    fn spawn(&self) -> wit::Controller {
        Self::wrap(self.0.borrow().spawn())
    }

    fn step(&self, ctx: wit::MoveContext) -> wit::MoveOutput {
        self.0.borrow_mut().step(&ctx.into()).into()
    }

    fn tunables(&self) -> Vec<wit::Tunable> {
        self.0.borrow().tunables().into_iter().map(Into::into).collect()
    }

    fn set_tunable(&self, name: String, value: f32) -> bool {
        self.0.borrow_mut().set_tunable(&name, value)
    }
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
        }
    }
}

impl From<Tunable> for wit::Tunable {
    fn from(t: Tunable) -> Self {
        wit::Tunable { name: t.name, value: t.value, min: t.min, max: t.max }
    }
}
