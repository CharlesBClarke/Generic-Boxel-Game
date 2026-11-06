//! Test fixture: a brain that misbehaves on purpose. Its `behavior` tunable picks what it does:
//!
//! - 0: behaves, walking east and counting its thinks in memory.
//! - 1: loops forever in `think`.
//! - 2: panics in `think`.

use std::hint::black_box;

use api::{Brain, BrainContext, BrainOutput, Tunable, Tunables, Vec2};

#[cfg(target_arch = "wasm32")]
api::export_brain!("test:hostile", Hostile);

#[derive(Clone, Debug, Default)]
pub struct Hostile {
    pub behavior: u32,
}

impl Brain for Hostile {
    fn think(&self, ctx: &BrainContext) -> BrainOutput {
        match self.behavior {
            1 => loop {
                black_box(());
            },
            2 => panic!("this brain panics when asked to"),
            _ => {}
        }
        let thinks = ctx.memory.first().copied().unwrap_or(0.0);
        BrainOutput { walk: Vec2::X, yaw: ctx.yaw, clip_speed: 1.0, memory: vec![thinks + 1.0], ..Default::default() }
    }
}

impl Tunables for Hostile {
    fn tunables(&self) -> Vec<Tunable> {
        vec![Tunable::new("behavior", self.behavior as f32, 0.0, 2.0)]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        if name != "behavior" {
            return false;
        }
        self.behavior = value as u32;
        true
    }
}
