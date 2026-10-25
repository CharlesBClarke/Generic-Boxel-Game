//! The player body: movement, gravity and collision against solid boxels.
//!
//! All collision math happens relative to the player's own cell, so it is the same at any
//! distance from zero.

use api::{BoxelPos, I64Vec3, PlayerDef, Vec3, WorldPos};
use glam::Vec2;

/// Slack for touching faces, so standing on a floor doesn't count as being inside it.
const EPS: f32 = 1e-4;

/// One tick's worth of player intent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerInput {
    /// Horizontal movement in the player's frame: +x right, +y forward. Clamped to length 1.
    pub movement: Vec2,
    /// Fly mode only: +1 up, -1 down.
    pub vertical: f32,
    /// Facing, radians counterclockwise from +X (east) around +Z.
    pub yaw: f32,
    pub jump: bool,
    /// Switches fly mode on or off this tick.
    pub toggle_fly: bool,
}

#[derive(Clone, Debug)]
pub struct Player {
    def: PlayerDef,
    pos: WorldPos,
    prev_pos: WorldPos,
    velocity: Vec3,
    on_ground: bool,
    flying: bool,
}

impl Player {
    /// A player standing with its feet at `pos`.
    pub fn new(def: PlayerDef, pos: WorldPos) -> Self {
        Self { def, pos, prev_pos: pos, velocity: Vec3::ZERO, on_ground: false, flying: false }
    }

    pub fn def(&self) -> &PlayerDef {
        &self.def
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

    /// Moves instantly, with no interpolation from the old spot.
    pub fn teleport(&mut self, pos: WorldPos) {
        self.pos = pos;
        self.prev_pos = pos;
        self.velocity = Vec3::ZERO;
        self.on_ground = false;
    }

    /// Advances one tick of `dt` seconds. `solid` says whether a boxel blocks movement.
    pub fn step(&mut self, input: &PlayerInput, dt: f32, solid: impl Fn(BoxelPos) -> bool) {
        self.prev_pos = self.pos;
        if input.toggle_fly {
            self.flying = !self.flying;
            self.velocity = Vec3::ZERO;
        }

        let (sin, cos) = input.yaw.sin_cos();
        let forward = Vec3::new(cos, sin, 0.0);
        let right = Vec3::new(sin, -cos, 0.0);
        let movement = input.movement.clamp_length_max(1.0);
        let wish = right * movement.x + forward * movement.y;

        let delta = if self.flying {
            let vertical = Vec3::Z * input.vertical.clamp(-1.0, 1.0);
            self.velocity = (wish + vertical) * self.def.fly_speed;
            self.velocity * dt
        } else {
            let walk = wish * self.def.walk_speed;
            self.velocity.x = walk.x;
            self.velocity.y = walk.y;
            if input.jump && self.on_ground {
                self.velocity.z = (2.0 * self.def.gravity * self.def.jump_height).sqrt();
            }
            // Average of the old and new vertical speed, so jumps follow the exact parabola.
            let vz_before = self.velocity.z;
            self.velocity.z -= self.def.gravity * dt;
            Vec3::new(walk.x * dt, walk.y * dt, (vz_before + self.velocity.z) * 0.5 * dt)
        };

        let moved = self.sweep(delta, &solid);
        for axis in 0..3 {
            if moved[axis] != delta[axis] {
                self.velocity[axis] = 0.0;
            }
        }
        self.on_ground = delta.z < 0.0 && moved.z > delta.z;
        self.pos = self.pos.translate(moved);
    }

    /// Moves the collision box by `delta` one axis at a time (vertical first), stopping each
    /// axis at the first solid boxel. Returns how far it actually moved.
    fn sweep(&self, delta: Vec3, solid: &impl Fn(BoxelPos) -> bool) -> Vec3 {
        let half = self.def.width * 0.5;
        let mut min = self.pos.offset() - Vec3::new(half, half, 0.0);
        let mut max = self.pos.offset() + Vec3::new(half, half, self.def.height);
        let mut moved = Vec3::ZERO;
        for axis in [2, 0, 1] {
            if delta[axis] == 0.0 {
                continue;
            }
            let d = clamp_axis(axis, delta[axis], min, max, self.pos.cell(), solid);
            min[axis] += d;
            max[axis] += d;
            moved[axis] = d;
        }
        moved
    }
}

/// How far a box from `min` to `max` (relative to `cell`) can move by `d` along `axis`.
fn clamp_axis(
    axis: usize,
    d: f32,
    min: Vec3,
    max: Vec3,
    cell: I64Vec3,
    solid: &impl Fn(BoxelPos) -> bool,
) -> f32 {
    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
    let span = |lo: f32, hi: f32| (lo + EPS).floor() as i64..=(hi - EPS).floor() as i64;
    let blocked = |i: i64| {
        span(min[a], max[a]).any(|j| {
            span(min[b], max[b]).any(|k| {
                let mut rel = [0i64; 3];
                rel[axis] = i;
                rel[a] = j;
                rel[b] = k;
                solid(BoxelPos(cell + I64Vec3::from_array(rel)))
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

    const DT: f32 = 1.0 / 60.0;

    fn def() -> PlayerDef {
        PlayerDef {
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            walk_speed: 4.0,
            jump_height: 1.25,
            gravity: 25.0,
            fly_speed: 10.0,
        }
    }

    fn at(x: f32, y: f32, z: f32) -> WorldPos {
        WorldPos::new(I64Vec3::ZERO, Vec3::new(x, y, z))
    }

    /// Solid everywhere below z = 0.
    fn floor(p: BoxelPos) -> bool {
        p.0.z < 0
    }

    fn run(player: &mut Player, input: &PlayerInput, ticks: usize, solid: impl Fn(BoxelPos) -> bool) {
        for _ in 0..ticks {
            player.step(input, DT, &solid);
        }
    }

    fn walk_east() -> PlayerInput {
        PlayerInput { movement: Vec2::Y, yaw: 0.0, ..Default::default() }
    }

    #[test]
    fn falls_and_lands_on_the_floor() {
        let mut p = Player::new(def(), at(0.5, 0.5, 5.0));
        run(&mut p, &PlayerInput::default(), 120, floor);
        assert!(p.on_ground());
        assert_eq!(p.pos().cell().z, 0);
        assert!(p.pos().offset().z.abs() < 1e-3);
        assert_eq!(p.velocity().z, 0.0);
    }

    #[test]
    fn a_fast_fall_does_not_pass_through_the_floor() {
        let mut p = Player::new(def(), at(0.5, 0.5, 2.0));
        p.velocity.z = -500.0;
        run(&mut p, &PlayerInput::default(), 1, floor);
        assert!(p.on_ground());
        assert_eq!(p.pos().cell().z, 0);
    }

    #[test]
    fn walls_stop_movement() {
        // A wall filling x >= 3.
        let world = |p: BoxelPos| floor(p) || p.0.x >= 3;
        let mut p = Player::new(def(), at(0.5, 0.5, 0.0));
        run(&mut p, &walk_east(), 120, world);
        let right_edge = p.pos().cell().x as f32 + p.pos().offset().x + 0.3;
        assert!((right_edge - 3.0).abs() < 1e-3, "right edge at {right_edge}");
        assert_eq!(p.velocity().x, 0.0);
    }

    #[test]
    fn walks_in_the_facing_direction() {
        let mut p = Player::new(def(), at(0.5, 0.5, 0.0));
        let north = PlayerInput { movement: Vec2::Y, yaw: std::f32::consts::FRAC_PI_2, ..Default::default() };
        run(&mut p, &north, 60, floor);
        let d = at(0.5, 0.5, 0.0).delta_to(p.pos());
        assert!((d.y - 4.0).abs() < 0.01 && d.x.abs() < 0.01, "moved {d}");

        let mut p = Player::new(def(), at(0.5, 0.5, 0.0));
        let strafe_right = PlayerInput { movement: Vec2::X, yaw: 0.0, ..Default::default() };
        run(&mut p, &strafe_right, 60, floor);
        let d = at(0.5, 0.5, 0.0).delta_to(p.pos());
        assert!((d.y + 4.0).abs() < 0.01, "moved {d}");
    }

    #[test]
    fn jump_reaches_the_configured_height() {
        let mut p = Player::new(def(), at(0.5, 0.5, 0.0));
        run(&mut p, &PlayerInput::default(), 5, floor);
        let jump = PlayerInput { jump: true, ..Default::default() };
        let mut peak = 0.0f32;
        p.step(&jump, DT, floor);
        for _ in 0..120 {
            p.step(&PlayerInput::default(), DT, floor);
            peak = peak.max(p.pos().cell().z as f32 + p.pos().offset().z);
        }
        assert!((peak - 1.25).abs() < 0.02, "peak {peak}");
        assert!(p.on_ground());
    }

    #[test]
    fn cannot_jump_in_midair() {
        let mut p = Player::new(def(), at(0.5, 0.5, 10.0));
        let jump = PlayerInput { jump: true, ..Default::default() };
        run(&mut p, &jump, 10, floor);
        assert!(p.velocity().z < 0.0);
    }

    #[test]
    fn can_jump_onto_a_one_boxel_step() {
        let step = |p: BoxelPos| floor(p) || (p.0.x >= 2 && p.0.z == 0);
        let mut p = Player::new(def(), at(0.5, 0.5, 0.0));
        run(&mut p, &PlayerInput::default(), 5, step);
        let jump_east = PlayerInput { jump: true, ..walk_east() };
        run(&mut p, &jump_east, 60, step);
        assert_eq!(p.pos().cell().z, 1);
        assert!(p.pos().cell().x >= 2);
    }

    #[test]
    fn ceilings_stop_jumps() {
        // Exactly 2 boxels of headroom.
        let room = |p: BoxelPos| floor(p) || p.0.z >= 2;
        let mut p = Player::new(def(), at(0.5, 0.5, 0.0));
        run(&mut p, &PlayerInput::default(), 5, room);
        let jump = PlayerInput { jump: true, ..Default::default() };
        let mut peak = 0.0f32;
        p.step(&jump, DT, room);
        for _ in 0..60 {
            p.step(&PlayerInput::default(), DT, room);
            peak = peak.max(p.pos().offset().z + p.pos().cell().z as f32);
        }
        assert!(peak <= 0.2 + 1e-3, "head went through the ceiling: feet peaked at {peak}");
    }

    #[test]
    fn falls_into_a_pit() {
        // A 3×3 pit, 5 deep, around the origin.
        let pit = |p: BoxelPos| p.0.z < -5 || (floor(p) && !(p.0.x.abs() <= 1 && p.0.y.abs() <= 1));
        let mut p = Player::new(def(), at(0.5, 0.5, 0.0));
        run(&mut p, &PlayerInput::default(), 120, pit);
        assert!(p.on_ground());
        assert_eq!(p.pos().cell().z, -5);
    }

    #[test]
    fn fly_mode_ignores_gravity_and_still_collides() {
        let mut p = Player::new(def(), at(0.5, 0.5, 3.0));
        run(&mut p, &PlayerInput { toggle_fly: true, ..Default::default() }, 1, floor);
        assert!(p.flying());
        run(&mut p, &PlayerInput::default(), 60, floor);
        assert!((p.pos().offset().z + p.pos().cell().z as f32 - 3.0).abs() < 1e-5);

        run(&mut p, &PlayerInput { vertical: -1.0, ..Default::default() }, 60, floor);
        assert_eq!(p.pos().cell().z, 0);
        assert!(p.pos().offset().z.abs() < 1e-3);
    }

    #[test]
    fn teleport_skips_interpolation() {
        let mut p = Player::new(def(), at(0.5, 0.5, 0.0));
        let far = WorldPos::new(I64Vec3::new(3_000_000, 0, 0), Vec3::splat(0.5));
        p.teleport(far);
        assert_eq!(p.prev_pos(), far);
        assert_eq!(p.pos(), far);
    }

    #[test]
    fn movement_is_identical_in_the_millions() {
        let shift = I64Vec3::new(3_000_000, -2_000_000, 1_000_000);
        let near_world = |p: BoxelPos| floor(p) || p.0.x >= 3;
        let far_world = |p: BoxelPos| near_world(BoxelPos(p.0 - shift));
        let start = at(0.5, 0.5, 2.0);
        let far_start = WorldPos::new(start.cell() + shift, start.offset());
        let mut near = Player::new(def(), start);
        let mut far = Player::new(def(), far_start);

        let inputs = [
            PlayerInput { movement: Vec2::new(0.3, 1.0), yaw: 0.4, ..Default::default() },
            PlayerInput { movement: Vec2::Y, yaw: 0.0, jump: true, ..Default::default() },
            PlayerInput { movement: Vec2::new(-1.0, 0.2), yaw: 2.0, ..Default::default() },
        ];
        for input in inputs.iter().cycle().take(600) {
            near.step(input, DT, near_world);
            far.step(input, DT, far_world);
            assert_eq!(near.pos().offset(), far.pos().offset());
            assert_eq!(near.pos().cell() + shift, far.pos().cell());
        }
    }
}
