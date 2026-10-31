//! Shared by the tests that place the base mod's blocks.

use api::{ApiError, BlockDef, Mod, Registrar};
use data_mods::DataMod;

/// The base mod's data half with every block placeable, so tests can place the shapes the
/// palette hides (stone slabs, logs, fence posts) to check how placement works.
pub struct AllPlaceable(pub DataMod);

impl AllPlaceable {
    pub fn load() -> Self {
        AllPlaceable(data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap())
    }
}

impl Mod for AllPlaceable {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        for look in self.0.looks() {
            r.register_look(look.clone())?;
        }
        for block in self.0.blocks() {
            r.register_block(BlockDef { placeable: true, ..block.clone() })?;
        }
        for model in self.0.models() {
            r.register_model(model.clone())?;
        }
        for sound in self.0.sounds() {
            r.register_sound(sound.clone())?;
        }
        for target in self.0.targets() {
            r.register_target(target.clone())?;
        }
        Ok(())
    }
}
