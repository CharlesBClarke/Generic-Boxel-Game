//! Test fixture: a movement controller that never finishes being created, so loading the
//! component (which makes the controller) loops forever unless the engine stops it.

use std::hint::black_box;

use api::{MoveContext, MoveOutput, MovementController, Tunable, Tunables};

#[cfg(target_arch = "wasm32")]
api::export_movement_controller!("test:stuck", Stuck);

#[derive(Clone, Debug)]
pub struct Stuck;

impl Default for Stuck {
    fn default() -> Self {
        loop {
            black_box(());
        }
    }
}

impl MovementController for Stuck {
    fn step(&self, ctx: &MoveContext) -> MoveOutput {
        MoveOutput { velocity: ctx.velocity, ..Default::default() }
    }
}

impl Tunables for Stuck {
    fn tunables(&self) -> Vec<Tunable> {
        Vec::new()
    }

    fn set_tunable(&mut self, _name: &str, _value: f32) -> bool {
        false
    }
}
