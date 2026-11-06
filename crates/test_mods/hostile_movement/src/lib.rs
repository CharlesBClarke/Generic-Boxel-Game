//! Test fixture: a movement controller that misbehaves on purpose. Its `behavior` tunable
//! picks what it does, so one component covers every limit:
//!
//! - 0: behaves, passing velocity through.
//! - 1: loops forever in `step`.
//! - 2: allocates forever in `step`.
//! - 3: recurses forever in `step`.
//! - 5: loops forever in `tunables`.
//! - 6: allocates `allocate_mib` mebibytes in `step` and keeps them.
//!
//! (4 looped in `spawn`, which controllers no longer have.)

use std::hint::black_box;
use std::sync::Mutex;

use api::{MoveContext, MoveOutput, MovementController, Tunable, Tunables};

#[cfg(target_arch = "wasm32")]
api::export_movement_controller!("test:hostile", Hostile);

#[derive(Debug, Default)]
pub struct Hostile {
    pub behavior: u32,
    pub allocate_mib: u32,
    /// Memory kept by behavior 6, so it stays allocated.
    kept: Mutex<Vec<Vec<u8>>>,
}

fn spin() -> ! {
    loop {
        black_box(());
    }
}

fn recurse(depth: u64) -> u64 {
    // Never true in practice; it only stops the compiler seeing the recursion as endless.
    if depth == u64::MAX {
        return 0;
    }
    black_box(recurse(black_box(depth + 1))) + 1
}

impl MovementController for Hostile {
    fn step(&self, ctx: &MoveContext) -> MoveOutput {
        match self.behavior {
            1 => spin(),
            2 => loop {
                // Reserved but never touched, so only memory growth costs anything.
                std::mem::forget(black_box(Vec::<u8>::with_capacity(16 << 20)));
            },
            3 => {
                black_box(recurse(0));
            }
            6 => self.kept.lock().unwrap().push(black_box(Vec::with_capacity((self.allocate_mib as usize) << 20))),
            _ => {}
        }
        MoveOutput { velocity: ctx.velocity, ..Default::default() }
    }
}

impl Tunables for Hostile {
    fn tunables(&self) -> Vec<Tunable> {
        if self.behavior == 5 {
            spin();
        }
        vec![
            Tunable::new("behavior", self.behavior as f32, 0.0, 6.0),
            Tunable::new("allocate_mib", self.allocate_mib as f32, 0.0, 1024.0),
        ]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "behavior" => self.behavior = value as u32,
            "allocate_mib" => self.allocate_mib = value as u32,
            _ => return false,
        }
        true
    }
}
