//! The simulation: loads mods, owns the world and advances it at a fixed rate.

use std::collections::HashMap;
use std::fmt;

use api::block::is_valid_name;
use api::{
    ApiError, BlockDef, BoxelPos, Brain, ChunkPos, CraftingDef, CrowdDef, I64Vec3, InteractionDef, InventoryDef, ItemDropDef, LookDef,
    ModelDef, Mod, MovementController, Orientation, PlayerDef, PushOutDef, Registrar, ShapeGrid, SoundDef,
    TargetDef, ToolDef, Turning, Vec3, WorldEdit, WorldPos, look_direction,
};

use crate::combat::{StepContext, Strike, Swing, SwingStage, TargetKind, Targets, ToolId};
use crate::crafting::{Craft, CraftPlan, CraftRefusal};
use crate::edit::{ChangedChunks, Repeater};
use crate::inventory::{Inventory, capacity_units};
use crate::items::{Collector, Items, pop_velocity};
use crate::player::{PushOut, Terrain};
use crate::{
    BlockId, BlockRegistry, CellBox, Grid, Hit, MaterialId, Occupant, Player, PlayerInput, LookRegistry, Universe, can_share,
    occupant_boxes, raycast,
};

/// How far in front of an aimed-at face the point that picks the boxel for a placed block is.
const IN_FRONT: f32 = 1e-3;
/// How far below the eye a thrown item starts, in meters, so it leaves from about the chest.
const THROW_DROP: f32 = 0.3;

#[derive(Debug, PartialEq)]
pub enum SimError {
    Api(ApiError),
    /// No mod registered a player definition.
    NoPlayerDef,
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SimError::Api(e) => write!(f, "{e}"),
            SimError::NoPlayerDef => write!(f, "no mod registered a player definition"),
        }
    }
}

impl std::error::Error for SimError {}

impl From<ApiError> for SimError {
    fn from(e: ApiError) -> Self {
        SimError::Api(e)
    }
}

/// What placing a block would do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaceTarget {
    /// The boxel it goes into.
    pub pos: BoxelPos,
    /// The block, turned by the placement rule.
    pub occupant: Occupant,
    /// Whether it may go there. A refused placement still says where it would have gone, so
    /// a preview can show it as not allowed.
    pub allowed: bool,
}

/// The player's last placement attempt, for tools that explain it. Read only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaceReport {
    /// The tick it happened on, as [`Simulation::tick`] read before that tick's step.
    pub tick: u64,
    /// Where the block went or would have gone; `None` with nothing in reach to place on.
    pub target: Option<PlaceTarget>,
    /// How the block pushed the player out, and at what speed, if it went inside the body.
    pub push_out: Option<(PushOut, f32)>,
}

#[derive(Default)]
pub struct Simulation {
    blocks: BlockRegistry,
    looks: LookRegistry,
    /// Movement controllers by name, as registered. Each body gets its own from `spawn`.
    controllers: HashMap<String, Box<dyn MovementController>>,
    /// Brains by name, in registration order.
    brains: Vec<(String, Box<dyn Brain>)>,
    /// Each target kind's brain, as an index into `brains`.
    target_brains: Vec<Option<usize>>,
    /// Targets stand still as dummies, whatever their brains would do.
    brains_off: bool,
    player_def: Option<PlayerDef>,
    interaction_def: Option<InteractionDef>,
    push_out_def: Option<PushOutDef>,
    crowd_def: Option<CrowdDef>,
    item_drop_def: Option<ItemDropDef>,
    models: Vec<ModelDef>,
    sounds: Vec<SoundDef>,
    target_defs: Vec<TargetDef>,
    tools: Vec<ToolDef>,
    universe: Universe,
    spawn: WorldPos,
    waypoints: Vec<(String, WorldPos)>,
    player: Option<Player>,
    breaking: Repeater,
    placing: Repeater,
    dropping: Repeater,
    changed: ChangedChunks,
    items: Items,
    inventory_def: Option<InventoryDef>,
    /// What the player carries.
    inventory: Inventory,
    crafting_def: Option<CraftingDef>,
    /// The last craft: the part made and how many, or why nothing was.
    last_craft: Option<Result<(MaterialId, u32), CraftRefusal>>,
    targets: Targets,
    swing: Option<Swing>,
    guarding: bool,
    /// Strikes since the last [`Simulation::take_strikes`].
    strikes: Vec<Strike>,
    tick: u64,
    last_place: Option<PlaceReport>,
}

impl Simulation {
    /// Ticks per second.
    pub const TICK_RATE: f64 = 60.0;
    /// Seconds per tick.
    pub const TICK_SECONDS: f32 = 1.0 / Self::TICK_RATE as f32;

    /// An empty simulation with no content: no blocks, no player definition.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers every mod's content, then lets every mod build the world.
    pub fn with_mods(mods: &[&dyn Mod]) -> Result<Self, SimError> {
        let mut sim = Self::new();
        for m in mods {
            m.register(&mut ContentRegistrar {
                looks: &mut sim.looks,
                blocks: &mut sim.blocks,
                controllers: &mut sim.controllers,
                brains: &mut sim.brains,
                player: &mut sim.player_def,
                interaction: &mut sim.interaction_def,
                push_out: &mut sim.push_out_def,
                crowd: &mut sim.crowd_def,
                item_drops: &mut sim.item_drop_def,
                inventory: &mut sim.inventory_def,
                crafting: &mut sim.crafting_def,
                models: &mut sim.models,
                sounds: &mut sim.sounds,
                targets: &mut sim.target_defs,
                tools: &mut sim.tools,
            })?;
        }
        sim.looks.check_blocks(&sim.blocks)?;
        if let Some(def) = &sim.crafting_def
            && sim.blocks.id(&def.table).is_none()
        {
            return Err(ApiError::UnknownBlock(def.table.clone()).into());
        }
        if let Some(def) = &sim.inventory_def {
            for (name, boxels) in &def.start {
                let material = sim.blocks.material(name).ok_or_else(|| ApiError::UnknownMaterial(name.clone()))?;
                sim.inventory.add(material, capacity_units(*boxels), None);
            }
        }
        for def in &sim.target_defs {
            if !sim.models.iter().any(|m| m.name == def.model) {
                return Err(ApiError::UnknownModel(def.model.clone()).into());
            }
            if let Some(sound) = def.hit_sound.as_ref().filter(|&name| !sim.sounds.iter().any(|s| s.name == *name)) {
                return Err(ApiError::UnknownSound(sound.clone()).into());
            }
            let brain = match &def.brain {
                None => None,
                Some(name) => Some(
                    sim.brains.iter().position(|(n, _)| n == name).ok_or_else(|| ApiError::UnknownBrain(name.clone()))?,
                ),
            };
            sim.target_brains.push(brain);
        }
        for m in mods {
            m.build_world(&mut WorldEditor {
                blocks: &mut sim.blocks,
                grid: sim.universe.main_mut(),
                spawn: &mut sim.spawn,
                waypoints: &mut sim.waypoints,
                target_defs: &sim.target_defs,
                targets: &mut sim.targets,
            })?;
        }
        Ok(sim)
    }

    /// Adds a movement controller the engine loaded itself, such as one from a WebAssembly
    /// component, as if a mod had registered it.
    pub fn add_movement_controller(
        &mut self,
        name: &str,
        controller: Box<dyn MovementController>,
    ) -> Result<(), ApiError> {
        add_controller(&mut self.controllers, name, controller)
    }

    /// Puts the player at the spawn point. Needs a mod to have registered a player definition,
    /// and the movement controller it names.
    pub fn spawn_player(&mut self) -> Result<(), SimError> {
        let def = self.player_def.as_ref().ok_or(SimError::NoPlayerDef)?;
        let controller = self
            .controllers
            .get(&def.controller)
            .ok_or_else(|| ApiError::UnknownController(def.controller.clone()))?;
        self.player = Some(Player::new(def, controller.spawn(), self.spawn));
        Ok(())
    }

    /// Advances one fixed tick. Block edits and tool swings happen first, from where the
    /// player stood at the end of the last tick, which is what the client aimed from. Targets
    /// move after the player, and loose items last, toward where the player ended up.
    pub fn step(&mut self, input: &PlayerInput) {
        if let Some(craft) = &input.craft {
            self.last_craft = Some(self.craft(craft));
        }
        if let Some(m) = input.discard {
            self.inventory.take(m, self.inventory.amount(m));
        }
        self.edit_blocks(input);
        self.use_tool(input);
        self.throw(input);
        let capacity = self.capacity();
        let Self { blocks, universe, player, items, item_drop_def, inventory, targets, target_defs, crowd_def, .. } = self;
        let terrain = GridTerrain { grid: universe.main(), blocks };
        if let Some(player) = player {
            player.step(input, Self::TICK_SECONDS, &terrain);
        }
        let brains: Vec<Option<&dyn Brain>> = if self.brains_off {
            Vec::new()
        } else {
            self.target_brains.iter().map(|b| b.map(|i| &*self.brains[i].1)).collect()
        };
        let ctx = StepContext {
            defs: target_defs,
            brains: &brains,
            crowd: crowd_def.as_ref(),
            player: player.as_mut(),
            tick: self.tick,
            dt: Self::TICK_SECONDS,
        };
        targets.step(ctx, &terrain);
        if let Some(def) = item_drop_def {
            let room = inventory.room(capacity);
            let collector =
                player.as_ref().map(|p| Collector { feet: p.pos(), width: p.shape().width, height: p.height(), room });
            for drop in items.step(def, blocks, Self::TICK_SECONDS, &terrain, collector) {
                inventory.add(drop.material, drop.amount, capacity);
            }
        }
        self.tick += 1;
    }

    /// The occupant the player would break looking along `yaw` and `pitch`, and the face a
    /// placed block would go on. `None` with nothing in reach, no player, or no mod defining
    /// interaction.
    pub fn target(&self, yaw: f32, pitch: f32) -> Option<Hit> {
        let player = self.player.as_ref()?;
        let reach = self.interaction_def.as_ref()?.reach;
        let grid = self.universe.main();
        let blocks = &self.blocks;
        raycast(player.eye(), look_direction(yaw, pitch), reach, |p, out| {
            for (i, &o) in grid.cell(p).iter().enumerate() {
                out.extend(occupant_boxes(o, blocks).map(|b| (b, i)));
            }
        })
    }

    /// What placing `block` aimed at `hit`, looking along `yaw` and `pitch`, would do, or `None`
    /// if there's nothing to place: the block isn't placeable or the hit has no face.
    ///
    /// The block goes into the boxel holding the point just in front of the aimed-at face. If it
    /// can't share that boxel, it goes into the boxel on the far side of the face instead, like
    /// a full block placed on a slab. A solid block inside the player is only allowed if a
    /// push-out rule is registered and there's room to push the player out.
    ///
    /// Every block is turned by the same rules, kept as far as its [`Turning`] allows: basic
    /// placement, or with `precise`, precise placement (see [`orient`]). A face inside a boxel,
    /// like the top of a bottom slab, is aimed at from that boxel, so a block placed into the
    /// boxel itself goes against the boxel's far side, facing back toward the face: a slab on a
    /// slab fills the top half.
    pub fn placement(&self, hit: &Hit, block: BlockId, yaw: f32, pitch: f32, precise: bool) -> Option<PlaceTarget> {
        let def = self.blocks.get(block).filter(|def| def.placeable)?;
        if hit.normal == I64Vec3::ZERO {
            return None;
        }
        let front = hit.point + hit.normal.as_vec3() * IN_FRONT;
        let inside = hit.pos.offset(front.floor().as_i64vec3());
        let grid = self.universe.main();
        let look = look_direction(yaw, pitch);
        let region = face_region(hit);
        let mut first = None;
        for pos in [inside, hit.adjacent()] {
            let face = if pos == hit.pos { -hit.normal } else { hit.normal };
            let aim = Aim {
                face,
                look,
                // The aimed-at point in this boxel's space.
                upper: (front - (pos.0 - hit.pos.0).as_vec3()).z > 0.5,
                region: precise.then_some(region),
            };
            let occupant = Occupant::turned(block, orient(def.turning, &aim));
            if can_share(grid.cell(pos), occupant, &self.blocks).is_ok() {
                let allowed = self.can_pay(block) && self.body_allows(pos, occupant);
                return Some(PlaceTarget { pos, occupant, allowed });
            }
            first.get_or_insert(PlaceTarget { pos, occupant, allowed: false });
        }
        first
    }

    /// Whether the player carries enough to place `block`: its volume of its material. Always,
    /// without an inventory definition or for a block made of no material.
    fn can_pay(&self, block: BlockId) -> bool {
        match (&self.inventory_def, self.blocks.drop(block)) {
            (Some(_), Some(cost)) => self.inventory.amount(cost.material) >= cost.amount,
            _ => true,
        }
    }

    /// Whether the bodies in the world leave room for `occupant` at `pos`: it isn't solid, or
    /// it overlaps no target and either doesn't overlap the player or the player can be pushed
    /// out.
    fn body_allows(&self, pos: BoxelPos, occupant: Occupant) -> bool {
        if !self.blocks.is_solid(occupant.block) {
            return true;
        }
        let boxes: Vec<_> = occupant_boxes(occupant, &self.blocks).collect();
        if self.targets.iter().any(|t| t.overlaps_boxes(&self.target_defs[t.kind.index()], pos, &boxes)) {
            return false;
        }
        let Some(player) = &self.player else {
            return true;
        };
        !player.overlaps_boxes(pos, &boxes) || self.push_out(player, pos, &boxes).is_some()
    }

    /// How the player would get out of a solid block filling `boxes` at `pos`, if there's a
    /// rule and room.
    fn push_out(&self, player: &Player, pos: BoxelPos, boxes: &[CellBox]) -> Option<PushOut> {
        let def = self.push_out_def.as_ref()?;
        let inner = GridTerrain { grid: self.universe.main(), blocks: &self.blocks };
        let terrain = WithBoxes { inner, pos, boxes };
        player.push_out(pos, boxes, def.upward_bias, &terrain)
    }

    /// Chunks whose blocks changed since the last call, including neighbors whose border faces
    /// changed. World building at load time isn't included.
    pub fn take_changed_chunks(&mut self) -> Vec<ChunkPos> {
        self.changed.take()
    }

    /// Breaks or places a block if the input asks for it this tick. Breaking wins when both
    /// buttons act on the same tick.
    fn edit_blocks(&mut self, input: &PlayerInput) {
        let Some(def) = &self.interaction_def else {
            return;
        };
        if input.tool.is_some() {
            return;
        }
        let interval = 1.0 / def.repeat_rate;
        let dt = Self::TICK_SECONDS;
        let breaking = self.breaking.update(input.use_pressed, input.use_held, interval, dt);
        let placing = self.placing.update(input.alt_pressed, input.alt_held, interval, dt);
        if !breaking && !placing {
            return;
        }
        let hit = self.target(input.yaw, input.pitch);
        if breaking {
            if let Some(hit) = hit {
                self.remove_occupant(hit.pos, hit.occupant);
            }
            return;
        }
        let target = hit.and_then(|hit| self.placement(&hit, input.place_block, input.yaw, input.pitch, input.place_precise));
        let push_out = match target {
            Some(place) if place.allowed => {
                if let (Some(_), Some(cost)) = (&self.inventory_def, self.blocks.drop(place.occupant.block)) {
                    self.inventory.take(cost.material, cost.amount);
                }
                self.add_occupant(place.pos, place.occupant)
            }
            _ => None,
        };
        self.last_place = Some(PlaceReport { tick: self.tick, target, push_out });
    }

    /// Throws a full block's worth of the material the input names, or what's left of it if less,
    /// on the drop button's press and then repeating while it's held, at the interaction
    /// definition's rate. The item leaves from just below the eye along the look, carrying the
    /// player's own speed, and can't be picked up until the inventory's pickup delay has passed.
    /// Needs an inventory, item drops and an interaction definition.
    fn throw(&mut self, input: &PlayerInput) {
        let (Some(inventory), Some(interaction), Some(_)) = (&self.inventory_def, &self.interaction_def, &self.item_drop_def)
        else {
            return;
        };
        let interval = 1.0 / interaction.repeat_rate;
        if !self.dropping.update(input.drop_pressed, input.drop_held, interval, Self::TICK_SECONDS) {
            return;
        }
        let (Some(player), Some(material)) = (&self.player, input.drop_material) else {
            return;
        };
        if material.index() >= self.blocks.material_count() {
            return;
        }
        // A part goes whole; a material a full block's worth, or what's left.
        let unit = self.blocks.unit(material);
        let have = self.inventory.amount(material);
        let amount = if unit > 1 { unit } else { have.min(crate::FULL_VOLUME) };
        if amount == 0 || have < amount {
            return;
        }
        let look = look_direction(input.yaw, input.pitch);
        let at = player.eye().translate(Vec3::NEG_Z * THROW_DROP);
        let velocity = look * inventory.throw_speed + player.velocity();
        let wait = inventory.pickup_delay;
        self.inventory.take(material, amount);
        let block = self.blocks.material_block(material);
        self.items.spawn_waiting(crate::Drop { material, amount }, block, &self.blocks, at, velocity, wait);
    }

    /// Works the tool in hand: the main use starts a swing when none is under way, held or
    /// pressed, and the strike lands at the end of the wind-up, aimed where the player looks
    /// on that tick. The second use guards while no swing is under way. Putting the tool away,
    /// or switching tools, drops the swing.
    fn use_tool(&mut self, input: &PlayerInput) {
        let dt = Self::TICK_SECONDS;
        let tool = input.tool.filter(|t| t.index() < self.tools.len());
        if self.swing.is_some_and(|s| Some(s.tool) != tool) {
            self.swing = None;
        }
        let Some(tool) = tool else {
            self.guarding = false;
            return;
        };
        if self.swing.is_none() && (input.use_pressed || input.use_held) {
            self.swing = Some(Swing { tool, stage: SwingStage::Windup, time: 0.0 });
        }
        self.guarding = self.swing.is_none() && input.alt_held;
        let Some(swing) = &mut self.swing else {
            return;
        };
        let def = &self.tools[tool.index()];
        swing.time += dt;
        match swing.stage {
            SwingStage::Windup if swing.time >= def.windup => {
                *swing = Swing { tool, stage: SwingStage::Recover, time: 0.0 };
                self.strike(tool, input.yaw, input.pitch);
            }
            SwingStage::Recover if swing.time >= def.recover => self.swing = None,
            _ => {}
        }
    }

    /// A strike with `tool` looking along `yaw` and `pitch`. Blocks in the way cut its reach.
    fn strike(&mut self, tool: ToolId, yaw: f32, pitch: f32) {
        let Some(player) = &self.player else {
            return;
        };
        let def = &self.tools[tool.index()];
        let (eye, look, carry) = (player.eye(), look_direction(yaw, pitch), player.velocity());
        let grid = self.universe.main();
        let blocks = &self.blocks;
        let wall = raycast(eye, look, def.reach, |p, out| {
            for (i, &o) in grid.cell(p).iter().enumerate() {
                if blocks.is_solid(o.block) {
                    out.extend(occupant_boxes(o, blocks).map(|b| (b, i)));
                }
            }
        });
        let reach = wall.map_or(def.reach, |w| w.distance);
        let hit = self.targets.strike(&self.target_defs, def, eye, look, reach, carry);
        self.strikes.push(Strike { tick: self.tick, tool, eye, look, reach, hit });
    }

    /// Removes one occupant from a boxel, by its index in the boxel's list. It drops its
    /// material as a loose item, from the middle of the space it filled, if items are defined.
    fn remove_occupant(&mut self, pos: BoxelPos, index: usize) {
        let grid = self.universe.main_mut();
        let mut cell = grid.cell(pos).to_vec();
        if index >= cell.len() {
            return;
        }
        let occupant = cell.remove(index);
        grid.set_cell(pos, &cell);
        self.changed.mark(pos);
        if let (Some(def), Some(drop)) = (&self.item_drop_def, self.blocks.drop(occupant.block)) {
            let boxes: Vec<_> = occupant_boxes(occupant, &self.blocks).collect();
            let middle = CellBox::bounds(&boxes).map_or(Vec3::splat(0.5), |b| (b.min + b.max) * 0.5);
            let seed = self.tick ^ (pos.0.x as u64).rotate_left(21) ^ (pos.0.y as u64).rotate_left(42) ^ pos.0.z as u64;
            let at = WorldPos::at_boxel(pos).translate(middle);
            self.items.spawn(drop, occupant.block, &self.blocks, at, pop_velocity(def, seed));
        }
    }

    /// Adds an occupant to a boxel. A solid block appearing inside the player pushes it out, if
    /// there's a rule and room.
    fn add_occupant(&mut self, pos: BoxelPos, occupant: Occupant) -> Option<(PushOut, f32)> {
        let push = match (&self.player, &self.push_out_def) {
            (Some(player), Some(def)) if self.blocks.is_solid(occupant.block) => {
                let boxes: Vec<_> = occupant_boxes(occupant, &self.blocks).collect();
                self.push_out(player, pos, &boxes).map(|push| (push, def.strength))
            }
            _ => None,
        };
        let grid = self.universe.main_mut();
        let mut cell = grid.cell(pos).to_vec();
        cell.push(occupant);
        grid.set_cell(pos, &cell);
        self.changed.mark(pos);
        if let (Some((push, strength)), Some(player)) = (push, &mut self.player) {
            player.apply_push_out(push, strength);
        }
        push
    }

    /// Whether `hit` is on a crafting table. Every face of it opens the same table.
    pub fn is_table(&self, hit: &Hit) -> bool {
        let Some(table) = self.crafting_def.as_ref().and_then(|def| self.blocks.id(&def.table)) else {
            return false;
        };
        self.universe.main().cell(hit.pos).get(hit.occupant).is_some_and(|o| o.block == table)
    }

    /// What crafting `craft` would make, without making it: the same checks a craft makes.
    /// The table must be in the player's reach.
    pub fn craft_plan(&self, craft: &Craft) -> Result<CraftPlan, CraftRefusal> {
        let def = self.crafting_def.as_ref().ok_or(CraftRefusal::NoCrafting)?;
        let table = self.blocks.id(&def.table).ok_or(CraftRefusal::NoCrafting)?;
        if !self.universe.main().cell(craft.table).iter().any(|o| o.block == table) {
            return Err(CraftRefusal::NoTable);
        }
        let reach = self.interaction_def.as_ref().map_or(0.0, |d| d.reach);
        let player = self.player.as_ref().ok_or(CraftRefusal::TooFar)?;
        let center = WorldPos::at_boxel(craft.table).translate(Vec3::splat(0.5));
        // Measured to the table's middle, so a whole face in reach counts.
        if player.eye().delta_to(center).length() > reach + 0.5 {
            return Err(CraftRefusal::TooFar);
        }
        crate::crafting::plan(&self.blocks, &self.inventory, self.capacity(), &craft.cells, craft.flip, craft.count)
    }

    /// Crafts: makes the parts `craft` asks for, if it can.
    fn craft(&mut self, craft: &Craft) -> Result<(MaterialId, u32), CraftRefusal> {
        let plan = self.craft_plan(craft)?;
        let capacity = self.capacity();
        let part = crate::crafting::make(&mut self.blocks, &mut self.inventory, capacity, &plan)?;
        Ok((part, plan.count))
    }

    /// The last craft's outcome: the part made and how many, or why nothing was.
    pub fn last_craft(&self) -> Option<Result<(MaterialId, u32), CraftRefusal>> {
        self.last_craft
    }

    pub fn crafting_def(&self) -> Option<&CraftingDef> {
        self.crafting_def.as_ref()
    }

    /// The block of `material` in `shape`, registering it if it's new, as crafting does. For
    /// tests and tools.
    pub fn shaped_block(&mut self, material: MaterialId, shape: &ShapeGrid) -> Result<BlockId, ApiError> {
        self.blocks.shaped(material, shape)
    }

    pub fn teleport_player(&mut self, pos: WorldPos) {
        if let Some(player) = &mut self.player {
            player.teleport(pos);
        }
    }

    /// Queues an instant velocity change for the player's next tick.
    pub fn apply_player_impulse(&mut self, impulse: Vec3) {
        if let Some(player) = &mut self.player {
            player.apply_impulse(impulse);
        }
    }

    pub fn blocks(&self) -> &BlockRegistry {
        &self.blocks
    }

    pub fn looks(&self) -> &LookRegistry {
        &self.looks
    }

    pub fn player_def(&self) -> Option<&PlayerDef> {
        self.player_def.as_ref()
    }

    pub fn interaction_def(&self) -> Option<&InteractionDef> {
        self.interaction_def.as_ref()
    }

    /// For engine tools such as the tuning panel.
    pub fn interaction_def_mut(&mut self) -> Option<&mut InteractionDef> {
        self.interaction_def.as_mut()
    }

    pub fn push_out_def(&self) -> Option<&PushOutDef> {
        self.push_out_def.as_ref()
    }

    /// For engine tools such as the tuning panel.
    pub fn push_out_def_mut(&mut self) -> Option<&mut PushOutDef> {
        self.push_out_def.as_mut()
    }

    pub fn crowd_def(&self) -> Option<&CrowdDef> {
        self.crowd_def.as_ref()
    }

    /// For engine tools such as the tuning panel.
    pub fn crowd_def_mut(&mut self) -> Option<&mut CrowdDef> {
        self.crowd_def.as_mut()
    }

    pub fn item_drop_def(&self) -> Option<&ItemDropDef> {
        self.item_drop_def.as_ref()
    }

    /// For engine tools such as the tuning panel.
    pub fn item_drop_def_mut(&mut self) -> Option<&mut ItemDropDef> {
        self.item_drop_def.as_mut()
    }

    /// The loose items lying in the world.
    pub fn items(&self) -> &Items {
        &self.items
    }

    /// Drops `amount` of `material` with its center at `pos`, at rest. For tests and tools; in
    /// play, items come from breaking blocks.
    pub fn spawn_item(&mut self, material: MaterialId, amount: u32, pos: WorldPos) {
        let block = self.blocks.material_block(material);
        self.items.spawn(crate::Drop { material, amount }, block, &self.blocks, pos, Vec3::ZERO);
    }

    /// What the player carries.
    pub fn inventory(&self) -> &Inventory {
        &self.inventory
    }

    /// How much the player can carry, every material together, in [`crate::FULL_VOLUME`] units,
    /// or `None` for no limit: without an inventory definition, or one with no capacity.
    pub fn capacity(&self) -> Option<u32> {
        self.inventory_def.as_ref().and_then(|def| def.capacity).map(capacity_units)
    }

    pub fn inventory_def(&self) -> Option<&InventoryDef> {
        self.inventory_def.as_ref()
    }

    /// For engine tools such as the tuning panel.
    pub fn inventory_def_mut(&mut self) -> Option<&mut InventoryDef> {
        self.inventory_def.as_mut()
    }

    /// Every registered model, in registration order.
    pub fn models(&self) -> &[ModelDef] {
        &self.models
    }

    /// Every registered sound, in registration order.
    pub fn sounds(&self) -> &[SoundDef] {
        &self.sounds
    }

    /// Every registered target definition, by [`TargetKind`] index.
    pub fn target_defs(&self) -> &[TargetDef] {
        &self.target_defs
    }

    pub fn target_def(&self, kind: TargetKind) -> &TargetDef {
        &self.target_defs[kind.index()]
    }

    /// For engine tools such as the tuning panel.
    pub fn target_defs_mut(&mut self) -> &mut [TargetDef] {
        &mut self.target_defs
    }

    /// Every registered tool, by [`ToolId`] index.
    pub fn tools(&self) -> &[ToolDef] {
        &self.tools
    }

    /// Every tool's ID, in registration order.
    pub fn tool_ids(&self) -> impl Iterator<Item = ToolId> {
        (0..self.tools.len()).map(|i| ToolId(i as u16))
    }

    pub fn tool(&self, id: ToolId) -> &ToolDef {
        &self.tools[id.index()]
    }

    /// For engine tools such as the tuning panel.
    pub fn tools_mut(&mut self) -> &mut [ToolDef] {
        &mut self.tools
    }

    /// Registered brains' names, in registration order.
    pub fn brain_names(&self) -> impl Iterator<Item = &str> {
        self.brains.iter().map(|(name, _)| name.as_str())
    }

    /// A brain by its index in [`Simulation::brain_names`], for engine tools such as the tuning
    /// panel.
    pub fn brain_mut(&mut self, index: usize) -> Option<&mut dyn Brain> {
        self.brains.get_mut(index).map(|(_, b)| &mut **b as &mut dyn Brain)
    }

    /// Whether targets' brains run. Off, every target is a dummy that only moves when hit.
    pub fn brains_on(&self) -> bool {
        !self.brains_off
    }

    pub fn set_brains_on(&mut self, on: bool) {
        self.brains_off = !on;
    }

    /// The targets in the world, broken ones excluded.
    pub fn targets(&self) -> &Targets {
        &self.targets
    }

    /// Puts every target back at its spot, whole.
    pub fn respawn_targets(&mut self) {
        self.targets.respawn_all(&self.target_defs);
    }

    /// The tool swing under way, if any.
    pub fn swing(&self) -> Option<&Swing> {
        self.swing.as_ref()
    }

    /// Whether the player is guarding with the tool in hand.
    pub fn guarding(&self) -> bool {
        self.guarding
    }

    /// Targets slammed into walls since the last call, oldest first.
    pub fn take_slams(&mut self) -> Vec<crate::Slam> {
        self.targets.take_slams()
    }

    /// Tool strikes since the last call, hits and misses, oldest first.
    pub fn take_strikes(&mut self) -> Vec<Strike> {
        std::mem::take(&mut self.strikes)
    }

    pub fn universe(&self) -> &Universe {
        &self.universe
    }

    pub fn player(&self) -> Option<&Player> {
        self.player.as_ref()
    }

    /// For engine tools such as the tuning panel.
    pub fn player_mut(&mut self) -> Option<&mut Player> {
        self.player.as_mut()
    }

    pub fn spawn(&self) -> WorldPos {
        self.spawn
    }

    pub fn waypoints(&self) -> &[(String, WorldPos)] {
        &self.waypoints
    }

    /// The player's last placement attempt, refused ones included.
    pub fn last_place(&self) -> Option<&PlaceReport> {
        self.last_place.as_ref()
    }

    /// Ticks run so far.
    pub fn tick(&self) -> u64 {
        self.tick
    }
}

struct GridTerrain<'a> {
    grid: &'a Grid,
    blocks: &'a BlockRegistry,
}

impl Terrain for GridTerrain<'_> {
    fn boxes(&self, pos: BoxelPos, out: &mut Vec<CellBox>) {
        for &o in self.grid.cell(pos) {
            if self.blocks.is_solid(o.block) {
                out.extend(occupant_boxes(o, self.blocks));
            }
        }
    }

    /// The first solid occupant's friction.
    fn friction(&self, pos: BoxelPos) -> f32 {
        let cell = self.grid.cell(pos);
        let def = cell.iter().filter_map(|o| self.blocks.get(o.block)).find(|def| def.solid);
        def.map_or(1.0, |def| def.friction)
    }
}

/// Terrain with extra solid boxes in one boxel, for checking a block that hasn't been placed yet.
struct WithBoxes<'a, T> {
    inner: T,
    pos: BoxelPos,
    boxes: &'a [CellBox],
}

impl<T: Terrain> Terrain for WithBoxes<'_, T> {
    fn boxes(&self, pos: BoxelPos, out: &mut Vec<CellBox>) {
        self.inner.boxes(pos, out);
        if pos == self.pos {
            out.extend_from_slice(self.boxes);
        }
    }

    fn friction(&self, pos: BoxelPos) -> f32 {
        self.inner.friction(pos)
    }
}

/// Half the width of the square in the middle of a face, for precise placement.
pub const FACE_CENTER: f32 = 0.25;

/// Which part of the aimed-at face a hit is in, for precise placement: `None` for the square
/// in the middle, or the outward direction of the nearest edge, for the four triangles around
/// it (drawn corner to corner, so each edge gets an equal share). The parts span the whole
/// boxel across the face, even on a partial block's face.
pub fn face_region(hit: &Hit) -> Option<[i32; 3]> {
    let axis = hit.normal.abs().max_position();
    let offset = hit.point - Vec3::splat(0.5);
    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
    if offset[a].abs() < FACE_CENTER && offset[b].abs() < FACE_CENTER {
        return None;
    }
    let edge = if offset[a].abs() >= offset[b].abs() { a } else { b };
    let mut dir = [0i32; 3];
    dir[edge] = if offset[edge] < 0.0 { -1 } else { 1 };
    Some(dir)
}

/// What the placement rules look at.
struct Aim {
    /// The outward normal of the face the block goes against.
    face: I64Vec3,
    look: Vec3,
    /// Aimed at the upper half of the boxel the block goes into.
    upper: bool,
    /// For precise placement, the part of the face aimed at (see [`face_region`]); `None` for
    /// basic placement.
    region: Option<Option<[i32; 3]>>,
}

/// How a block placed with `aim` is turned, kept as far as `turning` allows.
///
/// Basic placement is like Minecraft's: the block stands upright with its front (+Y) the way
/// the player looks, and upside down against a ceiling or when aimed at the upper half of a
/// wall; a block that only turns its axis lines up with the face.
///
/// Precise placement puts the block's bottom against the face aimed at, and looks at the part
/// of the face aimed at to turn it about the face: in the middle square its front points the
/// way the player looks along the face, in one of the four triangles toward that edge. A slab
/// lies flat against a wall, a post sticks out of it. A block that only turns its axis lines
/// up with the face in the middle, and lies toward the edge in a triangle.
fn orient(turning: Turning, aim: &Aim) -> Orientation {
    let normal = aim.face.to_array().map(|c| c as i32);
    let axis = aim.face.abs().max_position();
    // The look rounded to the strongest axis other than `skip`. Looking straight along `skip`
    // leaves no direction, so any one will do.
    let look_except = |skip: usize| {
        let mut along = aim.look;
        along[skip] = 0.0;
        let strongest = along.abs().max_position();
        let mut dir = [0i32; 3];
        if strongest == skip {
            dir[(skip + 1) % 3] = 1;
        } else {
            dir[strongest] = if along[strongest] < 0.0 { -1 } else { 1 };
        }
        dir
    };
    let (up, forward) = match aim.region {
        None => {
            let upside_down = normal[2] == -1 || (normal[2] == 0 && aim.upper);
            let up = if upside_down { [0, 0, -1] } else { [0, 0, 1] };
            if turning == Turning::Axis {
                (normal, look_except(axis))
            } else {
                (up, look_except(2))
            }
        }
        Some(None) => (normal, look_except(axis)),
        // A block that only turns its axis lies along the triangle's edge, like a log on its side.
        Some(Some(edge)) if turning == Turning::Axis => (edge.map(|c| -c), normal.map(|c| -c)),
        Some(Some(edge)) => (normal, edge),
    };
    match turning {
        Turning::None => Orientation::IDENTITY,
        Turning::Axis => Orientation::z_along(up.iter().position(|&c| c != 0).expect("a unit axis")),
        Turning::Any => Orientation::from_axes(up, forward).expect("two axes at right angles"),
    }
}

fn add_controller(
    controllers: &mut HashMap<String, Box<dyn MovementController>>,
    name: &str,
    controller: Box<dyn MovementController>,
) -> Result<(), ApiError> {
    if !is_valid_name(name) {
        return Err(ApiError::InvalidName(name.into()));
    }
    if controllers.contains_key(name) {
        return Err(ApiError::DuplicateName(name.into()));
    }
    controllers.insert(name.into(), controller);
    Ok(())
}

struct ContentRegistrar<'a> {
    looks: &'a mut LookRegistry,
    blocks: &'a mut BlockRegistry,
    controllers: &'a mut HashMap<String, Box<dyn MovementController>>,
    brains: &'a mut Vec<(String, Box<dyn Brain>)>,
    player: &'a mut Option<PlayerDef>,
    interaction: &'a mut Option<InteractionDef>,
    push_out: &'a mut Option<PushOutDef>,
    crowd: &'a mut Option<CrowdDef>,
    item_drops: &'a mut Option<ItemDropDef>,
    inventory: &'a mut Option<InventoryDef>,
    crafting: &'a mut Option<CraftingDef>,
    models: &'a mut Vec<ModelDef>,
    sounds: &'a mut Vec<SoundDef>,
    targets: &'a mut Vec<TargetDef>,
    tools: &'a mut Vec<ToolDef>,
}

/// Checks `name` is a valid name that `taken` doesn't already hold.
fn check_new_name(name: &str, mut taken: impl Iterator<Item = String>) -> Result<(), ApiError> {
    if !is_valid_name(name) {
        return Err(ApiError::InvalidName(name.into()));
    }
    if taken.any(|t| t == name) {
        return Err(ApiError::DuplicateName(name.into()));
    }
    Ok(())
}

impl Registrar for ContentRegistrar<'_> {
    fn register_look(&mut self, def: LookDef) -> Result<(), ApiError> {
        self.looks.register(def)
    }

    fn register_block(&mut self, def: BlockDef) -> Result<(), ApiError> {
        self.blocks.register(def).map(|_| ())
    }

    fn register_movement_controller(
        &mut self,
        name: &str,
        controller: Box<dyn MovementController>,
    ) -> Result<(), ApiError> {
        add_controller(self.controllers, name, controller)
    }

    fn register_brain(&mut self, name: &str, brain: Box<dyn Brain>) -> Result<(), ApiError> {
        check_new_name(name, self.brains.iter().map(|(n, _)| n.clone()))?;
        self.brains.push((name.into(), brain));
        Ok(())
    }

    fn register_player(&mut self, def: PlayerDef) -> Result<(), ApiError> {
        if self.player.is_some() {
            return Err(ApiError::DuplicatePlayerDef);
        }
        def.validate().map_err(ApiError::InvalidDef)?;
        *self.player = Some(def);
        Ok(())
    }

    fn register_interaction(&mut self, def: InteractionDef) -> Result<(), ApiError> {
        if self.interaction.is_some() {
            return Err(ApiError::DuplicateInteractionDef);
        }
        def.validate().map_err(ApiError::InvalidDef)?;
        *self.interaction = Some(def);
        Ok(())
    }

    fn register_push_out(&mut self, def: PushOutDef) -> Result<(), ApiError> {
        if self.push_out.is_some() {
            return Err(ApiError::DuplicatePushOutDef);
        }
        def.validate().map_err(ApiError::InvalidDef)?;
        *self.push_out = Some(def);
        Ok(())
    }

    fn register_crowd(&mut self, def: CrowdDef) -> Result<(), ApiError> {
        if self.crowd.is_some() {
            return Err(ApiError::DuplicateCrowdDef);
        }
        def.validate().map_err(ApiError::InvalidDef)?;
        *self.crowd = Some(def);
        Ok(())
    }

    fn register_item_drops(&mut self, def: ItemDropDef) -> Result<(), ApiError> {
        if self.item_drops.is_some() {
            return Err(ApiError::DuplicateItemDropDef);
        }
        def.validate().map_err(ApiError::InvalidDef)?;
        *self.item_drops = Some(def);
        Ok(())
    }

    fn register_inventory(&mut self, def: InventoryDef) -> Result<(), ApiError> {
        if self.inventory.is_some() {
            return Err(ApiError::DuplicateInventoryDef);
        }
        def.validate().map_err(ApiError::InvalidDef)?;
        *self.inventory = Some(def);
        Ok(())
    }

    fn register_crafting(&mut self, def: CraftingDef) -> Result<(), ApiError> {
        if self.crafting.is_some() {
            return Err(ApiError::DuplicateCraftingDef);
        }
        if !is_valid_name(&def.table) {
            return Err(ApiError::InvalidName(def.table));
        }
        *self.crafting = Some(def);
        Ok(())
    }

    fn register_model(&mut self, def: ModelDef) -> Result<(), ApiError> {
        check_new_name(&def.name, self.models.iter().map(|m| m.name.clone()))?;
        def.validate().map_err(|why| ApiError::InvalidDef(format!("model {}: {why}", def.name)))?;
        self.models.push(def);
        Ok(())
    }

    fn register_sound(&mut self, def: SoundDef) -> Result<(), ApiError> {
        check_new_name(&def.name, self.sounds.iter().map(|s| s.name.clone()))?;
        self.sounds.push(def);
        Ok(())
    }

    fn register_target(&mut self, def: TargetDef) -> Result<(), ApiError> {
        check_new_name(&def.name, self.targets.iter().map(|t| t.name.clone()))?;
        def.validate().map_err(|why| ApiError::InvalidDef(format!("target {}: {why}", def.name)))?;
        self.targets.push(def);
        Ok(())
    }

    fn register_tool(&mut self, def: ToolDef) -> Result<(), ApiError> {
        check_new_name(&def.name, self.tools.iter().map(|t| t.name.clone()))?;
        def.validate().map_err(|why| ApiError::InvalidDef(format!("tool {}: {why}", def.name)))?;
        self.tools.push(def);
        Ok(())
    }
}

struct WorldEditor<'a> {
    blocks: &'a mut BlockRegistry,
    grid: &'a mut Grid,
    spawn: &'a mut WorldPos,
    waypoints: &'a mut Vec<(String, WorldPos)>,
    target_defs: &'a [TargetDef],
    targets: &'a mut Targets,
}

impl WorldEditor<'_> {
    fn add(&mut self, pos: BoxelPos, id: BlockId, orientation: Orientation) -> Result<(), ApiError> {
        let occupant = Occupant::turned(id, orientation);
        let mut cell = self.grid.cell(pos).to_vec();
        can_share(&cell, occupant, self.blocks).map_err(|e| {
            let name = self.blocks.get(id).map_or("?", |d| d.name.as_str());
            ApiError::CannotShare(format!("{name} at {:?}: {e}", pos.0.to_array()))
        })?;
        cell.push(occupant);
        self.grid.set_cell(pos, &cell);
        Ok(())
    }
}

impl WorldEdit for WorldEditor<'_> {
    fn set_block(&mut self, pos: BoxelPos, name: &str) -> Result<(), ApiError> {
        let id = self.blocks.id(name).ok_or_else(|| ApiError::UnknownBlock(name.into()))?;
        self.grid.set_block(pos, id);
        Ok(())
    }

    fn add_block(&mut self, pos: BoxelPos, name: &str, orientation: Orientation) -> Result<(), ApiError> {
        let id = self.blocks.id(name).ok_or_else(|| ApiError::UnknownBlock(name.into()))?;
        self.add(pos, id, orientation)
    }

    fn add_shaped(
        &mut self,
        pos: BoxelPos,
        material: &str,
        shape: &ShapeGrid,
        orientation: Orientation,
    ) -> Result<(), ApiError> {
        let m = self.blocks.material(material).ok_or_else(|| ApiError::UnknownMaterial(material.into()))?;
        let id = self.blocks.shaped(m, shape)?;
        self.add(pos, id, orientation)
    }

    fn clear_block(&mut self, pos: BoxelPos) {
        self.grid.set_block(pos, crate::BlockId::EMPTY);
    }

    fn set_spawn(&mut self, pos: WorldPos) {
        *self.spawn = pos;
    }

    fn add_waypoint(&mut self, name: &str, pos: WorldPos) {
        self.waypoints.push((name.into(), pos));
    }

    fn add_target(&mut self, name: &str, pos: WorldPos, yaw: f32) -> Result<(), ApiError> {
        let index = self.target_defs.iter().position(|d| d.name == name).ok_or_else(|| ApiError::UnknownTarget(name.into()))?;
        self.targets.add_spot(TargetKind(index as u16), pos, yaw, &self.target_defs[index]);
        Ok(())
    }
}
