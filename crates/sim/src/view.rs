//! A read-only look at the world, for questions about it that both the simulation and a client
//! ask: what the player aims at, what placing a block there would do, what a craft would make.
//! The simulation answers them on its own state; a client answers them on its copy of it, the
//! same way, so a preview shows what the simulation will do.

use api::{
    BoxelPos, CraftingDef, I64Vec3, InteractionDef, InventoryDef, Orientation, PushOutDef, TargetDef, Turning, Vec3, WorldPos,
    look_direction,
};

use crate::crafting::{Craft, CraftPlan, CraftRefusal};
use crate::inventory::capacity_units;
use crate::player::{Body, PushOut, Terrain};
use crate::{
    BlockId, BlockRegistry, CellBox, Grid, Hit, Inventory, Occupant, Target, can_share, occupant_boxes, raycast,
};

/// How far in front of an aimed-at face the point that picks the boxel for a placed block is.
const IN_FRONT: f32 = 1e-3;

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

/// Everything the questions below look at, borrowed from the simulation or from a client's copy.
#[derive(Clone, Copy)]
pub struct View<'a> {
    pub blocks: &'a BlockRegistry,
    pub grid: &'a Grid,
    pub player: Option<&'a Body>,
    pub targets: &'a [Target],
    pub target_defs: &'a [TargetDef],
    pub interaction: Option<&'a InteractionDef>,
    pub push_out: Option<&'a PushOutDef>,
    pub crafting: Option<&'a CraftingDef>,
    pub inventory: &'a Inventory,
    pub inventory_def: Option<&'a InventoryDef>,
}

impl View<'_> {
    /// The occupant the player would break looking along `yaw` and `pitch`, and the face a
    /// placed block would go on. `None` with nothing in reach, no player, or no mod defining
    /// interaction.
    pub fn target(&self, yaw: f32, pitch: f32) -> Option<Hit> {
        let player = self.player?;
        let reach = self.interaction?.reach;
        let (grid, blocks) = (self.grid, self.blocks);
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
            if can_share(self.grid.cell(pos), occupant, self.blocks).is_ok() {
                let allowed = self.can_pay(block) && self.body_allows(pos, occupant);
                return Some(PlaceTarget { pos, occupant, allowed });
            }
            first.get_or_insert(PlaceTarget { pos, occupant, allowed: false });
        }
        first
    }

    /// Whether the player carries enough to place `block`: its volume of its material. Always,
    /// without an inventory definition or for a block made of no material.
    pub fn can_pay(&self, block: BlockId) -> bool {
        match (self.inventory_def, self.blocks.drop(block)) {
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
        let boxes: Vec<_> = occupant_boxes(occupant, self.blocks).collect();
        if self.targets.iter().any(|t| t.overlaps_boxes(&self.target_defs[t.kind.index()], pos, &boxes)) {
            return false;
        }
        let Some(player) = self.player else {
            return true;
        };
        !player.overlaps_boxes(pos, &boxes) || self.push_out(player, pos, &boxes).is_some()
    }

    /// How `player` would get out of a solid block filling `boxes` at `pos`, if there's a rule
    /// and room.
    pub fn push_out(&self, player: &Body, pos: BoxelPos, boxes: &[CellBox]) -> Option<PushOut> {
        let def = self.push_out?;
        let inner = GridTerrain { grid: self.grid, blocks: self.blocks };
        let terrain = WithBoxes { inner, pos, boxes };
        player.push_out(pos, boxes, def.upward_bias, &terrain)
    }

    /// Whether `hit` is on a crafting table. Every face of it opens the same table.
    pub fn is_table(&self, hit: &Hit) -> bool {
        let Some(table) = self.crafting.and_then(|def| self.blocks.id(&def.table)) else {
            return false;
        };
        self.grid.cell(hit.pos).get(hit.occupant).is_some_and(|o| o.block == table)
    }

    /// What crafting `craft` would make, without making it: the same checks a craft makes.
    /// The table must be in the player's reach.
    pub fn craft_plan(&self, craft: &Craft) -> Result<CraftPlan, CraftRefusal> {
        let def = self.crafting.ok_or(CraftRefusal::NoCrafting)?;
        let table = self.blocks.id(&def.table).ok_or(CraftRefusal::NoCrafting)?;
        if !self.grid.cell(craft.table).iter().any(|o| o.block == table) {
            return Err(CraftRefusal::NoTable);
        }
        let reach = self.interaction.map_or(0.0, |d| d.reach);
        let player = self.player.ok_or(CraftRefusal::TooFar)?;
        let center = WorldPos::at_boxel(craft.table).translate(Vec3::splat(0.5));
        // Measured to the table's middle, so a whole face in reach counts.
        if player.eye().delta_to(center).length() > reach + 0.5 {
            return Err(CraftRefusal::TooFar);
        }
        crate::crafting::plan(self.blocks, self.inventory, self.capacity(), &craft.cells, craft.flip, craft.count)
    }

    /// How much the player can carry, every material together, in [`crate::FULL_VOLUME`] units,
    /// or `None` for no limit: without an inventory definition, or one with no capacity.
    pub fn capacity(&self) -> Option<u32> {
        self.inventory_def.and_then(|def| def.capacity).map(capacity_units)
    }
}

/// A grid's solid blocks, as terrain bodies collide with.
pub(crate) struct GridTerrain<'a> {
    pub grid: &'a Grid,
    pub blocks: &'a BlockRegistry,
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
