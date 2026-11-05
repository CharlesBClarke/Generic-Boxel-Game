//! The simulation: loads mods, owns the world and advances it at a fixed rate.

use std::collections::HashMap;
use std::fmt;

use api::block::is_valid_name;
use api::{
    ApiError, BlockDef, BoxelPos, Brain, ChunkPos, CraftingDef, CrowdDef, HealthDef, InteractionDef, InventoryDef, ItemDropDef, LookDef,
    ModelDef, Mod, MovementController, Orientation, PlayerDef, PushOutDef, Registrar, ShapeGrid, SoundDef,
    TargetDef, ToolDef, Vec2, Vec3, WorldEdit, WorldPos, look_direction,
};

use crate::combat::{StepContext, Strike, Swing, SwingStage, TargetKind, Targets, ToolId};
use crate::crafting::{Craft, CraftPlan, CraftRefusal};
use crate::edit::{ChangedChunks, Repeater};
use crate::inventory::{Inventory, capacity_units};
use crate::items::{Collector, Items, pop_velocity};
use crate::view::{GridTerrain, PlaceTarget, View};
use crate::{
    BlockId, BlockRegistry, CellBox, Grid, Hit, MaterialId, Occupant, Player, PlayerInput, LookRegistry, Universe, can_share,
    occupant_boxes, raycast,
};

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

/// The player being hurt: by a creature's attack, or by landing hard.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerHurt {
    pub tick: u64,
    /// The attacker's ID and kind; `None` for a fall.
    pub attacker: Option<(u64, TargetKind)>,
    /// Level unit direction from the player toward the attacker; zero for a fall.
    pub from: Vec3,
    /// Health lost, after the guard.
    pub damage: f32,
    /// The knockback given.
    pub push: Vec3,
    /// A raised guard caught it.
    pub guarded: bool,
    /// It took the last of the player's health, and they went back to the spawn point.
    pub died: bool,
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
    health_def: Option<HealthDef>,
    /// The player's health, and seconds since they were last hurt.
    health: f32,
    since_hurt: f32,
    /// Out of health doesn't send the player back; health stops at none.
    unkillable: bool,
    /// Blows and falls hurting the player since the last [`Simulation::take_player_hurts`].
    player_hurts: Vec<PlayerHurt>,
    /// The player's landings counted so far, so each is judged once.
    seen_landings: u32,
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
                health: &mut sim.health_def,
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
        sim.health = sim.health_def.as_ref().map_or(0.0, |d| d.health);
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
        self.health = self.health_def.as_ref().map_or(0.0, |d| d.health);
        Ok(())
    }

    /// Advances one fixed tick. Block edits and tool swings happen first, from where the
    /// player stood at the end of the last tick, which is what the client aimed from. Targets
    /// move after the player, and loose items after them, toward where the player ended up. Last,
    /// creatures' attacks that hit the player hurt them.
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
        self.hurt_player(input.yaw);
        self.tick += 1;
    }

    /// A read-only look at the world, for the questions in [`View`].
    pub fn view(&self) -> View<'_> {
        View {
            blocks: &self.blocks,
            grid: self.universe.main(),
            player: self.player.as_ref().map(|p| p.body()),
            targets: self.targets.as_slice(),
            target_defs: &self.target_defs,
            interaction: self.interaction_def.as_ref(),
            push_out: self.push_out_def.as_ref(),
            crafting: self.crafting_def.as_ref(),
            inventory: &self.inventory,
            inventory_def: self.inventory_def.as_ref(),
        }
    }

    /// See [`View::target`].
    pub fn target(&self, yaw: f32, pitch: f32) -> Option<Hit> {
        self.view().target(yaw, pitch)
    }

    /// See [`View::placement`].
    pub fn placement(&self, hit: &Hit, block: BlockId, yaw: f32, pitch: f32, precise: bool) -> Option<PlaceTarget> {
        self.view().placement(hit, block, yaw, pitch, precise)
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
        if let Some(place) = target.filter(|p| p.allowed) {
            if let (Some(_), Some(cost)) = (&self.inventory_def, self.blocks.drop(place.occupant.block)) {
                self.inventory.take(cost.material, cost.amount);
            }
            self.add_occupant(place.pos, place.occupant);
        }
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

    /// Hurts the player for a hard landing this tick, and hurts and knocks them back with the
    /// creatures' attacks that hit them, softened by a guard raised toward the attacker with the
    /// player looking along `yaw`. Lets health come back once they haven't been hurt for a
    /// while. Out of health, they go back to the spawn point, whole, and every creature is put
    /// back too if the health definition says so.
    fn hurt_player(&mut self, yaw: f32) {
        let blows = self.targets.take_blows();
        let Some(player) = &mut self.player else {
            return;
        };
        let dt = Self::TICK_SECONDS;
        self.since_hurt += dt;
        if let Some(def) = &self.health_def
            && self.since_hurt >= def.regen_delay
        {
            self.health = (self.health + def.regen * dt).min(def.health);
        }
        if player.landings() != self.seen_landings {
            self.seen_landings = player.landings();
            let speed = player.last_landing_speed();
            if let Some(def) = &self.health_def
                && speed > def.fall_speed
            {
                let damage = (speed - def.fall_speed) * def.fall_damage;
                self.health = (self.health - damage).max(0.0);
                self.since_hurt = 0.0;
                let died = self.health <= 0.0 && !self.unkillable;
                if died {
                    self.health = def.health;
                    player.teleport(self.spawn);
                    if def.knock_out_resets {
                        self.targets.respawn_all(&self.target_defs);
                    }
                }
                self.player_hurts.push(PlayerHurt {
                    tick: self.tick,
                    attacker: None,
                    from: Vec3::ZERO,
                    damage,
                    push: Vec3::ZERO,
                    guarded: false,
                    died,
                });
                if died {
                    return;
                }
            }
        }
        let look = Vec2::from_angle(yaw);
        for blow in blows {
            let a = &blow.attack;
            let toward = -blow.away.truncate();
            let guarded = self.guarding
                && self.health_def.as_ref().is_some_and(|d| look.angle_to(toward).abs() <= d.guard_angle);
            let (damage_share, push_share) = if guarded { (a.guarded_damage, a.guarded_push) } else { (1.0, 1.0) };
            // Knocked from wherever it was going: speed toward the attacker doesn't eat into the
            // knockback. Lift brings the upward speed up to it, from a fall too, but doesn't add
            // to a jump already rising, so a blow mid-jump throws no higher than one standing.
            let velocity = player.velocity();
            let back = velocity.dot(blow.away).min(0.0);
            let lift = a.lift * push_share;
            let up = if lift > 0.0 { (lift - velocity.z).max(0.0) } else { 0.0 };
            let push = blow.away * (a.push * push_share - back) + Vec3::Z * up;
            player.apply_impulse(push);
            let mut died = false;
            let mut damage = 0.0;
            if let Some(def) = &self.health_def {
                damage = (a.damage * damage_share).max(0.0);
                self.health = (self.health - damage).max(0.0);
                self.since_hurt = 0.0;
                if self.health <= 0.0 && !self.unkillable {
                    died = true;
                    self.health = def.health;
                    player.teleport(self.spawn);
                    if def.knock_out_resets {
                        self.targets.respawn_all(&self.target_defs);
                    }
                }
            }
            self.player_hurts.push(PlayerHurt {
                tick: self.tick,
                attacker: Some((blow.target, blow.kind)),
                from: toward.extend(0.0),
                damage,
                push,
                guarded,
                died,
            });
            if died {
                break;
            }
        }
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
        let mut recoil = Vec3::ZERO;
        if let (Some(hit), Some(player)) = (&hit, &mut self.player)
            && !player.on_ground()
            && !player.flying()
        {
            // Opposite the look, by the damage, but never past rising at the bounce.
            recoil = -look * def.recoil * hit.damage;
            if recoil.z > 0.0 {
                recoil.z = recoil.z.min((def.recoil_bounce - player.velocity().z).max(0.0));
            }
            player.apply_impulse(recoil);
        }
        self.strikes.push(Strike { tick: self.tick, tool, eye, look, reach, hit, recoil });
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
    fn add_occupant(&mut self, pos: BoxelPos, occupant: Occupant) {
        let push = match (&self.player, &self.push_out_def) {
            (Some(player), Some(def)) if self.blocks.is_solid(occupant.block) => {
                let boxes: Vec<_> = occupant_boxes(occupant, &self.blocks).collect();
                self.view().push_out(player, pos, &boxes).map(|push| (push, def.strength))
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
    }

    /// See [`View::is_table`].
    pub fn is_table(&self, hit: &Hit) -> bool {
        self.view().is_table(hit)
    }

    /// See [`View::craft_plan`].
    pub fn craft_plan(&self, craft: &Craft) -> Result<CraftPlan, CraftRefusal> {
        self.view().craft_plan(craft)
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

    pub fn health_def(&self) -> Option<&HealthDef> {
        self.health_def.as_ref()
    }

    /// For tuning while the game runs. Health above a lowered limit comes down to it.
    pub fn health_def_mut(&mut self) -> Option<&mut HealthDef> {
        self.health_def.as_mut()
    }

    /// The player's health; 0 without a health definition.
    pub fn health(&self) -> f32 {
        match &self.health_def {
            Some(def) => self.health.min(def.health),
            None => 0.0,
        }
    }

    /// Sets the player's health, within what the health definition allows.
    pub fn set_health(&mut self, health: f32) {
        self.health = self.health_def.as_ref().map_or(0.0, |d| health.clamp(0.0, d.health));
    }

    pub fn unkillable(&self) -> bool {
        self.unkillable
    }

    /// With it on, running out of health doesn't send the player back.
    pub fn set_unkillable(&mut self, on: bool) {
        self.unkillable = on;
    }

    /// Blows and falls that hurt the player since the last call, oldest first.
    pub fn take_player_hurts(&mut self) -> Vec<PlayerHurt> {
        std::mem::take(&mut self.player_hurts)
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

    /// See [`View::capacity`].
    pub fn capacity(&self) -> Option<u32> {
        self.view().capacity()
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
        ToolId::first(self.tools.len())
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

    /// Ticks run so far.
    pub fn tick(&self) -> u64 {
        self.tick
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
    health: &'a mut Option<HealthDef>,
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

    fn register_health(&mut self, def: HealthDef) -> Result<(), ApiError> {
        if self.health.is_some() {
            return Err(ApiError::DuplicateHealthDef);
        }
        def.validate().map_err(ApiError::InvalidDef)?;
        *self.health = Some(def);
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
