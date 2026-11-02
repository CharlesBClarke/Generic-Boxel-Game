//! Shared by the tests that place the base mod's blocks.

#![allow(dead_code)]

use api::{ApiError, BlockDef, Mod, Registrar, ShapeGrid};
use data_mods::DataMod;
use sim::{BlockId, Simulation};

/// The block of `material` in `shape`, which the base test area already built with.
pub fn shaped(sim: &Simulation, material: &str, shape: &ShapeGrid) -> BlockId {
    let blocks = sim.blocks();
    let m = blocks.material(material).unwrap_or_else(|| panic!("no material {material}"));
    blocks.find_shaped(m, shape).unwrap_or_else(|| panic!("the test area has no {material} {shape:?}"))
}

pub fn stone_slab(sim: &Simulation) -> BlockId {
    shaped(sim, base::STONE, &base::shapes::slab())
}

pub fn brick_slab(sim: &Simulation) -> BlockId {
    shaped(sim, base::BRICK, &base::shapes::slab())
}

pub fn stone_stairs(sim: &Simulation) -> BlockId {
    shaped(sim, base::STONE, &base::shapes::stairs())
}

pub fn wood_post(sim: &Simulation) -> BlockId {
    shaped(sim, base::WOOD, &base::shapes::post())
}

/// The base mod's data half with every block placeable, so tests can place the blocks the
/// palette hides (stone, logs) to check how placement works.
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
        if let Some(crafting) = self.0.crafting() {
            r.register_crafting(crafting.clone())?;
        }
        Ok(())
    }
}
