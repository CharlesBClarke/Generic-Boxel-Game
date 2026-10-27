//! Test fixture: a movement controller that panics on its first step. Its tunables work, so
//! tests can check the engine copes before and after the trap.

use api::{MoveContext, MoveOutput, MovementController, Tunable, Tunables};

#[cfg(target_arch = "wasm32")]
api::export_movement_controller!("test:panicky", Panicky);

#[derive(Clone, Debug, Default)]
pub struct Panicky {
    pub speed: f32,
}

impl MovementController for Panicky {
    fn spawn(&self) -> Box<dyn MovementController> {
        Box::new(self.clone())
    }

    fn step(&mut self, _ctx: &MoveContext) -> MoveOutput {
        panic!("this controller always panics")
    }
}

impl Tunables for Panicky {
    fn tunables(&self) -> Vec<Tunable> {
        vec![Tunable::new("speed", self.speed, 0.0, 10.0)]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        if name != "speed" {
            return false;
        }
        self.speed = value;
        true
    }
}
