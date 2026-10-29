//! The simulation: loads mods, owns the world and advances it at a fixed rate.

use std::collections::HashMap;
use std::fmt;

use api::block::is_valid_name;
use api::{
    ApiError, BlockDef, BoxelPos, ChunkPos, I64Vec3, InteractionDef, Mod, MovementController, Orientation, PlayerDef,
    PushOutDef, Registrar, TextureDef, Turning, Vec3, WorldEdit, WorldPos, look_direction,
};

use crate::edit::{ChangedChunks, Repeater};
use crate::player::{PushOut, Terrain};
use crate::{
    BlockId, BlockRegistry, CellBox, Grid, Hit, Occupant, Player, PlayerInput, TextureRegistry, Universe, can_share,
    occupant_boxes, raycast,
};

/// How far in front of an aimed-at face the point that picks the boxel for a placed block is.
const IN_FRONT: f32 = 1e-3;

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

#[derive(Default)]
pub struct Simulation {
    blocks: BlockRegistry,
    textures: TextureRegistry,
    /// Movement controllers by name, as registered. Each body gets its own from `spawn`.
    controllers: HashMap<String, Box<dyn MovementController>>,
    player_def: Option<PlayerDef>,
    interaction_def: Option<InteractionDef>,
    push_out_def: Option<PushOutDef>,
    universe: Universe,
    spawn: WorldPos,
    waypoints: Vec<(String, WorldPos)>,
    player: Option<Player>,
    breaking: Repeater,
    placing: Repeater,
    changed: ChangedChunks,
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
                textures: &mut sim.textures,
                blocks: &mut sim.blocks,
                controllers: &mut sim.controllers,
                player: &mut sim.player_def,
                interaction: &mut sim.interaction_def,
                push_out: &mut sim.push_out_def,
            })?;
        }
        sim.textures.check_blocks(&sim.blocks)?;
        for m in mods {
            m.build_world(&mut WorldEditor {
                blocks: &sim.blocks,
                grid: sim.universe.main_mut(),
                spawn: &mut sim.spawn,
                waypoints: &mut sim.waypoints,
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

    /// Advances one fixed tick. Block edits happen first, from where the player stood at the
    /// end of the last tick, which is what the client aimed from.
    pub fn step(&mut self, input: &PlayerInput) {
        self.edit_blocks(input);
        let Self { blocks, universe, player, .. } = self;
        if let Some(player) = player {
            let terrain = GridTerrain { grid: universe.main(), blocks };
            player.step(input, Self::TICK_SECONDS, &terrain);
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
                return Some(PlaceTarget { pos, occupant, allowed: self.body_allows(pos, occupant) });
            }
            first.get_or_insert(PlaceTarget { pos, occupant, allowed: false });
        }
        first
    }

    /// Whether the player's body leaves room for `occupant` at `pos`: it isn't solid, doesn't
    /// overlap the body, or the body can be pushed out.
    fn body_allows(&self, pos: BoxelPos, occupant: Occupant) -> bool {
        let Some(player) = &self.player else {
            return true;
        };
        if !self.blocks.is_solid(occupant.block) {
            return true;
        }
        let boxes: Vec<_> = occupant_boxes(occupant, &self.blocks).collect();
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
        let interval = 1.0 / def.repeat_rate;
        let dt = Self::TICK_SECONDS;
        let breaking = self.breaking.update(input.break_pressed, input.break_held, interval, dt);
        let placing = self.placing.update(input.place_pressed, input.place_held, interval, dt);
        if !breaking && !placing {
            return;
        }
        let Some(hit) = self.target(input.yaw, input.pitch) else {
            return;
        };
        if breaking {
            self.remove_occupant(hit.pos, hit.occupant);
        } else if let Some(place) =
            self.placement(&hit, input.place_block, input.yaw, input.pitch, input.place_precise)
            && place.allowed
        {
            self.add_occupant(place.pos, place.occupant);
        }
    }

    /// Removes one occupant from a boxel, by its index in the boxel's list.
    fn remove_occupant(&mut self, pos: BoxelPos, index: usize) {
        let grid = self.universe.main_mut();
        let mut cell = grid.cell(pos).to_vec();
        if index < cell.len() {
            cell.remove(index);
            grid.set_cell(pos, &cell);
            self.changed.mark(pos);
        }
    }

    /// Adds an occupant to a boxel. A solid block appearing inside the player pushes it out, if
    /// there's a rule and room.
    fn add_occupant(&mut self, pos: BoxelPos, occupant: Occupant) {
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

    pub fn textures(&self) -> &TextureRegistry {
        &self.textures
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
const FACE_CENTER: f32 = 0.25;

/// Which part of the aimed-at face a hit is in, for precise placement: `None` for the square
/// in the middle, or the outward direction of the nearest edge, for the four triangles around
/// it (drawn corner to corner, so each edge gets an equal share).
fn face_region(hit: &Hit) -> Option<[i32; 3]> {
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
/// Precise placement looks at the part of the face aimed at. In the middle square the block's
/// bottom goes against the face, its front the way the player looks along the face. In one of
/// the four triangles it stands in the half of the boxel toward that edge, with its back
/// against the face: on a wall the top triangle makes a top slab, a side triangle a slab
/// standing against that side.
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
        Some(Some(edge)) => (edge.map(|c| -c), normal.map(|c| -c)),
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
    textures: &'a mut TextureRegistry,
    blocks: &'a mut BlockRegistry,
    controllers: &'a mut HashMap<String, Box<dyn MovementController>>,
    player: &'a mut Option<PlayerDef>,
    interaction: &'a mut Option<InteractionDef>,
    push_out: &'a mut Option<PushOutDef>,
}

impl Registrar for ContentRegistrar<'_> {
    fn register_texture(&mut self, def: TextureDef) -> Result<(), ApiError> {
        self.textures.register(def)
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
}

struct WorldEditor<'a> {
    blocks: &'a BlockRegistry,
    grid: &'a mut Grid,
    spawn: &'a mut WorldPos,
    waypoints: &'a mut Vec<(String, WorldPos)>,
}

impl WorldEdit for WorldEditor<'_> {
    fn set_block(&mut self, pos: BoxelPos, name: &str) -> Result<(), ApiError> {
        let id = self.blocks.id(name).ok_or_else(|| ApiError::UnknownBlock(name.into()))?;
        self.grid.set_block(pos, id);
        Ok(())
    }

    fn add_block(&mut self, pos: BoxelPos, name: &str, orientation: Orientation) -> Result<(), ApiError> {
        let id = self.blocks.id(name).ok_or_else(|| ApiError::UnknownBlock(name.into()))?;
        let occupant = Occupant::turned(id, orientation);
        let mut cell = self.grid.cell(pos).to_vec();
        can_share(&cell, occupant, self.blocks)
            .map_err(|e| ApiError::CannotShare(format!("{name} at {:?}: {e}", pos.0.to_array())))?;
        cell.push(occupant);
        self.grid.set_cell(pos, &cell);
        Ok(())
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
}
