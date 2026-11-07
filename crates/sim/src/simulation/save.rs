//! Saving a running simulation as plain data, and loading one back into it (see
//! [`crate::save`] for the data and its bytes).

use std::collections::HashSet;

use api::{ApiError, ChunkPos, I64Vec3, Tunables};

use super::Simulation;
use crate::save::{Made, SaveData, SavedChunk, SavedItem, SavedPlayer};
use crate::{ChunkData, Drop, Inventory, Occupant};

/// What a load couldn't bring back. Nothing here stops a load: a block from a mod that's gone
/// is left out of its boxel, a material that's gone out of the inventory.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadReport {
    /// One line per thing left out, without repeats.
    pub skipped: Vec<String>,
}

impl LoadReport {
    fn skip(&mut self, what: String) {
        if !self.skipped.contains(&what) {
            self.skipped.push(what);
        }
    }
}

impl Simulation {
    /// The world as it stands, to save: the terrain's values, the blocks made while the game
    /// ran, the chunks loading can't make again, the player, what they carry and the loose
    /// items. Creatures aren't saved.
    pub fn save(&self) -> SaveData {
        let blocks = &self.blocks;
        let name = |id| blocks.get(id).map_or(String::new(), |def| def.name.clone());
        let mut terrain: Vec<(String, String, f32)> =
            self.world.tunables().into_iter().map(|t| ("world".to_owned(), t.name.to_string(), t.value)).collect();
        for (generator, g) in &self.generators {
            terrain.extend(g.tunables().into_iter().map(|t| (format!("terrain {generator}"), t.name.to_string(), t.value)));
        }

        let mut made = Vec::new();
        for (id, _) in blocks.iter() {
            if let Some(d) = blocks.derived(id) {
                made.push(Made::Derived {
                    from: name(d.from.block),
                    orientation: d.from.orientation,
                    shape: d.shape.clone(),
                    hidden: d.hidden,
                });
            } else if let Some(bundle) = blocks.bundle_of(id) {
                let pieces = blocks.pieces(bundle);
                // A bundle is made whole, so it's listed once, at its first piece.
                if pieces.first() == Some(&id) {
                    let pieces = pieces
                        .iter()
                        .filter_map(|&b| Some((blocks.material_name(blocks.made_of(b)?).to_owned(), blocks.shape(b))))
                        .collect();
                    made.push(Made::Bundle { pieces });
                }
            } else if let Some(Drop { material, .. }) = blocks.drop(id)
                && let Some(of) = blocks.part_of(material)
                && blocks.material_block(material) == id
            {
                made.push(Made::Shaped { material: blocks.material_name(of).to_owned(), shape: blocks.shape(id) });
            }
        }

        let grid = self.universe.main();
        let edited = self.changed.edited();
        let mut positions: Vec<ChunkPos> = grid
            .chunks()
            .map(|(pos, _)| pos)
            .filter(|pos| !self.generated.contains(pos) || edited.contains(pos))
            .collect();
        positions.sort_by_key(|p| p.0.to_array());
        let chunks = positions
            .into_iter()
            .map(|pos| {
                let data = grid.export_chunk(pos);
                let lists = data
                    .lists
                    .iter()
                    .map(|list| list.iter().map(|o| (name(o.block), o.orientation)).collect())
                    .collect();
                SavedChunk { pos, lists, cells: data.cells }
            })
            .collect();

        let player = self.player.as_ref().map(|p| SavedPlayer {
            pos: p.pos(),
            yaw: self.look.0,
            pitch: self.look.1,
            flying: p.flying(),
            health: self.health,
        });
        let inventory = self.inventory.iter().map(|(m, amount)| (blocks.material_name(m).to_owned(), amount)).collect();
        let items = self
            .items
            .iter()
            .map(|item| SavedItem {
                material: blocks.material_name(item.drop.material).to_owned(),
                amount: item.drop.amount,
                block: name(item.block),
                pos: item.pos,
                velocity: item.velocity,
            })
            .collect();
        SaveData { tick: self.tick, terrain, made, chunks, player, inventory, items }
    }

    /// Puts the world back as `save` has it: generates the terrain with its values, makes its
    /// blocks again, replaces every chunk the terrain doesn't cover and the edited ones it does,
    /// and puts back the player, what they carry and the loose items. Creatures start fresh at
    /// their spots. Anything a missing mod named is left out and listed in the report; only
    /// failing to generate the terrain fails the load.
    pub fn load(&mut self, save: &SaveData) -> Result<LoadReport, ApiError> {
        let mut report = LoadReport::default();
        for (section, name, value) in &save.terrain {
            let set = match section.strip_prefix("terrain ") {
                None => section == "world" && self.world.set_tunable(name, *value),
                Some(generator) => {
                    let found = self.generators.iter_mut().find(|(n, _)| n == generator);
                    found.is_some_and(|(_, g)| g.set_tunable(name, *value))
                }
            };
            if !set {
                report.skip(format!("terrain value [{section}] {name}"));
            }
        }
        for made in &save.made {
            if let Err(why) = self.remake(made) {
                report.skip(format!("a crafted block: {why}"));
            }
        }

        self.generate_terrain()?;
        // Nothing counts as edited but what the save brings back.
        self.changed.forget_all_edits();
        let grid = self.universe.main_mut();
        let built: Vec<ChunkPos> = grid.chunks().map(|(pos, _)| pos).filter(|p| !self.generated.contains(p)).collect();
        for pos in built {
            grid.remove_chunk(pos);
            self.changed.mark_chunk(pos);
        }
        for chunk in &save.chunks {
            let lists = chunk
                .lists
                .iter()
                .map(|list| {
                    list.iter()
                        .filter_map(|(name, orientation)| match self.blocks.id(name) {
                            Some(id) => Some(Occupant::turned(id, *orientation)),
                            None => {
                                report.skip(format!("block {name}"));
                                None
                            }
                        })
                        .collect()
                })
                .collect();
            grid.import_chunk(&ChunkData { pos: chunk.pos, lists, cells: chunk.cells.clone() });
            // Its neighbors' border faces may have changed too.
            for side in [I64Vec3::ZERO, I64Vec3::X, I64Vec3::NEG_X, I64Vec3::Y, I64Vec3::NEG_Y, I64Vec3::Z, I64Vec3::NEG_Z] {
                self.changed.mark_chunk(ChunkPos(chunk.pos.0 + side));
            }
            if self.generated.contains(&chunk.pos) {
                self.changed.mark_edited(chunk.pos);
            }
        }

        if let (Some(player), Some(saved)) = (self.player.as_mut(), &save.player) {
            player.teleport(saved.pos);
            player.set_flying(saved.flying);
            self.look = (saved.yaw, saved.pitch);
            let full = self.health_def.as_ref().map_or(0.0, |d| d.health);
            self.health = saved.health.clamp(0.0, full);
            self.since_hurt = 0.0;
        }
        self.swing = None;
        self.guarding = false;
        self.inventory = Inventory::default();
        for (name, amount) in &save.inventory {
            match self.blocks.material(name) {
                Some(m) => {
                    self.inventory.add(m, *amount, None);
                }
                None => report.skip(format!("carried {name}")),
            }
        }
        self.items.clear();
        for item in &save.items {
            match (self.blocks.material(&item.material), self.blocks.id(&item.block)) {
                (Some(material), Some(block)) => {
                    let drop = Drop { material, amount: item.amount };
                    self.items.spawn(drop, block, &self.blocks, item.pos, item.velocity);
                }
                _ => report.skip(format!("a loose item of {}", item.material)),
            }
        }
        self.respawn_targets();
        self.lift_out_of_ground();
        Ok(report)
    }

    /// Makes a block again from how a save says it was made.
    fn remake(&mut self, made: &Made) -> Result<(), String> {
        let blocks = &mut self.blocks;
        let material = |blocks: &crate::BlockRegistry, name: &str| blocks.material(name).ok_or(format!("no material {name}"));
        match made {
            Made::Shaped { material: name, shape } => {
                let m = material(blocks, name)?;
                blocks.shaped(m, shape).map_err(|e| e.to_string())?;
            }
            Made::Bundle { pieces } => {
                let pieces = pieces
                    .iter()
                    .map(|(name, shape)| Ok((material(blocks, name)?, shape.clone())))
                    .collect::<Result<Vec<_>, String>>()?;
                blocks.bundle(&pieces).map_err(|e| e.to_string())?;
            }
            Made::Derived { from, orientation, shape, hidden } => {
                let block = blocks.id(from).ok_or(format!("no block {from}"))?;
                blocks.derive(Occupant::turned(block, *orientation), shape, *hidden).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// Chunks edited since they were last made from scratch, for tests.
    pub fn edited_chunks(&self) -> &HashSet<ChunkPos> {
        self.changed.edited()
    }
}
