//! Saving a running simulation as plain data, and loading one back into it (see
//! [`crate::save`] for the data and its bytes).

use std::collections::HashSet;

use api::{ApiError, ChunkPos, I64Vec3, Tunables};

use super::Simulation;
use crate::save::{Made, SaveData, SavedBlueprint, SavedChange, SavedChunk, SavedDesign, SavedItem, SavedPlayer, SavedRecord, SavedThing};
use crate::{Blueprint, BlockRegistry, Cells, Change, ChunkData, Design, Drop, Inventory, Layer, Library, Light, Occupant, Piece, Recipe, Thing};

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
    /// Saves the world as it stands: hands the edited terrain loaded now to the chunk store
    /// (see [`crate::store`]) and makes everything stored the saved chunks, then returns the
    /// rest to write (see [`Simulation::snapshot`]). Fails if the store couldn't keep the
    /// chunks.
    pub fn save(&self) -> Result<SaveData, String> {
        self.store_loaded_edits();
        self.stream.store().commit()?;
        Ok(self.snapshot())
    }

    /// Saves the world as it stands into `store`, a new place, and keeps chunks there from now
    /// on; the old store is left as last saved. Returns the rest to write, as
    /// [`Simulation::save`] does.
    pub fn save_as(&mut self, store: std::sync::Arc<dyn crate::ChunkStore>) -> Result<SaveData, String> {
        self.store_loaded_edits();
        let old = std::sync::Arc::clone(self.stream.store());
        crate::store::copy(old.as_ref(), store.as_ref())?;
        old.discard()?;
        self.stream.set_store(store);
        Ok(self.snapshot())
    }

    /// Keeps chunks in `store` from now on, as when another world is opened. Load a save next:
    /// what's loaded now came from the old one.
    pub fn set_chunk_store(&mut self, store: std::sync::Arc<dyn crate::ChunkStore>) {
        self.stream.set_store(store);
    }

    /// Where edited terrain goes while unloaded.
    pub fn chunk_store(&self) -> &std::sync::Arc<dyn crate::ChunkStore> {
        self.stream.store()
    }

    /// Puts the edited terrain loaded now, and the tops edits moved, in the store's unsaved
    /// layer.
    fn store_loaded_edits(&self) {
        self.store_tops();
        let grid = self.universe.main();
        let edited = self.changed.edited();
        let mut positions: Vec<ChunkPos> = edited.iter().copied().filter(|&pos| self.is_terrain(pos) && grid.chunk(pos).is_some()).collect();
        positions.sort_by_key(|p| p.0.to_array());
        let chunks: Vec<(ChunkPos, Vec<u8>)> =
            positions.into_iter().map(|pos| (pos, name_chunk(&self.blocks, &grid.export_chunk(pos)).to_bytes())).collect();
        if !chunks.is_empty() {
            self.stream.store().put(chunks);
        }
    }

    /// The world as it stands but for its edited terrain, which lives in the chunk store: the
    /// terrain's values, the blocks made while the game ran, the chunks outside the terrain,
    /// the player, what they carry and the loose items. Creatures aren't saved. Touches no
    /// store, so it can be taken any time (the server keeps one from before any terrain, to
    /// start fresh worlds from).
    pub fn snapshot(&self) -> SaveData {
        let blocks = &self.blocks;
        let name = |id| blocks.get(id).map_or(String::new(), |def| def.name.clone());
        let mut terrain: Vec<(String, String, f32)> =
            self.world.tunables().into_iter().map(|t| ("world".to_owned(), t.name.to_string(), t.value)).collect();
        for (i, (generator, _)) in self.generators.iter().enumerate() {
            let values = self.generator_tunables(i).into_iter();
            terrain.extend(values.map(|t| (format!("terrain {generator}"), t.name.to_string(), t.value)));
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
        // Built chunks: the edited terrain is in the store.
        let mut chunks: Vec<SavedChunk> = grid
            .chunks()
            .map(|(pos, _)| pos)
            .filter(|&pos| !self.is_terrain(pos))
            .map(|pos| name_chunk(blocks, &grid.export_chunk(pos)))
            .collect();
        chunks.sort_by_key(|c| c.pos.0.to_array());

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
        let blueprints = save_blueprints(blocks, &self.blueprints);
        SaveData { tick: self.tick, terrain, made, chunks, player, inventory, items, blueprints, hours: Some(self.clock.hours()) }
    }

    /// Puts the world back as `save` has it: throws away the chunk store's unsaved chunks (so
    /// edited terrain comes back as last saved), generates the terrain with its values, makes
    /// its blocks again, replaces every chunk the terrain doesn't cover, and puts back the
    /// player, what they carry and the loose items. Creatures start fresh at their spots. A save
    /// from before version 4 holds its edited terrain itself, which goes to the store. Anything
    /// a missing mod named is left out and listed in the report; only failing to generate the
    /// terrain or to reach the store fails the load.
    pub fn load(&mut self, save: &SaveData) -> Result<LoadReport, ApiError> {
        let mut report = LoadReport::default();
        self.stream.store().discard().map_err(ApiError::Storage)?;
        self.clock.set_hours(save.hours.unwrap_or(crate::Clock::START_HOURS));
        for (section, name, value) in &save.terrain {
            let set = match section.strip_prefix("terrain ") {
                None => section == "world" && self.world.set_tunable(name, *value),
                Some(generator) => {
                    let found = self.generators.iter().position(|(n, _)| n == generator);
                    found.is_some_and(|i| self.tune_generator(i, name, *value))
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

        // The terrain is made again from the save's values as it streams in; everything else
        // comes from the save. Nothing counts as edited but what the save brings back.
        self.drop_terrain();
        self.changed.forget_all_edits();
        let grid = self.universe.main_mut();
        let built: Vec<ChunkPos> = grid.chunks().map(|(pos, _)| pos).collect();
        for pos in built {
            grid.remove_chunk(pos);
            self.changed.mark_chunk(pos);
        }
        self.light_settled = false;
        let mut older = Vec::new();
        for chunk in &save.chunks {
            if self.is_terrain_place(chunk.pos) {
                // From an older save: edited terrain goes to the store, and comes back from it
                // when its column streams in.
                older.push((chunk.pos, chunk.to_bytes()));
                continue;
            }
            let data = unname_chunk(&self.blocks, chunk, &mut report);
            let grid = self.universe.main_mut();
            grid.import_chunk(&data);
            // Its neighbors' border faces may have changed too.
            for side in [I64Vec3::ZERO, I64Vec3::X, I64Vec3::NEG_X, I64Vec3::Y, I64Vec3::NEG_Y, I64Vec3::Z, I64Vec3::NEG_Z] {
                self.changed.mark_chunk(ChunkPos(chunk.pos.0 + side));
            }
        }

        if !older.is_empty() {
            self.stream.store().put(older);
        }
        // A save from before records has none; one with them keeps what it remembered.
        self.remember_all();

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
        self.refill_endless();
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
        let library = load_blueprints(&self.blocks, &save.blueprints, &mut report);
        self.set_blueprints(library);
        self.respawn_targets();
        self.settle_light();
        self.stream_terrain(Some(Self::NEAR));
        match &self.stream.error {
            Some(e) => Err(e.clone()),
            None => Ok(report),
        }
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

/// `data` with its blocks named.
pub(crate) fn name_chunk(blocks: &BlockRegistry, data: &ChunkData) -> SavedChunk {
    let name = |id| blocks.get(id).map_or(String::new(), |def| def.name.clone());
    let lists = data.lists.iter().map(|list| list.iter().map(|o| (name(o.block), o.orientation)).collect()).collect();
    let records = data
        .records
        .iter()
        .map(|(index, r)| SavedRecord {
            index: *index,
            stored: r.stored.iter().map(|&(m, a)| (blocks.material_name(m).to_owned(), a)).collect(),
            memory: r.memory.iter().map(|(o, s)| (name(o.block), o.orientation, s.clone())).collect(),
        })
        .collect();
    SavedChunk { pos: data.pos, lists, cells: data.cells.clone(), records }
}

/// `chunk` with its names looked up, unlit. A name no mod registers is left out of its
/// boxels, and listed in `report`.
pub(crate) fn unname_chunk(blocks: &BlockRegistry, chunk: &SavedChunk, report: &mut LoadReport) -> ChunkData {
    let lists = chunk
        .lists
        .iter()
        .map(|list| {
            list.iter()
                .filter_map(|(name, orientation)| match blocks.id(name) {
                    Some(id) => Some(Occupant::turned(id, *orientation)),
                    None => {
                        report.skip(format!("block {name}"));
                        None
                    }
                })
                .collect()
        })
        .collect();
    let records = chunk
        .records
        .iter()
        .map(|r| {
            let mut record = crate::Record::default();
            for (name, amount) in &r.stored {
                match blocks.material(name) {
                    Some(m) => record.add(m, *amount),
                    None => report.skip(format!("stored {name}")),
                }
            }
            for (name, orientation, shape) in &r.memory {
                match blocks.id(name) {
                    Some(id) => record.memory.push((Occupant::turned(id, *orientation), shape.clone())),
                    None => report.skip(format!("remembered block {name}")),
                }
            }
            (r.index, record)
        })
        .filter(|(_, r)| !r.is_empty())
        .collect();
    ChunkData { pos: chunk.pos, lists, cells: chunk.cells.clone(), light: Layer::Uniform(Light::SKY), records }
}

/// `library` by name.
fn save_blueprints(blocks: &BlockRegistry, library: &Library) -> Vec<SavedBlueprint> {
    let name = |id| library.get(id).map_or(String::new(), |b: &Blueprint| b.name.clone());
    library
        .iter()
        .map(|(_, b)| {
            let design = match &b.design {
                Design::Recipe(recipe) => SavedDesign::Recipe {
                    layers: recipe
                        .cells
                        .iter()
                        .flatten()
                        .flatten()
                        .map(|p| {
                            p.map(|p| {
                                let thing = match p.thing {
                                    Thing::Material(m) => SavedThing::Material(blocks.material_name(m).to_owned()),
                                    Thing::Blueprint(id) => SavedThing::Blueprint(name(id)),
                                };
                                (thing, p.turns)
                            })
                        })
                        .collect(),
                    side: recipe.side,
                    flip: recipe.flip,
                },
                Design::Variant { base, changes } => SavedDesign::Variant {
                    base: name(*base),
                    changes: changes
                        .iter()
                        .map(|c| match *c {
                            Change::Turn { axis, quarters } => SavedChange::Turn { axis, quarters },
                            Change::Mirror { axis } => SavedChange::Mirror { axis },
                            Change::Shift(by) => SavedChange::Shift(by),
                            Change::Swap { from, to } => SavedChange::Swap {
                                from: blocks.material_name(from).to_owned(),
                                to: blocks.material_name(to).to_owned(),
                            },
                        })
                        .collect(),
                },
            };
            SavedBlueprint { name: b.name.clone(), design }
        })
        .collect()
}

/// A library from `saved`. A blueprint naming a material or blueprint that's gone is left out,
/// and so is every blueprint using it.
fn load_blueprints(blocks: &BlockRegistry, saved: &[SavedBlueprint], report: &mut LoadReport) -> Library {
    let mut kept: Vec<&SavedBlueprint> = saved.iter().collect();
    loop {
        let names: Vec<&str> = kept.iter().map(|b| b.name.as_str()).collect();
        let mut library = Library::default();
        for b in &kept {
            // Designs come once every name has an id, since a layer may use a later one.
            library.insert(Blueprint { name: b.name.clone(), design: Design::Variant { base: crate::BlueprintId(u32::MAX), changes: Vec::new() } });
        }
        let mut lost = None;
        for b in &kept {
            match design_of(blocks, &library, &names, &b.design) {
                Ok(design) => {
                    let id = library.find(&b.name).expect("just inserted");
                    library.set_design(id, design);
                }
                Err(why) => {
                    lost = Some((b.name.clone(), why));
                    break;
                }
            }
        }
        match lost {
            None => return library,
            Some((name, why)) => {
                report.skip(format!("blueprint {name}: {why}"));
                kept.retain(|b| b.name != name);
            }
        }
    }
}

fn design_of(blocks: &BlockRegistry, library: &Library, names: &[&str], saved: &SavedDesign) -> Result<Design, String> {
    let material = |name: &str| blocks.material(name).ok_or(format!("no material {name}"));
    let blueprint = |name: &str| match names.contains(&name) {
        true => Ok(library.find(name).expect("every kept name is in")),
        false => Err(format!("no blueprint {name}")),
    };
    Ok(match saved {
        SavedDesign::Recipe { layers, side, flip } => {
            let mut cells: Cells = Default::default();
            for (i, layer) in layers.iter().enumerate().take(64) {
                let Some((thing, turns)) = layer else {
                    continue;
                };
                let thing = match thing {
                    SavedThing::Material(name) => Thing::Material(material(name)?),
                    SavedThing::Blueprint(name) => Thing::Blueprint(blueprint(name)?),
                };
                cells[i / 16][i / 4 % 4][i % 4] = Some(Piece { thing, turns: *turns % 4 });
            }
            Design::Recipe(Recipe { cells, side: *side, flip: *flip })
        }
        SavedDesign::Variant { base, changes } => Design::Variant {
            base: blueprint(base)?,
            changes: changes
                .iter()
                .map(|c| {
                    Ok(match c {
                        SavedChange::Turn { axis, quarters } => Change::Turn { axis: *axis, quarters: *quarters },
                        SavedChange::Mirror { axis } => Change::Mirror { axis: *axis },
                        SavedChange::Shift(by) => Change::Shift(*by),
                        SavedChange::Swap { from, to } => Change::Swap { from: material(from)?, to: material(to)? },
                    })
                })
                .collect::<Result<_, String>>()?,
        },
    })
}
