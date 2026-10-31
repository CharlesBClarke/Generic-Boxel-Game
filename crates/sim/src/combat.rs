//! Hitting things. Targets are bodies with health and no will of their own: they fall, slide
//! and bounce off the boxes of solid blocks, and only move when something hits them. One
//! knocked into a wall fast enough is slammed and takes damage. A held tool swings in stages, a
//! wind-up, the strike, then a recovery, and the strike hits the nearest hit box along the line
//! of sight that no block hides.
//!
//! Like the player, each target does its collision math relative to its own cell, so it behaves
//! the same at any distance from zero.

use api::{TargetDef, ToolDef, Vec3, WorldPos};

use crate::player::{Terrain, clamp_axis, sweep_box};
use crate::raycast::ray_box;
use crate::CellBox;

/// How far below a target still counts as resting on something.
const GROUND_PROBE: f32 = 0.01;
/// Slower hits than this, in meters per second, stop a target instead of bouncing it, so it
/// settles instead of buzzing on the floor.
const MIN_BOUNCE_SPEED: f32 = 1.5;
/// A target this far below its spot fell out of the world, and goes back.
const LOST_DEPTH: f32 = 200.0;

/// A runtime target definition ID, in registration order. Names are the identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TargetKind(pub(crate) u16);

impl TargetKind {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A runtime tool ID, in registration order. Names are the identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToolId(pub(crate) u16);

impl ToolId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// One target in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// Unique among the targets of one simulation and never reused, so a renderer can follow
    /// it. A target that breaks comes back under a new ID.
    pub id: u64,
    pub kind: TargetKind,
    /// The middle of its feet.
    pub pos: WorldPos,
    pub prev_pos: WorldPos,
    pub velocity: Vec3,
    /// Facing, radians counterclockwise from +X around +Z. Only for drawing.
    pub yaw: f32,
    pub health: f32,
    pub on_ground: bool,
}

impl Target {
    /// Its collision box relative to its feet.
    fn bounds(def: &TargetDef) -> (Vec3, Vec3) {
        let half = def.width * 0.5;
        (Vec3::new(-half, -half, 0.0), Vec3::new(half, half, def.height))
    }

    /// Whether its box overlaps `boxes` in the boxel at `pos`.
    pub fn overlaps_boxes(&self, def: &TargetDef, pos: api::BoxelPos, boxes: &[CellBox]) -> bool {
        let (min, max) = Self::bounds(def);
        let rel = WorldPos::at_boxel(pos).delta_to(self.pos);
        let (min, max) = (min + rel, max + rel);
        boxes.iter().any(|b| (min.cmplt(b.max) & max.cmpgt(b.min)).all())
    }
}

/// Where a target lives: it starts there and comes back there after breaking.
#[derive(Clone, Debug)]
struct Spot {
    kind: TargetKind,
    pos: WorldPos,
    yaw: f32,
    /// Seconds until it's back, while broken.
    respawn_in: Option<f32>,
}

/// What a strike did to the target it hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetHit {
    pub target: u64,
    pub kind: TargetKind,
    /// Which of its definition's hit boxes was hit, or `None` for the collision box of a target
    /// without any.
    pub zone: Option<usize>,
    /// Where the line of sight met the box, widened by the tool's radius for a near miss.
    pub point: WorldPos,
    /// The change in the target's velocity.
    pub push: Vec3,
    pub damage: f32,
    /// The player's speed toward the target that went into the hit, meters per second.
    pub carried: f32,
    /// The hit used up its health. It's gone from the world, and was last at `pos`.
    pub broke: bool,
    /// The middle of its feet when hit.
    pub pos: WorldPos,
}

/// A target knocked into a wall hard enough to hurt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slam {
    pub target: u64,
    pub kind: TargetKind,
    /// The middle of its feet at the impact.
    pub pos: WorldPos,
    /// Unit direction it was moving into the wall.
    pub into: Vec3,
    /// Its speed into the wall, meters per second.
    pub speed: f32,
    pub damage: f32,
    /// The slam used up its health: it's gone from the world.
    pub broke: bool,
}

/// A tool's strike, hit or miss.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strike {
    /// The tick it happened on, as [`crate::Simulation::tick`] read before that tick's step.
    pub tick: u64,
    pub tool: ToolId,
    /// Where it was aimed from and along.
    pub eye: WorldPos,
    pub look: Vec3,
    /// How far it reached: the tool's reach, or less where a block was in the way.
    pub reach: f32,
    pub hit: Option<TargetHit>,
}

/// Where a swing is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SwingStage {
    /// Before the strike.
    Windup,
    /// After it, until the next swing can start.
    Recover,
}

/// A swing in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swing {
    pub tool: ToolId,
    pub stage: SwingStage,
    /// Seconds into the stage.
    pub time: f32,
}

impl Swing {
    /// How far through its stage it is, from 0 to 1.
    pub fn progress(&self, def: &ToolDef) -> f32 {
        let length = match self.stage {
            SwingStage::Windup => def.windup,
            SwingStage::Recover => def.recover,
        };
        if length <= 0.0 { 1.0 } else { (self.time / length).min(1.0) }
    }
}

#[derive(Debug, Default)]
pub struct Targets {
    list: Vec<Target>,
    spots: Vec<Spot>,
    /// Each live target's spot, alongside `list`.
    homes: Vec<usize>,
    next_id: u64,
    /// Slams since the last [`Targets::take_slams`].
    slams: Vec<Slam>,
}

impl Targets {
    /// Adds a spot and puts a target there.
    pub fn add_spot(&mut self, kind: TargetKind, pos: WorldPos, yaw: f32, def: &TargetDef) {
        self.spots.push(Spot { kind, pos, yaw, respawn_in: None });
        self.spawn(self.spots.len() - 1, def);
    }

    fn spawn(&mut self, spot: usize, def: &TargetDef) {
        let s = &mut self.spots[spot];
        s.respawn_in = None;
        self.list.push(Target {
            id: self.next_id,
            kind: s.kind,
            pos: s.pos,
            prev_pos: s.pos,
            velocity: Vec3::ZERO,
            yaw: s.yaw,
            health: def.health,
            on_ground: false,
        });
        self.homes.push(spot);
        self.next_id += 1;
    }

    pub fn iter(&self) -> impl Iterator<Item = &Target> {
        self.list.iter()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Puts every target back at its spot, whole and at rest, under new IDs.
    pub fn respawn_all(&mut self, defs: &[TargetDef]) {
        self.list.clear();
        self.homes.clear();
        for spot in 0..self.spots.len() {
            let kind = self.spots[spot].kind;
            self.spawn(spot, &defs[kind.index()]);
        }
    }

    /// Slams since the last call, oldest first.
    pub fn take_slams(&mut self) -> Vec<Slam> {
        std::mem::take(&mut self.slams)
    }

    /// Advances every target one tick of `dt` seconds: moves them, slams the ones that hit a
    /// wall fast enough, and brings back broken ones whose time is up.
    pub fn step(&mut self, defs: &[TargetDef], dt: f32, terrain: &impl Terrain) {
        let mut broken = Vec::new();
        for (i, target) in self.list.iter_mut().enumerate() {
            let def = &defs[target.kind.index()];
            let Some(into) = step_target(target, def, dt, terrain) else {
                continue;
            };
            let speed = into.length();
            if speed < def.slam_speed || speed <= 0.0 {
                continue;
            }
            let damage = (speed - def.slam_speed) * def.slam_damage;
            target.health -= damage;
            let broke = target.health <= 0.0;
            self.slams.push(Slam { target: target.id, kind: target.kind, pos: target.pos, into: into / speed, speed, damage, broke });
            if broke {
                broken.push(i);
            }
        }
        for &i in broken.iter().rev() {
            self.break_target(i, defs);
        }
        let lost: Vec<usize> = (0..self.list.len())
            .filter(|&i| self.spots[self.homes[i]].pos.delta_to(self.list[i].pos).z < -LOST_DEPTH)
            .collect();
        for &i in lost.iter().rev() {
            let spot = self.homes[i];
            self.remove(i);
            self.spawn(spot, &defs[self.spots[spot].kind.index()]);
        }
        for spot in 0..self.spots.len() {
            let Some(left) = &mut self.spots[spot].respawn_in else {
                continue;
            };
            *left -= dt;
            if *left <= 0.0 {
                let kind = self.spots[spot].kind;
                self.spawn(spot, &defs[kind.index()]);
            }
        }
    }

    fn remove(&mut self, i: usize) {
        self.list.remove(i);
        self.homes.remove(i);
    }

    /// Takes a target out of the world until its respawn time is up.
    fn break_target(&mut self, i: usize, defs: &[TargetDef]) {
        let spot = self.homes[i];
        let def = &defs[self.list[i].kind.index()];
        self.remove(i);
        self.spots[spot].respawn_in = Some(def.respawn_time);
    }

    /// The nearest hit box along the ray from `eye` along `look` (a unit vector), within
    /// `reach`: the target's index, the box's index, and how far along the ray it was met. A
    /// box the ray goes through beats one it only passes within `radius` of, so aiming at a
    /// small box gets it even with a bigger one just behind.
    fn nearest_along(
        &self,
        defs: &[TargetDef],
        eye: WorldPos,
        look: Vec3,
        reach: f32,
        radius: f32,
    ) -> Option<(usize, Option<usize>, f32)> {
        let mut exact: Option<(usize, Option<usize>, f32)> = None;
        let mut near: Option<(usize, Option<usize>, f32)> = None;
        let closer = |best: &mut Option<(usize, Option<usize>, f32)>, found: (usize, Option<usize>, f32)| {
            if found.2 <= reach && best.is_none_or(|b| found.2 < b.2) {
                *best = Some(found);
            }
        };
        for (i, t) in self.list.iter().enumerate() {
            let def = &defs[t.kind.index()];
            // The ray in the target's own space: from its feet, turned so its front is +Y.
            let from = t.pos.delta_to(eye);
            let turn = t.yaw - std::f32::consts::FRAC_PI_2;
            let (from, dir) = (unturn(from, turn), unturn(look, turn));
            let boxes: Vec<(Option<usize>, Vec3, Vec3)> = if def.hit_boxes.is_empty() {
                let (min, max) = Target::bounds(def);
                vec![(None, min, max)]
            } else {
                def.hit_boxes.iter().enumerate().map(|(z, b)| (Some(z), Vec3::from(b.min), Vec3::from(b.max))).collect()
            };
            for (zone, min, max) in boxes {
                if let Some((d, _)) = ray_box(from, dir, CellBox { min, max }) {
                    closer(&mut exact, (i, zone, d));
                }
                let wide = CellBox { min: min - Vec3::splat(radius), max: max + Vec3::splat(radius) };
                if let Some((d, _)) = ray_box(from, dir, wide) {
                    closer(&mut near, (i, zone, d));
                }
            }
        }
        exact.or(near)
    }

    /// Strikes with `tool` from `eye` along `look`, out to `reach`, by a player moving at
    /// `carry`. Hits the nearest hit box in range: the target takes the tool's damage times the
    /// box's, plus damage for the player's speed toward it, and is knocked away level along the
    /// look, and up, by the tool's knockback and lift over its weight, plus a share of the
    /// player's speed. A target out of health breaks, and comes back at its spot after its
    /// respawn time.
    pub fn strike(
        &mut self,
        defs: &[TargetDef],
        tool: &ToolDef,
        eye: WorldPos,
        look: Vec3,
        reach: f32,
        carry: Vec3,
    ) -> Option<TargetHit> {
        let (i, zone, distance) = self.nearest_along(defs, eye, look, reach, tool.radius)?;
        let target = &mut self.list[i];
        let def = &defs[target.kind.index()];
        let carried = carry.dot(look).max(0.0);
        // Level along the look; straight up or down, along the facing that gave it.
        let level = Vec3::new(look.x, look.y, 0.0).normalize_or(Vec3::X);
        let push = (level * tool.knockback + Vec3::Z * tool.lift) / def.weight + look * carried * tool.momentum;
        target.velocity.x += push.x;
        target.velocity.y += push.y;
        // Lift starts from rest, not from a fall, so a target can be juggled.
        target.velocity.z = target.velocity.z.max(0.0) + push.z.max(0.0);
        target.on_ground = false;
        let zone_damage = zone.map_or(1.0, |z| def.hit_boxes[z].damage);
        let damage = tool.damage * zone_damage + carried * tool.momentum_damage;
        target.health -= damage;
        let hit = TargetHit {
            target: target.id,
            kind: target.kind,
            zone,
            point: eye.translate(look * distance),
            push,
            damage,
            carried,
            broke: target.health <= 0.0,
            pos: target.pos,
        };
        if hit.broke {
            self.break_target(i, defs);
        }
        Some(hit)
    }
}

/// `v` turned by `-angle` about +Z.
fn unturn(v: Vec3, angle: f32) -> Vec3 {
    let (sin, cos) = angle.sin_cos();
    Vec3::new(cos * v.x + sin * v.y, -sin * v.x + cos * v.y, v.z)
}

/// Moves a target one tick. Returns its velocity into a wall it hit sideways this tick, if any.
fn step_target(target: &mut Target, def: &TargetDef, dt: f32, terrain: &impl Terrain) -> Option<Vec3> {
    target.prev_pos = target.pos;
    target.velocity.z -= def.gravity * dt;
    if !target.on_ground {
        let keep = (-def.air_drag * dt).exp();
        target.velocity.x *= keep;
        target.velocity.y *= keep;
    }
    let (min, max) = Target::bounds(def);
    let offset = target.pos.offset();
    let (min, max) = (min + offset, max + offset);
    let cell = target.pos.cell();
    if target.on_ground {
        let below = target.pos.translate(Vec3::Z * -GROUND_PROBE).boxel();
        let keep = (-def.drag * terrain.friction(below) * dt).exp();
        target.velocity.x *= keep;
        target.velocity.y *= keep;
    }
    let delta = target.velocity * dt;
    let moved = sweep_box(min, max, delta, cell, terrain);
    let mut into = Vec3::ZERO;
    for axis in 0..3 {
        if moved[axis] != delta[axis] {
            if axis != 2 {
                into[axis] = target.velocity[axis];
            }
            let speed = target.velocity[axis].abs();
            target.velocity[axis] = if speed >= MIN_BOUNCE_SPEED { -target.velocity[axis] * def.bounce } else { 0.0 };
        }
    }
    target.pos = target.pos.translate(moved);
    let (min, max) = (min + moved, max + moved);
    target.on_ground = target.velocity.z <= 0.0 && clamp_axis(2, -GROUND_PROBE, min, max, cell, terrain) > -GROUND_PROBE;
    (into != Vec3::ZERO).then_some(into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{BoxelPos, I64Vec3};

    const DT: f32 = 1.0 / 60.0;

    fn def() -> TargetDef {
        TargetDef {
            name: "test:dummy".into(),
            model: "test:dummy".into(),
            hit_sound: None,
            width: 0.9,
            height: 1.3,
            hit_boxes: Vec::new(),
            health: 20.0,
            weight: 2.0,
            gravity: 25.0,
            drag: 4.0,
            air_drag: 0.0,
            bounce: 0.2,
            respawn_time: 1.0,
            // Off unless a test turns it on.
            slam_speed: 1000.0,
            slam_damage: 1.0,
        }
    }

    fn tool() -> ToolDef {
        ToolDef {
            name: "test:club".into(),
            reach: 2.5,
            radius: 0.3,
            damage: 5.0,
            knockback: 10.0,
            lift: 4.0,
            momentum: 0.5,
            momentum_damage: 1.0,
            windup: 0.1,
            recover: 0.3,
        }
    }

    /// A floor filling every boxel below z = 0, a wall filling x >= `wall_x`, and ice where
    /// y >= `ice_y`.
    struct Room {
        wall_x: i64,
        ice_y: i64,
    }

    impl Terrain for Room {
        fn boxes(&self, pos: BoxelPos, out: &mut Vec<CellBox>) {
            if pos.0.z < 0 || pos.0.x >= self.wall_x {
                out.push(CellBox::FULL);
            }
        }

        fn friction(&self, pos: BoxelPos) -> f32 {
            if pos.0.y >= self.ice_y { 0.02 } else { 1.0 }
        }
    }

    fn room() -> Room {
        Room { wall_x: 1000, ice_y: 1000 }
    }

    fn at(x: f32, y: f32, z: f32) -> WorldPos {
        WorldPos::new(I64Vec3::ZERO, Vec3::new(x, y, z))
    }

    fn one(pos: WorldPos) -> (Targets, Vec<TargetDef>) {
        let mut targets = Targets::default();
        targets.add_spot(TargetKind(0), pos, 0.0, &def());
        (targets, vec![def()])
    }

    fn run(targets: &mut Targets, defs: &[TargetDef], terrain: &impl Terrain, seconds: f32) {
        for _ in 0..(seconds / DT) as usize {
            targets.step(defs, DT, terrain);
        }
    }

    /// Strikes east from 1.5 m west of the target, at chest height.
    fn strike_east(targets: &mut Targets, defs: &[TargetDef]) -> Option<TargetHit> {
        let t = targets.iter().next().unwrap().pos;
        targets.strike(defs, &tool(), t.translate(Vec3::new(-1.5, 0.0, 1.0)), Vec3::X, tool().reach, Vec3::ZERO)
    }

    #[test]
    fn a_target_falls_and_comes_to_rest_on_the_floor() {
        let (mut targets, defs) = one(at(0.5, 0.5, 3.0));
        run(&mut targets, &defs, &room(), 2.0);
        let t = targets.iter().next().unwrap();
        assert!(t.on_ground);
        assert!(t.pos.offset().z.abs() < 1e-3, "feet on the floor: {}", t.pos.offset().z);
        assert_eq!(t.velocity, Vec3::ZERO);
    }

    #[test]
    fn a_strike_in_reach_hits_and_one_out_of_reach_misses() {
        let (mut targets, defs) = one(at(0.5, 0.5, 0.0));
        let hit = strike_east(&mut targets, &defs).expect("in reach");
        assert_eq!(hit.damage, 5.0);
        assert!(!hit.broke);
        assert_eq!(targets.iter().next().unwrap().health, 15.0);
        let far = at(0.5 - 4.0, 0.5, 1.0);
        assert_eq!(targets.strike(&defs, &tool(), far, Vec3::X, tool().reach, Vec3::ZERO), None);
        let wide = at(-1.0, 0.5 + 0.45 + 0.35, 1.0);
        assert_eq!(targets.strike(&defs, &tool(), wide, Vec3::X, tool().reach, Vec3::ZERO), None, "past the radius");
        let near_miss = at(-1.0, 0.5 + 0.45 + 0.25, 1.0);
        assert!(targets.strike(&defs, &tool(), near_miss, Vec3::X, tool().reach, Vec3::ZERO).is_some(), "within the radius");
    }

    #[test]
    fn a_strike_hits_the_nearest_target() {
        let mut targets = Targets::default();
        targets.add_spot(TargetKind(0), at(2.5, 0.5, 0.0), 0.0, &def());
        targets.add_spot(TargetKind(0), at(1.0, 0.5, 0.0), 0.0, &def());
        let hit = targets.strike(&[def()], &tool(), at(0.0, 0.5, 1.0), Vec3::X, 5.0, Vec3::ZERO).unwrap();
        assert_eq!(hit.target, 1);
    }

    #[test]
    fn knockback_is_level_along_the_look_and_divided_by_weight() {
        let (mut targets, defs) = one(at(0.5, 0.5, 0.0));
        run(&mut targets, &defs, &room(), 0.5);
        // Looking down at it still knocks it level, away.
        let look = Vec3::new(1.0, 0.0, -0.5).normalize();
        let hit = targets.strike(&defs, &tool(), at(-0.7, 0.5, 1.8), look, 3.0, Vec3::ZERO).unwrap();
        assert!((hit.push - Vec3::new(5.0, 0.0, 2.0)).length() < 1e-5, "{}", hit.push);

        let heavy = vec![TargetDef { weight: 4.0, ..def() }];
        let (mut targets, _) = one(at(0.5, 0.5, 0.0));
        let hit = strike_east(&mut targets, &heavy).unwrap();
        assert!((hit.push - Vec3::new(2.5, 0.0, 1.0)).length() < 1e-5, "twice the weight, half the push");
    }

    #[test]
    fn a_hit_target_flies_slides_and_settles() {
        let (mut targets, defs) = one(at(0.5, 0.5, 0.0));
        run(&mut targets, &defs, &room(), 0.5);
        strike_east(&mut targets, &defs);
        run(&mut targets, &defs, &room(), 0.1);
        assert!(!targets.iter().next().unwrap().on_ground, "lifted off the floor");
        run(&mut targets, &defs, &room(), 3.0);
        let t = targets.iter().next().unwrap();
        assert!(t.on_ground && t.velocity.length() < 1e-3, "settled: {}", t.velocity);
        let moved = at(0.5, 0.5, 0.0).delta_to(t.pos);
        assert!(moved.x > 0.5 && moved.y.abs() < 1e-4 && moved.z.abs() < 1e-3, "knocked east: {moved}");
    }

    #[test]
    fn ice_lets_it_slide_further() {
        let distance = |terrain: &Room| {
            let (mut targets, defs) = one(at(0.5, 0.5, 0.0));
            run(&mut targets, &defs, terrain, 0.5);
            strike_east(&mut targets, &defs);
            run(&mut targets, &defs, terrain, 2.0);
            at(0.5, 0.5, 0.0).delta_to(targets.iter().next().unwrap().pos).x
        };
        let on_ice = distance(&Room { wall_x: 1000, ice_y: -1000 });
        let on_grass = distance(&room());
        assert!(on_ice > on_grass * 2.0, "ice {on_ice}, grass {on_grass}");
    }

    #[test]
    fn a_wall_stops_it_and_it_never_ends_up_inside() {
        let terrain = Room { wall_x: 2, ice_y: 1000 };
        let (mut targets, defs) = one(at(0.5, 0.5, 0.0));
        run(&mut targets, &defs, &terrain, 0.5);
        // Three hits of 5 leave it whole.
        for _ in 0..3 {
            strike_east(&mut targets, &defs);
            run(&mut targets, &defs, &terrain, 0.3);
            let east_side = at(0.0, 0.0, 0.0).delta_to(targets.iter().next().unwrap().pos).x + 0.45;
            assert!(east_side <= 2.0 + 1e-3, "inside the wall by {}", east_side - 2.0);
        }
    }

    #[test]
    fn it_breaks_when_out_of_health_and_comes_back_at_its_spot() {
        let spot = at(0.5, 0.5, 0.0);
        let (mut targets, defs) = one(spot);
        let first = targets.iter().next().unwrap().id;
        let mut broke = None;
        for _ in 0..4 {
            let hit = strike_east(&mut targets, &defs).unwrap();
            run(&mut targets, &defs, &room(), 0.2);
            if hit.broke {
                broke = Some(hit);
                break;
            }
        }
        assert!(broke.is_some_and(|h| h.target == first), "four hits of 5 break 20 health");
        assert!(targets.is_empty(), "gone once broken");
        run(&mut targets, &defs, &room(), 0.5);
        assert!(targets.is_empty(), "not back before its respawn time");
        run(&mut targets, &defs, &room(), 0.6);
        let back = targets.iter().next().expect("back after its respawn time");
        assert_ne!(back.id, first, "under a new ID");
        assert_eq!(back.health, def().health);
        assert!(spot.delta_to(back.pos).length() < 0.1);
    }

    #[test]
    fn respawning_all_puts_every_target_back_whole() {
        let (mut targets, defs) = one(at(0.5, 0.5, 0.0));
        strike_east(&mut targets, &defs);
        run(&mut targets, &defs, &room(), 0.5);
        targets.respawn_all(&defs);
        let t = targets.iter().next().unwrap();
        assert_eq!((t.health, t.pos, t.velocity), (def().health, at(0.5, 0.5, 0.0), Vec3::ZERO));
    }

    /// A dummy facing north (+Y, its model's front), with a small head box out front.
    fn headed() -> TargetDef {
        TargetDef {
            hit_boxes: vec![
                api::HitBox { name: "body".into(), min: [-0.45, -0.45, 0.0], max: [0.45, 0.45, 1.3], damage: 1.0 },
                api::HitBox { name: "head".into(), min: [-0.2, 0.45, 1.0], max: [0.2, 0.85, 1.4], damage: 2.0 },
            ],
            ..def()
        }
    }

    fn one_facing(pos: WorldPos, yaw: f32, def: TargetDef) -> (Targets, Vec<TargetDef>) {
        let mut targets = Targets::default();
        targets.add_spot(TargetKind(0), pos, yaw, &def);
        (targets, vec![def])
    }

    #[test]
    fn the_head_box_turns_with_the_facing_and_hits_harder() {
        // Facing north, the head is north of the body: a strike from the west at head height
        // just north of the body hits it.
        let north = std::f32::consts::FRAC_PI_2;
        let (mut targets, defs) = one_facing(at(0.5, 0.5, 0.0), north, headed());
        let eye = at(-1.5, 0.5 + 0.65, 1.2);
        let hit = targets.strike(&defs, &tool(), eye, Vec3::X, 3.0, Vec3::ZERO).expect("the head");
        assert_eq!(hit.zone, Some(1));
        assert_eq!(hit.damage, 10.0, "double damage");
        // Turned to face east, the same strike passes north of it, within the radius of the
        // body only.
        let (mut targets, defs) = one_facing(at(0.5, 0.5, 0.0), 0.0, headed());
        let hit = targets.strike(&defs, &tool(), eye, Vec3::X, 3.0, Vec3::ZERO);
        assert_eq!(hit.map(|h| h.zone), Some(Some(0)), "a near miss on the body");
        // Facing east, the head is east: a strike from the north at head height hits it.
        let (mut targets, defs) = one_facing(at(0.5, 0.5, 0.0), 0.0, headed());
        let hit = targets.strike(&defs, &tool(), at(0.5 + 0.65, 2.5, 1.2), -Vec3::Y, 3.0, Vec3::ZERO).unwrap();
        assert_eq!(hit.zone, Some(1));
    }

    #[test]
    fn aiming_through_a_box_beats_a_near_miss_on_a_closer_one() {
        // Facing west, toward the striker: the head sticks out toward the eye, and the strike
        // aims just over it at the body behind. The head is closer within the radius, but the
        // strike goes through the body.
        let west = std::f32::consts::PI;
        let (mut targets, defs) = one_facing(at(0.5, 0.5, 0.0), west, headed());
        let hit = targets.strike(&defs, &tool(), at(-1.5, 0.5, 0.6), Vec3::X, 3.0, Vec3::ZERO).unwrap();
        assert_eq!(hit.zone, Some(0), "low, under the head, into the body");
        let hit = targets.strike(&defs, &tool(), at(-1.5, 0.5, 1.2), Vec3::X, 3.0, Vec3::ZERO).unwrap();
        assert_eq!(hit.zone, Some(1), "level with the head");
    }

    #[test]
    fn running_into_a_swing_hits_harder_and_further() {
        let still = {
            let (mut targets, defs) = one(at(0.5, 0.5, 0.0));
            strike_east(&mut targets, &defs).unwrap()
        };
        let (mut targets, defs) = one(at(0.5, 0.5, 0.0));
        let t = targets.iter().next().unwrap().pos;
        let running = Vec3::new(8.0, 0.0, 0.0);
        let hit = targets.strike(&defs, &tool(), t.translate(Vec3::new(-1.5, 0.0, 1.0)), Vec3::X, 3.0, running).unwrap();
        assert_eq!(hit.carried, 8.0);
        assert_eq!(hit.damage, still.damage + 8.0, "a point per meter per second");
        assert!((hit.push.x - still.push.x - 4.0).abs() < 1e-5, "half the speed passed on: {}", hit.push.x);
        // Running away from it passes nothing on.
        let (mut targets, defs) = one(at(0.5, 0.5, 0.0));
        let back = targets.strike(&defs, &tool(), t.translate(Vec3::new(-1.5, 0.0, 1.0)), Vec3::X, 3.0, -running).unwrap();
        assert_eq!((back.carried, back.damage), (0.0, still.damage));
    }

    #[test]
    fn air_drag_cuts_a_flight_short() {
        let flight = |air_drag: f32| {
            let defs = vec![TargetDef { air_drag, gravity: 0.0, ..def() }];
            let mut targets = Targets::default();
            targets.add_spot(TargetKind(0), at(0.5, 0.5, 5.0), 0.0, &defs[0]);
            strike_east(&mut targets, &defs);
            run(&mut targets, &defs, &room(), 2.0);
            at(0.5, 0.5, 5.0).delta_to(targets.iter().next().unwrap().pos).x
        };
        let (floaty, punchy) = (flight(0.0), flight(4.0));
        assert!(punchy < floaty * 0.25, "{punchy} against {floaty}");
        assert!((punchy - 5.0 / 4.0).abs() < 0.05, "speed over drag: {punchy}");
    }

    #[test]
    fn a_fast_hit_into_a_wall_slams_and_a_slow_one_does_not() {
        let terrain = Room { wall_x: 2, ice_y: 1000 };
        let slam = |knockback: f32| {
            let defs = vec![TargetDef { slam_speed: 4.0, slam_damage: 2.0, ..def() }];
            let (mut targets, _) = one(at(0.5, 0.5, 0.0));
            run(&mut targets, &defs, &terrain, 0.5);
            let t = targets.iter().next().unwrap().pos;
            let club = ToolDef { knockback, lift: 0.0, ..tool() };
            targets.strike(&defs, &club, t.translate(Vec3::new(-1.5, 0.0, 1.0)), Vec3::X, 3.0, Vec3::ZERO);
            run(&mut targets, &defs, &terrain, 0.5);
            (targets.take_slams(), targets.iter().next().map(|t| t.health))
        };
        let (slams, _) = slam(4.0);
        assert!(slams.is_empty(), "2 m/s into the wall is a bump");
        let (slams, health) = slam(20.0);
        let [s] = slams[..] else { panic!("one slam, got {slams:?}") };
        assert!(s.speed > 5.0 && s.into.x > 0.99, "{s:?}");
        assert!((s.damage - (s.speed - 4.0) * 2.0).abs() < 1e-4);
        assert_eq!(health, Some(def().health - tool().damage - s.damage));
    }

    #[test]
    fn a_slam_can_break_a_target() {
        let terrain = Room { wall_x: 2, ice_y: 1000 };
        let defs = vec![TargetDef { slam_speed: 1.0, slam_damage: 10.0, ..def() }];
        let (mut targets, _) = one(at(0.5, 0.5, 0.0));
        run(&mut targets, &defs, &terrain, 0.5);
        strike_east(&mut targets, &defs);
        run(&mut targets, &defs, &terrain, 0.5);
        assert!(targets.take_slams().iter().any(|s| s.broke));
        assert!(targets.is_empty());
    }

    #[test]
    fn swing_progress_runs_through_each_stage() {
        let s = Swing { tool: ToolId(0), stage: SwingStage::Windup, time: 0.05 };
        assert!((s.progress(&tool()) - 0.5).abs() < 1e-5);
        let s = Swing { stage: SwingStage::Recover, time: 1.0, ..s };
        assert_eq!(s.progress(&tool()), 1.0);
        assert_eq!(s.progress(&ToolDef { recover: 0.0, ..tool() }), 1.0);
    }
}
