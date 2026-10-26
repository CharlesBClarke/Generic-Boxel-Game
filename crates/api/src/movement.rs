//! Movement controllers decide how a body moves: how input turns into velocity, how gravity
//! and friction act. The engine runs the controller each tick, then applies impulses and
//! resolves collisions, so effects from any mod work with any controller.

use glam::{Vec2, Vec3};

use crate::Tunables;

/// One tick's worth of intent for a body.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MoveInput {
    /// Horizontal movement in the body's frame: +x right, +y forward. Length at most 1.
    pub movement: Vec2,
    /// Facing, radians counterclockwise from +X (east) around +Z.
    pub yaw: f32,
    /// The jump key went down since the last tick.
    pub jump_pressed: bool,
    /// The jump key is down.
    pub jump_held: bool,
    pub sprint: bool,
    /// The crouch key is down.
    pub crouch: bool,
}

impl MoveInput {
    /// The horizontal direction the input asks for, in world space (Z-up). Length at most 1.
    pub fn wish_direction(&self) -> Vec3 {
        let (sin, cos) = self.yaw.sin_cos();
        let forward = Vec3::new(cos, sin, 0.0);
        let right = Vec3::new(sin, -cos, 0.0);
        let movement = self.movement.clamp_length_max(1.0);
        right * movement.x + forward * movement.y
    }
}

/// What the body is standing on.
#[derive(Clone, Debug, PartialEq)]
pub struct Ground {
    /// Friction of the block underfoot, from its definition.
    pub friction: f32,
}

/// Everything a controller sees for one tick.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveContext {
    /// Seconds this tick covers.
    pub dt: f32,
    pub velocity: Vec3,
    /// The body's current height, meters. Below `standing_height` while crouched, or while
    /// something overhead keeps it from standing up.
    pub height: f32,
    /// The body's full height, from its definition.
    pub standing_height: f32,
    pub input: MoveInput,
    /// `Some` while the body is standing on something solid.
    pub ground: Option<Ground>,
}

/// A controller's decision for one tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MoveOutput {
    /// The body's new velocity, before impulses.
    pub velocity: Vec3,
    /// An instant velocity change, added together with impulses from anywhere else.
    /// Jumps go here, so every source of sudden motion takes the same path.
    pub impulse: Vec3,
    /// The body height the controller wants, meters; `None` is standing height. The engine
    /// always allows shrinking but only grows the body as far as there is room. On the ground
    /// the feet stay put; in the air the head does, so crouching mid-air lifts the feet.
    pub height: Option<f32>,
}

/// Decides how one kind of body moves. One instance per body, so it can keep its own
/// state between ticks (timers, for example).
pub trait MovementController: Tunables + Send + Sync {
    /// A fresh controller for a newly spawned body, with the same settings as this one.
    fn spawn(&self) -> Box<dyn MovementController>;

    /// Advances one tick.
    fn step(&mut self, ctx: &MoveContext) -> MoveOutput;
}
