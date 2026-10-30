//! The player body. The mod's movement controller decides the velocity; the engine then
//! applies impulses and resolves collisions against the boxes of solid blocks.
//!
//! All collision math happens relative to the player's own cell, so it is the same at any
//! distance from zero.

use api::{
    BoxelPos, Ground, I64Vec3, MoveContext, MoveInput, MovementController, PlayerDef, Vec2, Vec3, WorldPos,
};

use crate::{BlockId, CellBox};

/// Slack for touching faces, so standing on a floor doesn't count as being inside it.
const EPS: f32 = 1e-4;
/// How far below the feet still counts as standing on something.
const GROUND_PROBE: f32 = 0.01;
/// Debug fly speed, meters per second, and the multiplier while sprinting.
const FLY_SPEED: f32 = 12.0;
const FLY_SPRINT: f32 = 3.0;
/// The shortest a body can get, whatever its controller asks for.
const MIN_HEIGHT: f32 = 0.2;

/// What the body collides with and stands on.
pub trait Terrain {
    /// Adds the solid boxes in the boxel at `pos`, in boxel space, to `out`.
    fn boxes(&self, pos: BoxelPos, out: &mut Vec<CellBox>);
    /// Friction of the block at `pos`, from its definition.
    fn friction(&self, pos: BoxelPos) -> f32;

    /// Whether anything solid is in the boxel at `pos`.
    fn solid(&self, pos: BoxelPos) -> bool {
        let mut boxes = Vec::new();
        self.boxes(pos, &mut boxes);
        !boxes.is_empty()
    }
}

/// One tick's worth of player intent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerInput {
    /// Horizontal movement in the player's frame: +x right, +y forward. Clamped to length 1.
    pub movement: Vec2,
    /// Fly mode only: +1 up, -1 down.
    pub vertical: f32,
    /// Facing, radians counterclockwise from +X (east) around +Z.
    pub yaw: f32,
    /// Look angle, radians up from level. Aims breaking and placing.
    pub pitch: f32,
    /// The jump key went down since the last tick.
    pub jump_pressed: bool,
    pub jump_held: bool,
    pub sprint: bool,
    /// The crouch key is down. Ignored while flying.
    pub crouch: bool,
    /// Switches fly mode on or off this tick.
    pub toggle_fly: bool,
    /// The break button went down since the last tick.
    pub break_pressed: bool,
    pub break_held: bool,
    /// The place button went down since the last tick.
    pub place_pressed: bool,
    pub place_held: bool,
    /// The block to place. Empty places nothing.
    pub place_block: BlockId,
    /// Precise placement, where the part of the face aimed at picks how a block sits, instead
    /// of basic placement.
    pub place_precise: bool,
}

/// How a body gets out of a block that appeared inside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PushOut {
    /// The move that clears the block.
    pub shift: Vec3,
    /// Unit direction of the launch.
    pub direction: Vec3,
}

/// How the last tick worked out the body's velocity, for tools that explain movement. Read
/// only; nothing in the engine acts on it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StepReport {
    /// The velocity going into the tick.
    pub velocity_in: Vec3,
    /// The velocity the controller gave back, before impulses.
    pub controller_velocity: Vec3,
    /// The controller's own impulse, such as a jump.
    pub controller_impulse: Vec3,
    /// Impulses from everything else, such as a push-out.
    pub external_impulse: Vec3,
    /// The velocity before collisions: the controller's plus all impulses.
    pub velocity_wanted: Vec3,
}

/// The body's collision shape, standing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub width: f32,
    pub height: f32,
    pub eye_height: f32,
    pub step_height: f32,
}

pub struct Player {
    shape: Shape,
    controller: Box<dyn MovementController>,
    pos: WorldPos,
    prev_pos: WorldPos,
    /// Current height; below the shape's while crouched.
    height: f32,
    velocity: Vec3,
    pending_impulse: Vec3,
    on_ground: bool,
    flying: bool,
    landings: u32,
    last_landing_speed: f32,
    stepped_total: f64,
    crouch_shift_total: f64,
    /// A drop the edge guard allowed, still being fallen: the guard's limit and the feet's height
    /// where the fall began. The guard keeps working until it lands, so the body can't drift off
    /// a narrow ledge on the way down.
    guarded_fall: Option<(f32, f64)>,
    last_step: StepReport,
}

impl Player {
    /// A player shaped by `def`, standing with its feet at `pos`.
    pub fn new(def: &PlayerDef, controller: Box<dyn MovementController>, pos: WorldPos) -> Self {
        let shape = Shape {
            width: def.width,
            height: def.height,
            eye_height: def.eye_height,
            step_height: def.step_height,
        };
        Self::with_controller(shape, controller, pos)
    }

    pub fn with_controller(shape: Shape, controller: Box<dyn MovementController>, pos: WorldPos) -> Self {
        Self {
            shape,
            controller,
            pos,
            prev_pos: pos,
            height: shape.height,
            velocity: Vec3::ZERO,
            pending_impulse: Vec3::ZERO,
            on_ground: false,
            flying: false,
            landings: 0,
            last_landing_speed: 0.0,
            stepped_total: 0.0,
            crouch_shift_total: 0.0,
            guarded_fall: None,
            last_step: StepReport::default(),
        }
    }

    pub fn shape(&self) -> Shape {
        self.shape
    }

    pub fn controller(&self) -> &dyn MovementController {
        self.controller.as_ref()
    }

    pub fn controller_mut(&mut self) -> &mut dyn MovementController {
        self.controller.as_mut()
    }

    /// Feet position after the latest tick.
    pub fn pos(&self) -> WorldPos {
        self.pos
    }

    /// Feet position before the latest tick, for interpolating between ticks.
    pub fn prev_pos(&self) -> WorldPos {
        self.prev_pos
    }

    /// Current height of the collision box.
    pub fn height(&self) -> f32 {
        self.height
    }

    /// Current camera height above the feet. The eyes stay the same distance below the top of
    /// the head at any height.
    pub fn eye_height(&self) -> f32 {
        (self.height - (self.shape.height - self.shape.eye_height)).max(MIN_HEIGHT * 0.5)
    }

    pub fn velocity(&self) -> Vec3 {
        self.velocity
    }

    pub fn on_ground(&self) -> bool {
        self.on_ground
    }

    pub fn flying(&self) -> bool {
        self.flying
    }

    /// How many times the player has landed. Lets a renderer notice landings it didn't see
    /// the tick of.
    pub fn landings(&self) -> u32 {
        self.landings
    }

    /// Downward speed at the most recent landing, meters per second.
    pub fn last_landing_speed(&self) -> f32 {
        self.last_landing_speed
    }

    /// Total height ever climbed by step-up. Lets a renderer smooth steps it didn't see the
    /// tick of.
    pub fn stepped_total(&self) -> f64 {
        self.stepped_total
    }

    /// Total distance the eyes have jumped from height changes: down when crouching on the
    /// ground, nothing for a mid-air tuck, where the head stays put. Lets a renderer ease
    /// changes it didn't see the tick of.
    pub fn crouch_shift_total(&self) -> f64 {
        self.crouch_shift_total
    }

    /// Where the eyes are, without any camera effects.
    pub fn eye(&self) -> WorldPos {
        self.pos.translate(Vec3::new(0.0, 0.0, self.eye_height()))
    }

    /// Whether the collision box overlaps the boxel at `pos`. Touching faces don't count.
    pub fn overlaps(&self, pos: BoxelPos) -> bool {
        self.overlaps_boxes(pos, &[CellBox::FULL])
    }

    /// Whether the collision box overlaps any of `boxes`, in the boxel at `pos`. Touching faces
    /// don't count.
    pub fn overlaps_boxes(&self, pos: BoxelPos, boxes: &[CellBox]) -> bool {
        let (min, max) = self.bounds();
        let at = (pos.0 - self.pos.cell()).as_vec3();
        boxes.iter().any(|b| overlap(min, max, b.translate(at)))
    }

    /// How the last tick worked out the velocity.
    pub fn last_step(&self) -> StepReport {
        self.last_step
    }

    /// Queues an instant velocity change for the next tick. Impulses from every source add up.
    pub fn apply_impulse(&mut self, impulse: Vec3) {
        self.pending_impulse += impulse;
    }

    /// Moves instantly, with no interpolation from the old spot.
    pub fn teleport(&mut self, pos: WorldPos) {
        self.pos = pos;
        self.prev_pos = pos;
        self.velocity = Vec3::ZERO;
        self.pending_impulse = Vec3::ZERO;
        self.on_ground = false;
    }

    /// Advances one tick of `dt` seconds.
    pub fn step(&mut self, input: &PlayerInput, dt: f32, terrain: &impl Terrain) {
        self.prev_pos = self.pos;
        if input.toggle_fly {
            self.flying = !self.flying;
            self.velocity = Vec3::ZERO;
        }
        let move_input = MoveInput {
            movement: input.movement,
            yaw: input.yaw,
            jump_pressed: input.jump_pressed,
            jump_held: input.jump_held,
            sprint: input.sprint,
            crouch: input.crouch && !self.flying,
        };
        let impulse = std::mem::take(&mut self.pending_impulse);
        let mut edge_guard = None;

        if self.flying {
            let speed = if input.sprint { FLY_SPEED * FLY_SPRINT } else { FLY_SPEED };
            let vertical = Vec3::Z * input.vertical.clamp(-1.0, 1.0);
            self.velocity = (move_input.wish_direction() + vertical) * speed;
            self.last_step = StepReport { velocity_wanted: self.velocity, ..StepReport::default() };
            self.resize(self.shape.height, terrain);
        } else {
            let ground = self.on_ground.then(|| Ground { friction: self.ground_friction(terrain) });
            let out = self.controller.step(&MoveContext {
                dt,
                velocity: self.velocity,
                height: self.height,
                standing_height: self.shape.height,
                input: move_input,
                ground,
            });
            self.last_step = StepReport {
                velocity_in: self.velocity,
                controller_velocity: out.velocity,
                controller_impulse: out.impulse,
                external_impulse: impulse,
                velocity_wanted: out.velocity + out.impulse + impulse,
            };
            self.velocity = out.velocity + out.impulse + impulse;
            self.resize(out.height.unwrap_or(self.shape.height), terrain);
            edge_guard = out.edge_guard.filter(|_| self.on_ground && self.velocity.z <= 0.0);
        }
        // Still falling a drop the guard allowed, crouched and not rising: guard what's left of it.
        let falling = self.velocity.z <= 0.0 && input.crouch && !self.flying && !self.on_ground;
        match self.guarded_fall {
            Some((drop, from)) if falling && edge_guard.is_none() => {
                edge_guard = Some((drop - (from - self.feet_z()) as f32).max(0.0));
            }
            Some(_) if !falling => self.guarded_fall = None,
            _ => {}
        }
        let guard_before = edge_guard;
        let feet_before = self.feet_z();

        let delta = self.velocity * dt;
        let (mut moved, stepped) = self.sweep(delta, terrain);
        if let Some(drop) = edge_guard
            && stepped == 0.0
        {
            moved = self.guard_edges(moved, drop, terrain);
        }
        let falling_speed = -self.velocity.z;
        for axis in 0..3 {
            if moved[axis] != delta[axis] {
                self.velocity[axis] = 0.0;
            }
        }
        self.pos = self.pos.translate(moved);
        // Step-ups are instant, like height changes: move the previous position too, so
        // renderers don't blend the rise between ticks on top of easing it themselves.
        self.prev_pos = self.prev_pos.translate(Vec3::Z * stepped);
        self.stepped_total += stepped as f64;

        let was_on_ground = self.on_ground;
        self.on_ground = self.velocity.z <= 0.0 && self.touching_ground(terrain);
        if self.on_ground {
            self.guarded_fall = None;
        } else if was_on_ground && let Some(drop) = guard_before {
            self.guarded_fall = Some((drop, feet_before));
        }
        if self.on_ground && !was_on_ground && stepped == 0.0 {
            self.landings += 1;
            self.last_landing_speed = falling_speed.max(0.0);
        }
    }

    /// How to get out of a block filling `boxes` in the boxel at `block`, which `terrain`
    /// already counts as solid, or `None` if there's no room. The block counts as the box
    /// around all its boxes. The launch points from that box's center to the body's, leaned
    /// upward by `upward_bias`. The body moves out along the launch's strongest axis if there's
    /// room that way, otherwise whichever way is shortest.
    pub fn push_out(
        &self,
        block: BoxelPos,
        boxes: &[CellBox],
        upward_bias: f32,
        terrain: &impl Terrain,
    ) -> Option<PushOut> {
        if !self.overlaps_boxes(block, boxes) {
            return None;
        }
        let (min, max) = self.bounds();
        let cell = self.pos.cell();
        let bounds = CellBox::bounds(boxes)?.translate((block.0 - cell).as_vec3());
        let (block_min, block_max) = (bounds.min, bounds.max);
        let away = ((min + max) * 0.5 - (block_min + block_max) * 0.5).normalize_or_zero();
        let direction = (away + Vec3::Z * upward_bias).normalize_or(Vec3::Z);

        let mut moves: Vec<Vec3> = (0..3)
            .flat_map(|axis| {
                let mut up = Vec3::ZERO;
                let mut down = Vec3::ZERO;
                up[axis] = block_max[axis] - min[axis];
                down[axis] = block_min[axis] - max[axis];
                [up, down]
            })
            .collect();
        let strongest = direction.abs().max_position();
        moves.sort_by(|a, b| {
            let along = |m: &Vec3| m[strongest] != 0.0 && m[strongest].signum() == direction[strongest].signum();
            along(b).cmp(&along(a)).then(a.length().total_cmp(&b.length()))
        });
        moves
            .into_iter()
            .find(|&shift| box_clear(min + shift, max + shift, cell, terrain))
            .map(|shift| PushOut { shift, direction })
    }

    /// Moves the body out of a block and launches it at `strength` meters per second.
    pub fn apply_push_out(&mut self, push: PushOut, strength: f32) {
        self.pos = self.pos.translate(push.shift);
        self.apply_impulse(push.direction * strength);
    }

    /// Cuts the horizontal parts of `moved` that would leave the body with nothing solid
    /// under it within `drop`, keeping whichever axis still has support, so it slides along
    /// edges instead of stopping dead.
    fn guard_edges(&self, moved: Vec3, drop: f32, terrain: &impl Terrain) -> Vec3 {
        let (min, max) = self.bounds();
        let cell = self.pos.cell();
        let reach = drop.max(0.0) + GROUND_PROBE;
        let supported = |m: Vec3| clamp_axis(2, -reach, min + m, max + m, cell, terrain) > -reach;
        if supported(moved) {
            return moved;
        }
        let x_only = Vec3::new(moved.x, 0.0, moved.z);
        let y_only = Vec3::new(0.0, moved.y, moved.z);
        let mut options = [x_only, y_only];
        options.sort_by(|a, b| b.truncate().length().total_cmp(&a.truncate().length()));
        options.into_iter().find(|&m| supported(m)).unwrap_or(Vec3::new(0.0, 0.0, moved.z))
    }

    /// Changes height toward `want`. Shrinking always works. Growing goes only as far as there
    /// is room: on the ground upward, keeping the feet put; in the air downward first, keeping
    /// the head put, then upward if the feet hit something. Height changes are instant, so they
    /// move the previous position too and aren't interpolated as motion.
    fn resize(&mut self, want: f32, terrain: &impl Terrain) {
        let want = want.clamp(MIN_HEIGHT, self.shape.height);
        let eye_before = self.eye_height();
        let mut feet = 0.0;
        if want < self.height {
            if !self.on_ground {
                feet = self.height - want;
            }
            self.height = want;
        } else if want > self.height {
            let mut grow = want - self.height;
            if !self.on_ground {
                let (min, max) = self.bounds();
                feet = clamp_axis(2, -grow, min, max, self.pos.cell(), terrain);
                self.height -= feet;
                grow += feet;
            }
            if grow > 0.0 {
                let (min, max) = self.bounds();
                let (min, max) = (min + Vec3::Z * feet, max + Vec3::Z * feet);
                self.height += clamp_axis(2, grow, min, max, self.pos.cell(), terrain).max(0.0);
            }
        }
        if feet != 0.0 {
            self.pos = self.pos.translate(Vec3::Z * feet);
            self.prev_pos = self.prev_pos.translate(Vec3::Z * feet);
        }
        self.crouch_shift_total += (feet + self.eye_height() - eye_before) as f64;
    }

    /// The feet's height above zero, for comparing heights across ticks.
    fn feet_z(&self) -> f64 {
        self.pos.cell().z as f64 + self.pos.offset().z as f64
    }

    /// The collision box relative to the player's cell.
    fn bounds(&self) -> (Vec3, Vec3) {
        let half = self.shape.width * 0.5;
        let offset = self.pos.offset();
        (offset - Vec3::new(half, half, 0.0), offset + Vec3::new(half, half, self.height))
    }

    fn touching_ground(&self, terrain: &impl Terrain) -> bool {
        let (min, max) = self.bounds();
        clamp_axis(2, -GROUND_PROBE, min, max, self.pos.cell(), terrain) > -GROUND_PROBE
    }

    /// Friction of what the player stands on: the block under the middle of the feet, or if
    /// that is empty (standing on an edge), the first solid block under the footprint.
    fn ground_friction(&self, terrain: &impl Terrain) -> f32 {
        let (min, max) = self.bounds();
        let below = (min.z - GROUND_PROBE).floor() as i64;
        let cell = self.pos.cell();
        let at = |x: f32, y: f32| BoxelPos(cell + I64Vec3::new(x.floor() as i64, y.floor() as i64, below));
        let center = (min + max) * 0.5;
        let spots = [
            at(center.x, center.y),
            at(min.x + EPS, min.y + EPS),
            at(max.x - EPS, min.y + EPS),
            at(min.x + EPS, max.y - EPS),
            at(max.x - EPS, max.y - EPS),
        ];
        spots.into_iter().find(|&p| terrain.solid(p)).map_or(1.0, |p| terrain.friction(p))
    }

    /// Moves the collision box by `delta`, stepping up ledges if allowed. Returns how far it
    /// moved and how high it stepped.
    fn sweep(&self, delta: Vec3, terrain: &impl Terrain) -> (Vec3, f32) {
        let (min, max) = self.bounds();
        let cell = self.pos.cell();
        let plain = sweep_box(min, max, delta, cell, terrain);
        let blocked_sideways = plain.x != delta.x || plain.y != delta.y;
        if self.shape.step_height <= 0.0 || !self.on_ground || !blocked_sideways {
            return (plain, 0.0);
        }

        // Try again from higher up: rise, move sideways, then settle back down.
        let up = clamp_axis(2, self.shape.step_height, min, max, cell, terrain);
        let lift = Vec3::new(0.0, 0.0, up);
        let side = sweep_box(min + lift, max + lift, Vec3::new(delta.x, delta.y, 0.0), cell, terrain);
        let raised = lift + side;
        let down = clamp_axis(2, -up + delta.z.min(0.0), min + raised, max + raised, cell, terrain);
        let stepped = Vec3::new(side.x, side.y, up + down);
        if side.truncate().length() > plain.truncate().length() + EPS && stepped.z > EPS {
            (stepped, stepped.z)
        } else {
            (plain, 0.0)
        }
    }
}

/// Moves a box one axis at a time (vertical first), stopping each axis at the first solid
/// boxel. Returns how far it actually moved.
pub(crate) fn sweep_box(mut min: Vec3, mut max: Vec3, delta: Vec3, cell: I64Vec3, terrain: &impl Terrain) -> Vec3 {
    let mut moved = Vec3::ZERO;
    for axis in [2, 0, 1] {
        if delta[axis] == 0.0 {
            continue;
        }
        let d = clamp_axis(axis, delta[axis], min, max, cell, terrain);
        min[axis] += d;
        max[axis] += d;
        moved[axis] = d;
    }
    moved
}

/// Whether two boxes overlap. Touching faces don't count.
fn overlap(min: Vec3, max: Vec3, b: CellBox) -> bool {
    (min.cmplt(b.max - EPS) & max.cmpgt(b.min + EPS)).all()
}

/// The solid boxes in every boxel the region from `min` to `max` (relative to `cell`) touches
/// or borders, relative to `cell`.
fn solid_boxes(min: Vec3, max: Vec3, cell: I64Vec3, terrain: &impl Terrain) -> Vec<CellBox> {
    let range = |axis: usize| (min[axis] - EPS).floor() as i64..=(max[axis] + EPS).floor() as i64;
    let mut out = Vec::new();
    let mut found = Vec::new();
    for z in range(2) {
        for y in range(1) {
            for x in range(0) {
                let rel = I64Vec3::new(x, y, z);
                terrain.boxes(BoxelPos(cell + rel), &mut found);
                out.extend(found.drain(..).map(|b| b.translate(rel.as_vec3())));
            }
        }
    }
    out
}

/// Whether a box from `min` to `max` (relative to `cell`) is free of solid boxes.
fn box_clear(min: Vec3, max: Vec3, cell: I64Vec3, terrain: &impl Terrain) -> bool {
    !solid_boxes(min, max, cell, terrain).into_iter().any(|b| overlap(min, max, b))
}

/// How far a box from `min` to `max` (relative to `cell`) can move by `d` along `axis`. Boxes
/// it already overlaps don't stop it.
pub(crate) fn clamp_axis(axis: usize, d: f32, min: Vec3, max: Vec3, cell: I64Vec3, terrain: &impl Terrain) -> f32 {
    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
    let (mut lo, mut hi) = (min, max);
    if d > 0.0 {
        hi[axis] += d;
    } else {
        lo[axis] += d;
    }
    let beside = |o: &CellBox| {
        [a, b].into_iter().all(|i| o.min[i] < max[i] - EPS && o.max[i] > min[i] + EPS)
    };
    solid_boxes(lo, hi, cell, terrain).into_iter().filter(beside).fold(d, |d, o| {
        if d > 0.0 && o.min[axis] >= max[axis] - EPS {
            d.min(o.min[axis] - max[axis])
        } else if d < 0.0 && o.max[axis] <= min[axis] + EPS {
            d.max(o.max[axis] - min[axis])
        } else {
            d
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{MoveOutput, Tunable, Tunables};

    const DT: f32 = 1.0 / 60.0;

    /// A plain test controller: instant walking speed, constant gravity, jump while held,
    /// crouch to 1 meter while held, with an edge guard while crouched on the ground.
    #[derive(Clone)]
    struct Walker;

    const WALK: f32 = 4.0;
    const GRAVITY: f32 = 25.0;
    const JUMP: f32 = 7.9;
    const CROUCH: f32 = 1.0;
    const GUARD_DROP: f32 = 0.6;

    impl Tunables for Walker {
        fn tunables(&self) -> Vec<Tunable> {
            Vec::new()
        }
        fn set_tunable(&mut self, _: &str, _: f32) -> bool {
            false
        }
    }

    impl MovementController for Walker {
        fn spawn(&self) -> Box<dyn MovementController> {
            Box::new(self.clone())
        }
        fn step(&mut self, ctx: &MoveContext) -> MoveOutput {
            let walk = ctx.input.wish_direction() * WALK;
            let mut velocity = Vec3::new(walk.x, walk.y, ctx.velocity.z - GRAVITY * ctx.dt);
            let mut impulse = Vec3::ZERO;
            if ctx.input.jump_held && ctx.ground.is_some() {
                velocity.z = 0.0;
                impulse.z = JUMP;
            }
            let crouched_on_ground = ctx.input.crouch && ctx.ground.is_some();
            MoveOutput {
                velocity,
                impulse,
                height: ctx.input.crouch.then_some(CROUCH),
                edge_guard: crouched_on_ground.then_some(GUARD_DROP),
            }
        }
    }

    /// A controller that reports the friction it was given, to check what the engine passes.
    struct FrictionSpy(std::sync::Arc<std::sync::Mutex<Option<f32>>>);

    impl Tunables for FrictionSpy {
        fn tunables(&self) -> Vec<Tunable> {
            Vec::new()
        }
        fn set_tunable(&mut self, _: &str, _: f32) -> bool {
            false
        }
    }

    impl MovementController for FrictionSpy {
        fn spawn(&self) -> Box<dyn MovementController> {
            Box::new(FrictionSpy(self.0.clone()))
        }
        fn step(&mut self, ctx: &MoveContext) -> MoveOutput {
            *self.0.lock().unwrap() = ctx.ground.as_ref().map(|g| g.friction);
            MoveOutput { velocity: ctx.velocity - Vec3::Z * GRAVITY * ctx.dt, ..Default::default() }
        }
    }

    /// Terrain from a solidity function, with ice (friction 0.1) wherever `icy` says.
    struct World<F: Fn(BoxelPos) -> bool, G: Fn(BoxelPos) -> bool = fn(BoxelPos) -> bool> {
        solid: F,
        icy: G,
    }

    impl<F: Fn(BoxelPos) -> bool, G: Fn(BoxelPos) -> bool> Terrain for World<F, G> {
        fn boxes(&self, pos: BoxelPos, out: &mut Vec<CellBox>) {
            if (self.solid)(pos) {
                out.push(CellBox::FULL);
            }
        }
        fn friction(&self, pos: BoxelPos) -> f32 {
            if (self.icy)(pos) { 0.1 } else { 1.0 }
        }
    }

    fn world(solid: impl Fn(BoxelPos) -> bool) -> World<impl Fn(BoxelPos) -> bool> {
        World { solid, icy: |_| false }
    }

    fn shape(step_height: f32) -> Shape {
        Shape { width: 0.6, height: 1.8, eye_height: 1.62, step_height }
    }

    fn player(x: f32, y: f32, z: f32) -> Player {
        Player::with_controller(shape(0.0), Box::new(Walker), at(x, y, z))
    }

    fn at(x: f32, y: f32, z: f32) -> WorldPos {
        WorldPos::new(I64Vec3::ZERO, Vec3::new(x, y, z))
    }

    fn height(p: &Player) -> f32 {
        p.pos().cell().z as f32 + p.pos().offset().z
    }

    /// Solid everywhere below z = 0.
    fn floor(p: BoxelPos) -> bool {
        p.0.z < 0
    }

    fn run(player: &mut Player, input: &PlayerInput, ticks: usize, terrain: &impl Terrain) {
        for _ in 0..ticks {
            player.step(input, DT, terrain);
        }
    }

    fn walk_east() -> PlayerInput {
        PlayerInput { movement: Vec2::Y, yaw: 0.0, ..Default::default() }
    }

    #[test]
    fn falls_and_lands_on_the_floor() {
        let mut p = player(0.5, 0.5, 5.0);
        run(&mut p, &PlayerInput::default(), 120, &world(floor));
        assert!(p.on_ground());
        assert_eq!(p.pos().cell().z, 0);
        assert!(p.pos().offset().z.abs() < 1e-3);
        assert_eq!(p.velocity().z, 0.0);
        assert_eq!(p.landings(), 1);
        assert!(p.last_landing_speed() > 10.0);
    }

    #[test]
    fn a_fast_fall_does_not_pass_through_the_floor() {
        let mut p = player(0.5, 0.5, 2.0);
        p.velocity.z = -500.0;
        run(&mut p, &PlayerInput::default(), 1, &world(floor));
        assert!(p.on_ground());
        assert_eq!(p.pos().cell().z, 0);
    }

    #[test]
    fn walls_stop_movement() {
        let w = world(|p| floor(p) || p.0.x >= 3);
        let mut p = player(0.5, 0.5, 0.0);
        run(&mut p, &walk_east(), 120, &w);
        let right_edge = p.pos().cell().x as f32 + p.pos().offset().x + 0.3;
        assert!((right_edge - 3.0).abs() < 1e-3, "right edge at {right_edge}");
        assert_eq!(p.velocity().x, 0.0);
    }

    #[test]
    fn walks_in_the_facing_direction() {
        let mut p = player(0.5, 0.5, 0.0);
        let north = PlayerInput { movement: Vec2::Y, yaw: std::f32::consts::FRAC_PI_2, ..Default::default() };
        run(&mut p, &north, 60, &world(floor));
        let d = at(0.5, 0.5, 0.0).delta_to(p.pos());
        assert!((d.y - 4.0).abs() < 0.01 && d.x.abs() < 0.01, "moved {d}");

        let mut p = player(0.5, 0.5, 0.0);
        let strafe_right = PlayerInput { movement: Vec2::X, yaw: 0.0, ..Default::default() };
        run(&mut p, &strafe_right, 60, &world(floor));
        let d = at(0.5, 0.5, 0.0).delta_to(p.pos());
        assert!((d.y + 4.0).abs() < 0.01, "moved {d}");
    }

    #[test]
    fn the_controller_only_sees_ground_while_standing() {
        let mut p = player(0.5, 0.5, 0.0);
        let jump = PlayerInput { jump_held: true, ..Default::default() };
        run(&mut p, &PlayerInput::default(), 5, &world(floor));
        run(&mut p, &jump, 10, &world(floor));
        assert!(!p.on_ground());
        assert!(height(&p) > 0.5, "jump never left the ground");
    }

    #[test]
    fn ceilings_stop_jumps() {
        let w = world(|p| floor(p) || p.0.z >= 2);
        let mut p = player(0.5, 0.5, 0.0);
        run(&mut p, &PlayerInput::default(), 5, &w);
        let jump = PlayerInput { jump_held: true, ..Default::default() };
        let mut peak = 0.0f32;
        for _ in 0..60 {
            p.step(&jump, DT, &w);
            peak = peak.max(height(&p));
        }
        assert!(peak <= 0.2 + 1e-3, "head went through the ceiling: feet peaked at {peak}");
    }

    #[test]
    fn falls_into_a_pit() {
        let w = world(|p| p.0.z < -5 || (floor(p) && !(p.0.x.abs() <= 1 && p.0.y.abs() <= 1)));
        let mut p = player(0.5, 0.5, 0.0);
        run(&mut p, &PlayerInput::default(), 120, &w);
        assert!(p.on_ground());
        assert_eq!(p.pos().cell().z, -5);
    }

    #[test]
    fn impulses_work_with_any_controller() {
        let w = world(floor);
        let mut p = player(0.5, 0.5, 0.0);
        run(&mut p, &PlayerInput::default(), 5, &w);
        p.apply_impulse(Vec3::new(0.0, 0.0, 5.0));
        p.apply_impulse(Vec3::new(0.0, 0.0, 5.0));
        p.step(&PlayerInput::default(), DT, &w);
        assert!((p.velocity().z - (10.0 - GRAVITY * DT)).abs() < 1e-4, "impulses should add up");
        assert!(!p.on_ground());
    }

    #[test]
    fn the_controller_is_told_the_friction_underfoot() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let ice = World { solid: floor, icy: |p: BoxelPos| p.0.x >= 0 };
        let mut p = Player::with_controller(shape(0.0), Box::new(FrictionSpy(seen.clone())), at(0.5, 0.5, 0.0));
        run(&mut p, &PlayerInput::default(), 3, &ice);
        assert_eq!(*seen.lock().unwrap(), Some(0.1));

        let mut p = Player::with_controller(shape(0.0), Box::new(FrictionSpy(seen.clone())), at(-0.5, 0.5, 0.0));
        run(&mut p, &PlayerInput::default(), 3, &ice);
        assert_eq!(*seen.lock().unwrap(), Some(1.0));

        let mut p = Player::with_controller(shape(0.0), Box::new(FrictionSpy(seen.clone())), at(0.5, 0.5, 3.0));
        run(&mut p, &PlayerInput::default(), 1, &ice);
        assert_eq!(*seen.lock().unwrap(), None, "no ground while falling");
    }

    #[test]
    fn step_up_climbs_low_ledges_only() {
        let one_high = world(|p| floor(p) || (p.0.x >= 2 && p.0.z == 0));
        let two_high = world(|p| floor(p) || (p.0.x >= 2 && p.0.z <= 1));

        let mut p = Player::with_controller(shape(1.0), Box::new(Walker), at(0.5, 0.5, 0.0));
        run(&mut p, &PlayerInput::default(), 5, &one_high);
        run(&mut p, &walk_east(), 60, &one_high);
        assert_eq!(p.pos().cell().z, 1, "should have stepped up");
        assert!(p.pos().cell().x >= 3);

        let mut p = Player::with_controller(shape(1.0), Box::new(Walker), at(0.5, 0.5, 0.0));
        run(&mut p, &PlayerInput::default(), 5, &two_high);
        run(&mut p, &walk_east(), 60, &two_high);
        assert_eq!(p.pos().cell().z, 0, "a two-high wall is too tall");

        let mut p = player(0.5, 0.5, 0.0);
        run(&mut p, &PlayerInput::default(), 5, &one_high);
        run(&mut p, &walk_east(), 60, &one_high);
        assert_eq!(p.pos().cell().z, 0, "step-up is off at height 0");
    }

    #[test]
    fn a_step_up_moves_the_previous_position_too() {
        // The rise is instant and the renderer eases it itself, so blending between ticks
        // must not show it a second time.
        let one_high = world(|p| floor(p) || (p.0.x >= 2 && p.0.z == 0));
        let mut p = Player::with_controller(shape(1.0), Box::new(Walker), at(1.5, 0.5, 0.0));
        run(&mut p, &PlayerInput::default(), 5, &one_high);
        for _ in 0..60 {
            let before = p.stepped_total();
            p.step(&walk_east(), DT, &one_high);
            if p.stepped_total() > before {
                let rise = p.prev_pos().delta_to(p.pos()).z;
                assert!(rise.abs() < 1e-4, "the tick blends a rise of {rise}");
                assert!(p.prev_pos().delta_to(p.pos()).x > 0.0, "but still blends the walk");
                return;
            }
        }
        panic!("never stepped up");
    }

    #[test]
    fn step_up_needs_headroom() {
        // A one-high ledge with a ceiling above it leaving only 1.5 boxels of room.
        let w = world(|p| floor(p) || (p.0.x >= 2 && (p.0.z == 0 || p.0.z == 2)));
        let mut p = Player::with_controller(shape(1.0), Box::new(Walker), at(0.5, 0.5, 0.0));
        run(&mut p, &PlayerInput::default(), 5, &w);
        run(&mut p, &walk_east(), 60, &w);
        assert_eq!(p.pos().cell().z, 0);
        assert!(p.pos().cell().x < 2);
    }

    #[test]
    fn fly_mode_ignores_the_controller_and_still_collides() {
        let w = world(floor);
        let mut p = player(0.5, 0.5, 3.0);
        run(&mut p, &PlayerInput { toggle_fly: true, ..Default::default() }, 1, &w);
        assert!(p.flying());
        run(&mut p, &PlayerInput::default(), 60, &w);
        assert!((height(&p) - 3.0).abs() < 1e-5);

        run(&mut p, &PlayerInput { vertical: -1.0, ..Default::default() }, 60, &w);
        assert_eq!(p.pos().cell().z, 0);
        assert!(p.pos().offset().z.abs() < 1e-3);
    }

    fn crouch() -> PlayerInput {
        PlayerInput { crouch: true, ..Default::default() }
    }

    fn head(p: &Player) -> f32 {
        height(p) + p.height()
    }

    #[test]
    fn crouching_on_the_ground_lowers_the_head_and_keeps_the_feet() {
        let w = world(floor);
        let mut p = player(0.5, 0.5, 0.0);
        run(&mut p, &PlayerInput::default(), 5, &w);
        run(&mut p, &crouch(), 1, &w);
        assert_eq!(p.height(), CROUCH);
        assert!(height(&p).abs() < 1e-4, "feet moved to {}", height(&p));
        assert!((p.eye_height() - 0.82).abs() < 1e-5, "eyes keep their distance below the head");
        assert!((p.crouch_shift_total() + 0.8).abs() < 1e-5, "the eyes dropped 0.8");
        assert_eq!(p.prev_pos(), p.pos(), "height changes aren't interpolated as motion");

        run(&mut p, &PlayerInput::default(), 1, &w);
        assert_eq!(p.height(), 1.8, "stands back up with room");
        assert!(p.crouch_shift_total().abs() < 1e-5);
    }

    #[test]
    fn cannot_stand_up_under_a_low_ceiling() {
        let open = world(floor);
        let low = world(|p| floor(p) || p.0.z == 1);
        let mut p = player(0.5, 0.5, 0.0);
        run(&mut p, &PlayerInput::default(), 5, &open);
        run(&mut p, &crouch(), 1, &open);
        // A ceiling exactly one boxel up: the crouched body just fits.
        run(&mut p, &PlayerInput::default(), 10, &low);
        assert!((p.height() - CROUCH).abs() < 1e-3, "stood up to {} through the ceiling", p.height());
        assert!(height(&p).abs() < 1e-4, "feet stay on the floor");

        // A ceiling 1.5 up lets it stand partway.
        let mid = world(|p| floor(p) || p.0.z >= 2);
        run(&mut p, &PlayerInput::default(), 1, &mid);
        assert!((p.height() - 1.8).abs() < 1e-3);
    }

    #[test]
    fn tucking_in_the_air_lifts_the_feet_and_keeps_the_head() {
        let w = world(floor);
        let mut p = player(0.5, 0.5, 0.0);
        run(&mut p, &PlayerInput::default(), 5, &w);
        run(&mut p, &PlayerInput { jump_held: true, ..Default::default() }, 1, &w);
        run(&mut p, &PlayerInput::default(), 3, &w);
        assert!(!p.on_ground());
        let (feet, top) = (height(&p), head(&p));
        let shift = p.crouch_shift_total();
        // The tuck happens before this tick's motion, so compare against the motion alone.
        let velocity = p.velocity().z - GRAVITY * DT;
        p.step(&crouch(), DT, &w);
        let moved = velocity * DT;
        assert!((height(&p) - (feet + 0.8 + moved)).abs() < 1e-4, "feet should rise 0.8");
        assert!((head(&p) - (top + moved)).abs() < 1e-4, "head should stay put");
        assert!((p.crouch_shift_total() - shift).abs() < 1e-5, "the eyes don't jump");
    }

    #[test]
    fn standing_up_in_the_air_reaches_down_then_up() {
        // Tucked just above the floor: standing extends the feet to the floor, then the head up.
        let w = world(floor);
        let mut p = player(0.5, 0.5, 0.3);
        p.height = CROUCH;
        p.resize(1.8, &w);
        assert!(height(&p).abs() < 1e-4, "feet reach the floor, at {}", height(&p));
        assert!((p.height() - 1.8).abs() < 1e-4);
        assert!((head(&p) - 1.8).abs() < 1e-4, "the rest of the growth went up");
    }

    #[test]
    fn a_block_at_the_feet_lifts_the_body_onto_it() {
        let block = BoxelPos::new(0, 0, 0);
        let w = world(move |p| floor(p) || p == block);
        let mut p = player(0.5, 0.5, 0.0);
        p.on_ground = true;
        let push = p.push_out(block, &[CellBox::FULL], 1.0, &w).expect("room above");
        assert!((push.shift - Vec3::Z).length() < 1e-5, "shift {}", push.shift);
        assert!((push.direction - Vec3::Z).length() < 1e-5, "straight up from the middle");
        p.apply_push_out(push, 6.0);
        p.step(&PlayerInput::default(), DT, &w);
        assert_eq!(p.pos().cell().z, 1);
        assert!(p.velocity().z > 5.0, "launched at {}", p.velocity().z);
    }

    #[test]
    fn the_far_edge_of_the_cell_tilts_the_launch() {
        let block = BoxelPos::new(0, 0, 0);
        let w = world(move |p| floor(p) || p == block);
        let p = player(0.85, 0.5, 0.0);
        let push = p.push_out(block, &[CellBox::FULL], 1.0, &w).unwrap();
        assert!((push.shift - Vec3::Z).length() < 1e-5, "still lifted onto the block");
        assert!(push.direction.x > 0.3, "tilted toward the far edge: {}", push.direction);
        assert_eq!(push.direction.y, 0.0);
        let centered = player(0.5, 0.5, 0.0).push_out(block, &[CellBox::FULL], 1.0, &w).unwrap();
        assert!(push.direction.z < centered.direction.z);
    }

    #[test]
    fn push_out_goes_sideways_without_headroom_and_fails_without_room() {
        let block = BoxelPos::new(0, 0, 0);
        // Room for only 2 boxels of height: lifting onto the block would hit the ceiling.
        let low = world(move |p| floor(p) || p == block || p.0.z >= 2);
        let p = player(0.85, 0.5, 0.0);
        let push = p.push_out(block, &[CellBox::FULL], 1.0, &low).expect("room to the side");
        assert!((push.shift - Vec3::new(0.45, 0.0, 0.0)).length() < 1e-4, "shift {}", push.shift);

        // A one-boxel shaft two high: nowhere to go.
        let shaft = world(move |p| floor(p) || p == block || p.0.z >= 2 || (p.0.z >= 0 && p.0 != I64Vec3::new(0, 0, p.0.z)));
        let p = player(0.5, 0.5, 0.0);
        assert_eq!(p.push_out(block, &[CellBox::FULL], 1.0, &shaft), None);
        assert_eq!(player(5.5, 5.5, 0.0).push_out(block, &[CellBox::FULL], 1.0, &world(floor)), None, "not inside it");
    }

    /// A floor that ends at x = 3, dropping 5 boxels, with a one-boxel step down at y < 0.
    fn cliff(p: BoxelPos) -> bool {
        (p.0.x < 3 && (if p.0.y < 0 { p.0.z < -1 } else { p.0.z < 0 })) || p.0.z < -5
    }

    fn crouch_walk(yaw: f32) -> PlayerInput {
        PlayerInput { movement: Vec2::Y, yaw, crouch: true, ..Default::default() }
    }

    fn right_edge(p: &Player) -> f32 {
        p.pos().cell().x as f32 + p.pos().offset().x + 0.3
    }

    #[test]
    fn crouching_stops_at_a_cliff_edge() {
        let w = world(cliff);
        let mut p = player(0.5, 0.5, 0.0);
        run(&mut p, &PlayerInput::default(), 5, &w);
        run(&mut p, &crouch_walk(0.0), 120, &w);
        assert!(p.on_ground(), "still on the ledge");
        assert_eq!(p.pos().cell().z, 0);
        // The body may hang over the edge, but never past it with nothing underfoot.
        let edge = right_edge(&p);
        assert!(edge > 3.0 && edge < 3.6 + 1e-3, "right side at {edge}");
        assert_eq!(p.velocity().x, 0.0, "stopped, not pushing");
    }

    #[test]
    fn crouching_slides_along_an_edge() {
        let w = world(cliff);
        let mut p = player(2.5, 0.5, 0.0);
        run(&mut p, &PlayerInput::default(), 5, &w);
        // Walk diagonally into the edge: the x part is cut, the y part keeps going.
        let start_y = p.pos().offset().y + p.pos().cell().y as f32;
        run(&mut p, &crouch_walk(std::f32::consts::FRAC_PI_4), 60, &w);
        let y = p.pos().offset().y + p.pos().cell().y as f32;
        assert!(y - start_y > 1.5, "slid only {}", y - start_y);
        assert_eq!(p.pos().cell().z, 0);
    }

    #[test]
    fn the_guard_allows_drops_up_to_its_limit() {
        // South of y = 0 the floor is one boxel lower. Standing at the step's edge, moving
        // south: a 0.6 guard stops at the one-boxel drop, a 1.2 guard lets the body go.
        let w = world(cliff);
        let mut p = player(0.5, 0.31, 0.0);
        p.on_ground = true;
        let south = Vec3::new(0.0, -0.7, 0.0);
        assert_eq!(p.guard_edges(south, 0.6, &w), Vec3::ZERO);
        assert_eq!(p.guard_edges(south, 1.2, &w), south);
        // Moving along the step is never cut.
        assert_eq!(p.guard_edges(Vec3::new(0.3, 0.0, 0.0), 0.6, &w), Vec3::new(0.3, 0.0, 0.0));
    }

    #[test]
    fn walking_without_crouch_goes_over_the_edge() {
        let w = world(cliff);
        let mut p = player(0.5, 0.5, 0.0);
        run(&mut p, &PlayerInput::default(), 5, &w);
        run(&mut p, &walk_east(), 120, &w);
        assert!(p.pos().cell().z < 0, "should have fallen");
    }

    #[test]
    fn jumping_off_while_crouched_still_works() {
        let w = world(cliff);
        let mut p = player(2.5, 0.5, 0.0);
        run(&mut p, &PlayerInput::default(), 5, &w);
        let jump = PlayerInput { jump_held: true, ..crouch_walk(0.0) };
        run(&mut p, &jump, 1, &w);
        run(&mut p, &crouch_walk(0.0), 120, &w);
        assert!(p.pos().cell().x >= 3 && p.pos().cell().z < 0, "jumped off the cliff");
    }

    #[test]
    fn teleport_skips_interpolation() {
        let mut p = player(0.5, 0.5, 0.0);
        let far = WorldPos::new(I64Vec3::new(3_000_000, 0, 0), Vec3::splat(0.5));
        p.teleport(far);
        assert_eq!(p.prev_pos(), far);
        assert_eq!(p.pos(), far);
    }

    #[test]
    fn movement_is_identical_in_the_millions() {
        let shift = I64Vec3::new(3_000_000, -2_000_000, 1_000_000);
        let near_solid = |p: BoxelPos| floor(p) || p.0.x >= 3 || (p.0.y >= 2 && p.0.z == 0);
        let near = world(near_solid);
        let far = world(move |p: BoxelPos| near_solid(BoxelPos(p.0 - shift)));
        let start = at(0.5, 0.5, 2.0);
        let far_start = WorldPos::new(start.cell() + shift, start.offset());
        let mut a = Player::with_controller(shape(1.0), Box::new(Walker), start);
        let mut b = Player::with_controller(shape(1.0), Box::new(Walker), far_start);

        let inputs = [
            PlayerInput { movement: Vec2::new(0.3, 1.0), yaw: 0.4, ..Default::default() },
            PlayerInput { movement: Vec2::Y, yaw: 0.0, jump_held: true, ..Default::default() },
            PlayerInput { movement: Vec2::new(-1.0, 0.2), yaw: 2.0, ..Default::default() },
        ];
        for input in inputs.iter().cycle().take(600) {
            a.step(input, DT, &near);
            b.step(input, DT, &far);
            assert_eq!(a.pos().offset(), b.pos().offset());
            assert_eq!(a.pos().cell() + shift, b.pos().cell());
        }
    }
}
