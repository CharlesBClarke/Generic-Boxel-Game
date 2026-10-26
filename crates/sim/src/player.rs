//! The player body. The mod's movement controller decides the velocity; the engine then
//! applies impulses and resolves collisions against solid boxels.
//!
//! All collision math happens relative to the player's own cell, so it is the same at any
//! distance from zero.

use api::{
    BoxelPos, Ground, I64Vec3, MoveContext, MoveInput, MovementController, PlayerDef, Vec2, Vec3, WorldPos,
};

use crate::BlockId;

/// Slack for touching faces, so standing on a floor doesn't count as being inside it.
const EPS: f32 = 1e-4;
/// How far below the feet still counts as standing on something.
const GROUND_PROBE: f32 = 0.01;
/// Debug fly speed, meters per second, and the multiplier while sprinting.
const FLY_SPEED: f32 = 12.0;
const FLY_SPRINT: f32 = 3.0;

/// What the body collides with and stands on.
pub trait Terrain {
    fn solid(&self, pos: BoxelPos) -> bool;
    /// Friction of the block at `pos`, from its definition.
    fn friction(&self, pos: BoxelPos) -> f32;
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
}

/// The body's collision shape.
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
    velocity: Vec3,
    pending_impulse: Vec3,
    on_ground: bool,
    flying: bool,
    landings: u32,
    last_landing_speed: f32,
    stepped_total: f64,
}

impl Player {
    /// A player standing with its feet at `pos`, with its own controller spawned from `def`'s.
    pub fn new(def: &PlayerDef, pos: WorldPos) -> Self {
        let shape = Shape {
            width: def.width,
            height: def.height,
            eye_height: def.eye_height,
            step_height: def.step_height,
        };
        Self::with_controller(shape, def.controller.spawn(), pos)
    }

    pub fn with_controller(shape: Shape, controller: Box<dyn MovementController>, pos: WorldPos) -> Self {
        Self {
            shape,
            controller,
            pos,
            prev_pos: pos,
            velocity: Vec3::ZERO,
            pending_impulse: Vec3::ZERO,
            on_ground: false,
            flying: false,
            landings: 0,
            last_landing_speed: 0.0,
            stepped_total: 0.0,
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

    /// Where the eyes are, without any camera effects.
    pub fn eye(&self) -> WorldPos {
        self.pos.translate(Vec3::new(0.0, 0.0, self.shape.eye_height))
    }

    /// Whether the collision box overlaps the boxel at `pos`. Touching faces don't count.
    pub fn overlaps(&self, pos: BoxelPos) -> bool {
        let (min, max) = self.bounds();
        let cell_min = (pos.0 - self.pos.cell()).as_vec3();
        let cell_max = cell_min + Vec3::ONE;
        (min.cmplt(cell_max - EPS) & max.cmpgt(cell_min + EPS)).all()
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
        };
        let impulse = std::mem::take(&mut self.pending_impulse);

        if self.flying {
            let speed = if input.sprint { FLY_SPEED * FLY_SPRINT } else { FLY_SPEED };
            let vertical = Vec3::Z * input.vertical.clamp(-1.0, 1.0);
            self.velocity = (move_input.wish_direction() + vertical) * speed;
        } else {
            let ground = self.on_ground.then(|| Ground { friction: self.ground_friction(terrain) });
            let out = self.controller.step(&MoveContext {
                dt,
                velocity: self.velocity,
                input: move_input,
                ground,
            });
            self.velocity = out.velocity + out.impulse + impulse;
        }

        let delta = self.velocity * dt;
        let (moved, stepped) = self.sweep(delta, terrain);
        let falling_speed = -self.velocity.z;
        for axis in 0..3 {
            if moved[axis] != delta[axis] {
                self.velocity[axis] = 0.0;
            }
        }
        self.pos = self.pos.translate(moved);
        self.stepped_total += stepped as f64;

        let was_on_ground = self.on_ground;
        self.on_ground = self.velocity.z <= 0.0 && self.touching_ground(terrain);
        if self.on_ground && !was_on_ground && stepped == 0.0 {
            self.landings += 1;
            self.last_landing_speed = falling_speed.max(0.0);
        }
    }

    /// The collision box relative to the player's cell.
    fn bounds(&self) -> (Vec3, Vec3) {
        let half = self.shape.width * 0.5;
        let offset = self.pos.offset();
        (offset - Vec3::new(half, half, 0.0), offset + Vec3::new(half, half, self.shape.height))
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
fn sweep_box(mut min: Vec3, mut max: Vec3, delta: Vec3, cell: I64Vec3, terrain: &impl Terrain) -> Vec3 {
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

/// How far a box from `min` to `max` (relative to `cell`) can move by `d` along `axis`.
fn clamp_axis(axis: usize, d: f32, min: Vec3, max: Vec3, cell: I64Vec3, terrain: &impl Terrain) -> f32 {
    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
    let span = |lo: f32, hi: f32| (lo + EPS).floor() as i64..=(hi - EPS).floor() as i64;
    let blocked = |i: i64| {
        span(min[a], max[a]).any(|j| {
            span(min[b], max[b]).any(|k| {
                let mut rel = [0i64; 3];
                rel[axis] = i;
                rel[a] = j;
                rel[b] = k;
                terrain.solid(BoxelPos(cell + I64Vec3::from_array(rel)))
            })
        })
    };

    if d > 0.0 {
        // Boxels whose near face is at or ahead of our leading face, nearest first.
        let first = (max[axis] - EPS).ceil() as i64;
        let last = (max[axis] + d).ceil() as i64 - 1;
        (first..=last).find(|&i| blocked(i)).map_or(d, |i| i as f32 - max[axis])
    } else {
        let first = (min[axis] + EPS).floor() as i64 - 1;
        let last = (min[axis] + d).floor() as i64;
        (last..=first).rev().find(|&i| blocked(i)).map_or(d, |i| (i + 1) as f32 - min[axis])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{MoveOutput, Tunable, Tunables};

    const DT: f32 = 1.0 / 60.0;

    /// A plain test controller: instant walking speed, constant gravity, jump while held.
    #[derive(Clone)]
    struct Walker;

    const WALK: f32 = 4.0;
    const GRAVITY: f32 = 25.0;
    const JUMP: f32 = 7.9;

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
            MoveOutput { velocity, impulse }
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
            MoveOutput { velocity: ctx.velocity - Vec3::Z * GRAVITY * ctx.dt, impulse: Vec3::ZERO }
        }
    }

    /// Terrain from a solidity function, with ice (friction 0.1) wherever `icy` says.
    struct World<F: Fn(BoxelPos) -> bool, G: Fn(BoxelPos) -> bool = fn(BoxelPos) -> bool> {
        solid: F,
        icy: G,
    }

    impl<F: Fn(BoxelPos) -> bool, G: Fn(BoxelPos) -> bool> Terrain for World<F, G> {
        fn solid(&self, pos: BoxelPos) -> bool {
            (self.solid)(pos)
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
