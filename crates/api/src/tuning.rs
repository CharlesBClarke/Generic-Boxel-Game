//! Live tuning. Anything with values worth adjusting while the game runs lists them here,
//! and the engine's tuning panel draws a slider for each. Plain data both ways, so it works
//! the same across a WebAssembly boundary.

/// One adjustable value.
#[derive(Clone, Debug, PartialEq)]
pub struct Tunable {
    /// Short identifier, such as `"ground_accel"`. Stable, so values can be saved and restored.
    pub name: String,
    pub value: f32,
    pub min: f32,
    pub max: f32,
}

impl Tunable {
    pub fn new(name: impl Into<String>, value: f32, min: f32, max: f32) -> Self {
        Self { name: name.into(), value, min, max }
    }
}

pub trait Tunables {
    /// Every adjustable value with its current setting.
    fn tunables(&self) -> Vec<Tunable>;

    /// Changes one value. Returns false if there is no value by that name.
    fn set_tunable(&mut self, name: &str, value: f32) -> bool;
}
