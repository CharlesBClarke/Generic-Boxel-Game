//! Hitting things. Targets are bodies with health: they fall, slide and bounce off the boxes of
//! solid blocks. One knocked into a wall fast enough is slammed and takes damage. A target with
//! a brain also walks where its brain wants, stepping up ledges like the player and kept from
//! walking off edges if its brain asks; one without only moves when something hits it. A held tool swings in stages, a
//! wind-up, the strike, then a recovery, and the strike hits the nearest hit box along the line
//! of sight that no block hides.
//!
//! With a crowd definition, targets see the others near them, and overlapping bodies, the
//! player's included, are pushed gently apart.
//!
//! Like the player, each target does its collision math relative to its own cell, so it behaves
//! the same at any distance from zero.

use api::{Attack, Brain, BrainContext, CrowdDef, Hurt, MAX_NEIGHBORS, Neighbor, Seen, TargetDef, ToolDef, Vec2, Vec3, WorldPos};

use crate::player::{Terrain, clamp_axis, guard_edges, sweep_stepping};
use crate::raycast::ray_box;
use crate::{CellBox, Player};

/// How far below a target still counts as resting on something.
const GROUND_PROBE: f32 = 0.01;
/// Slower hits than this, in meters per second, stop a target instead of bouncing it, so it
/// settles instead of buzzing on the floor.
const MIN_BOUNCE_SPEED: f32 = 1.5;
/// A target this far below its spot fell out of the world, and goes back.
const LOST_DEPTH: f32 = 200.0;
/// Bodies overlapping by less than this, in meters, aren't pushed, so ones just touching rest.
const PUSH_SLACK: f32 = 0.01;

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
    /// Facing, radians counterclockwise from +X around +Z. Hit boxes turn with it.
    pub yaw: f32,
    pub prev_yaw: f32,
    pub health: f32,
    pub on_ground: bool,
    /// The clip its brain wants its model to play, and how fast; empty without a brain.
    pub clip: String,
    pub clip_speed: f32,
    /// Its brain's memory, kept between ticks.
    pub memory: Vec<f32>,
    /// What its brain last asked for: a level velocity, and how far it may walk off an edge.
    pub walk: Vec2,
    pub edge_guard: Option<f32>,
    /// Its level movement was stopped last tick.
    pub blocked: bool,
    /// What its brain shows other targets.
    pub signal: String,
    /// The level velocity it's being pushed out of other bodies at this tick, on top of its own.
    shove: Vec2,
    /// What hurt it since its brain last thought, if anything.
    hurt: Option<Hurt>,
    /// The attack its brain has under way, if any.
    pub attack: Option<Attack>,
    /// The attack under way has already hit, so it doesn't hit again.
    spent: bool,
    /// Its attack hit since its brain last thought.
    landed: bool,
}

impl Target {
    /// Its collision box relative to its feet.
    fn bounds(def: &TargetDef) -> (Vec3, Vec3) {
        let half = def.width * 0.5;
        (Vec3::new(-half, -half, 0.0), Vec3::new(half, half, def.height))
    }

    /// Its level footprint for pushing, in its own space with the front along +Y: a capsule
    /// from `(x, back)` to `(x, front)` of the given radius. Over its hit boxes, since a long
    /// creature's collision box covers only its middle; without any, its collision box.
    fn footprint(def: &TargetDef) -> Footprint {
        if def.hit_boxes.is_empty() {
            return Footprint { x: 0.0, back: 0.0, front: 0.0, radius: def.width * 0.5 };
        }
        let (mut min, mut max) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for b in &def.hit_boxes {
            min = min.min(Vec2::new(b.min[0], b.min[1]));
            max = max.max(Vec2::new(b.max[0], b.max[1]));
        }
        let radius = (max.x - min.x) * 0.5;
        let (back, front) = (min.y + radius, max.y - radius);
        let (back, front) = if back <= front { (back, front) } else { ((min.y + max.y) * 0.5, (min.y + max.y) * 0.5) };
        Footprint { x: (min.x + max.x) * 0.5, back, front, radius }
    }

    /// Whether its box overlaps `boxes` in the boxel at `pos`.
    pub fn overlaps_boxes(&self, def: &TargetDef, pos: api::BoxelPos, boxes: &[CellBox]) -> bool {
        let (min, max) = Self::bounds(def);
        let rel = WorldPos::at_boxel(pos).delta_to(self.pos);
        let (min, max) = (min + rel, max + rel);
        boxes.iter().any(|b| (min.cmplt(b.max) & max.cmpgt(b.min)).all())
    }
}

/// A level capsule; see [`Target::footprint`].
#[derive(Clone, Copy, Debug)]
struct Footprint {
    x: f32,
    back: f32,
    front: f32,
    radius: f32,
}

/// A body taking part in pushing, relative to the first target's feet: a level segment with a
/// radius, its height range, and its weight.
struct Body {
    a: Vec2,
    b: Vec2,
    radius: f32,
    bottom: f32,
    top: f32,
    weight: f32,
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
    /// Attacks that hit the player since the last [`Targets::take_blows`].
    blows: Vec<Blow>,
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
            prev_yaw: s.yaw,
            health: def.health,
            on_ground: false,
            clip: String::new(),
            clip_speed: 1.0,
            memory: Vec::new(),
            walk: Vec2::ZERO,
            edge_guard: None,
            blocked: false,
            signal: String::new(),
            shove: Vec2::ZERO,
            hurt: None,
            attack: None,
            spent: false,
            landed: false,
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

    /// Attacks that hit the player since the last call, oldest first.
    pub fn take_blows(&mut self) -> Vec<Blow> {
        std::mem::take(&mut self.blows)
    }

    /// Slams since the last call, oldest first.
    pub fn take_slams(&mut self) -> Vec<Slam> {
        std::mem::take(&mut self.slams)
    }

    /// Advances every target one tick of `dt` seconds: lets each one's brain think, moves them,
    /// slams the ones that hit a wall fast enough, and brings back broken ones whose time is up.
    /// `brains` holds each target kind's brain, by index; kinds past its end have none.
    /// `player` is the player's feet and velocity, if there is a player, and `tick` the tick
    /// being run, for the brains' random numbers.
    pub fn step(&mut self, ctx: StepContext<'_>, terrain: &impl Terrain) {
        let StepContext { defs, brains, crowd, player, tick, dt } = ctx;
        let seen = player.as_ref().map(|p| (p.pos(), p.velocity()));
        let body = player.as_ref().map(|p| (p.pos(), p.shape().width * 0.5, p.height()));
        let mut others = crowd.map_or_else(Vec::new, |c| self.neighbors(defs, c.sight));
        for (i, target) in self.list.iter_mut().enumerate() {
            let def = &defs[target.kind.index()];
            target.prev_yaw = target.yaw;
            match brains.get(target.kind.index()).copied().flatten() {
                Some(brain) => {
                    let others = others.get_mut(i).map(std::mem::take).unwrap_or_default();
                    think(target, def, brain, seen, others, tick, dt);
                }
                None => {
                    target.walk = Vec2::ZERO;
                    target.edge_guard = None;
                    target.clip.clear();
                    target.clip_speed = 1.0;
                    target.signal.clear();
                    target.hurt = None;
                    target.attack = None;
                }
            }
        }
        for target in &mut self.list {
            target.shove = Vec2::ZERO;
        }
        if let Some(crowd) = crowd {
            self.push_apart(defs, crowd, player, dt);
        }
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
            hurt(target, -into / speed, damage);
            let broke = target.health <= 0.0;
            self.slams.push(Slam { target: target.id, kind: target.kind, pos: target.pos, into: into / speed, speed, damage, broke });
            if broke {
                broken.push(i);
            }
        }
        if let Some(body) = body {
            self.attack(body);
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

    /// Tests each live attack against the player's body: its feet, level radius and height. One
    /// that touches it, and hasn't hit yet, is a blow.
    fn attack(&mut self, (feet, radius, height): (WorldPos, f32, f32)) {
        for target in &mut self.list {
            let Some(attack) = &target.attack else {
                target.spent = false;
                continue;
            };
            if target.spent {
                continue;
            }
            let turn = target.yaw - std::f32::consts::FRAC_PI_2;
            let at = unturn(target.pos.delta_to(feet), turn);
            let (min, max) = (Vec3::from(attack.min), Vec3::from(attack.max));
            let near = Vec2::new(at.x.clamp(min.x, max.x), at.y.clamp(min.y, max.y));
            let touches = near.distance(at.truncate()) <= radius && at.z < max.z && at.z + height > min.z;
            if !touches {
                continue;
            }
            target.spent = true;
            target.landed = true;
            let level = target.pos.delta_to(feet).truncate();
            let (sin, cos) = target.yaw.sin_cos();
            let away = level.try_normalize().unwrap_or(Vec2::new(cos, sin)).extend(0.0);
            self.blows.push(Blow { target: target.id, kind: target.kind, pos: target.pos, away, attack: attack.clone() });
        }
    }

    /// Each target's view of the others within `sight` meters, nearest first, at most
    /// [`MAX_NEIGHBORS`], alongside the list.
    fn neighbors(&self, defs: &[TargetDef], sight: f32) -> Vec<Vec<Neighbor>> {
        let list = &self.list;
        (0..list.len())
            .map(|i| {
                let mut near: Vec<(f32, usize, Vec3)> = (0..list.len())
                    .filter(|&j| j != i)
                    .map(|j| {
                        let offset = list[i].pos.delta_to(list[j].pos);
                        (offset.length(), j, offset)
                    })
                    .filter(|&(d, ..)| d <= sight)
                    .collect();
                near.sort_by(|a, b| a.0.total_cmp(&b.0).then(list[a.1].id.cmp(&list[b.1].id)));
                near.truncate(MAX_NEIGHBORS);
                near.into_iter()
                    .map(|(_, j, offset)| Neighbor {
                        offset,
                        velocity: list[j].velocity,
                        kind: defs[list[j].kind.index()].name.clone(),
                        id: list[j].id,
                        signal: list[j].signal.clone(),
                    })
                    .collect()
            })
            .collect()
    }

    /// Works out how fast overlapping bodies are pushed apart this tick: each target's `shove`,
    /// and the player's share as a shove it takes on its next step.
    fn push_apart(&mut self, defs: &[TargetDef], crowd: &CrowdDef, player: Option<&mut Player>, dt: f32) {
        let Some(origin) = self.list.first().map(|t| t.pos) else {
            return;
        };
        let mut bodies: Vec<Body> = self
            .list
            .iter()
            .map(|t| {
                let def = &defs[t.kind.index()];
                let f = Target::footprint(def);
                let at = origin.delta_to(t.pos);
                // Its own space has the front along +Y; the facing is from +X.
                let turn = t.yaw - std::f32::consts::FRAC_PI_2;
                let place = |y: f32| at.truncate() + Vec2::from_angle(turn).rotate(Vec2::new(f.x, y));
                Body { a: place(f.back), b: place(f.front), radius: f.radius, bottom: at.z, top: at.z + def.height, weight: def.weight }
            })
            .collect();
        let players = bodies.len();
        if let Some(p) = &player {
            let at = origin.delta_to(p.pos());
            bodies.push(Body {
                a: at.truncate(),
                b: at.truncate(),
                radius: p.shape().width * 0.5,
                bottom: at.z,
                top: at.z + p.height(),
                weight: crowd.player_weight,
            });
        }
        let mut shoves = vec![Vec2::ZERO; bodies.len()];
        for i in 0..bodies.len() {
            for j in i + 1..bodies.len() {
                let (p, q) = (&bodies[i], &bodies[j]);
                if p.bottom >= q.top || q.bottom >= p.top {
                    continue;
                }
                let (on_p, on_q) = closest_points(p.a, p.b, q.a, q.b);
                let between = on_q - on_p;
                let depth = p.radius + q.radius - between.length();
                if depth <= PUSH_SLACK {
                    continue;
                }
                // Right on top of each other: apart along the line between their middles, or
                // failing that, along +X.
                let middles = (q.a + q.b - p.a - p.b) * 0.5;
                let dir = between.try_normalize().or(middles.try_normalize()).unwrap_or(Vec2::X);
                let speed = (crowd.push * depth).min(crowd.max_push);
                let total = p.weight + q.weight;
                shoves[i] -= dir * speed * q.weight / total;
                shoves[j] += dir * speed * p.weight / total;
            }
        }
        for (target, shove) in self.list.iter_mut().zip(&shoves) {
            target.shove = shove.clamp_length_max(crowd.max_push);
        }
        if let Some(p) = player {
            p.shove(shoves[players].clamp_length_max(crowd.max_push) * dt);
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
        hurt(target, -level, damage);
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

/// A creature's attack hitting the player.
#[derive(Clone, Debug, PartialEq)]
pub struct Blow {
    pub target: u64,
    pub kind: TargetKind,
    /// Where the attacker's feet were.
    pub pos: WorldPos,
    /// Level unit direction from the attacker toward the player: the way the blow knocks them.
    pub away: Vec3,
    pub attack: Attack,
}

/// What [`Targets::step`] needs besides the terrain.
pub struct StepContext<'a> {
    pub defs: &'a [TargetDef],
    pub brains: &'a [Option<&'a dyn Brain>],
    /// How targets see each other and get pushed apart; `None` for neither.
    pub crowd: Option<&'a CrowdDef>,
    /// The player, if there is one: brains see it, and it's pushed and pushes.
    pub player: Option<&'a mut Player>,
    pub tick: u64,
    pub dt: f32,
}

/// Records something hurting `target`, coming from the level direction `from`. Hurts before its
/// brain thinks again add up, keeping the latest direction.
fn hurt(target: &mut Target, from: Vec3, damage: f32) {
    let from = Vec3::new(from.x, from.y, 0.0).normalize_or_zero();
    let before = target.hurt.as_ref().map_or(0.0, |h| h.damage);
    target.hurt = Some(Hurt { from, damage: before + damage });
}

/// Lets `brain` decide what `target` wants this tick.
fn think(
    target: &mut Target,
    def: &TargetDef,
    brain: &dyn Brain,
    player: Option<(WorldPos, Vec3)>,
    others: Vec<Neighbor>,
    tick: u64,
    dt: f32,
) {
    let ctx = BrainContext {
        dt,
        id: target.id,
        velocity: target.velocity,
        yaw: target.yaw,
        on_ground: target.on_ground,
        blocked: target.blocked,
        health: target.health,
        full_health: def.health,
        player: player.map(|(feet, velocity)| Seen { offset: target.pos.delta_to(feet), velocity }),
        hurt: target.hurt.take(),
        attack_landed: std::mem::take(&mut target.landed),
        others,
        random: tick.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ target.id.rotate_left(32),
        memory: std::mem::take(&mut target.memory),
    };
    let out = brain.think(&ctx);
    target.walk = if out.walk.is_finite() { out.walk } else { Vec2::ZERO };
    if out.yaw.is_finite() {
        target.yaw = out.yaw;
    }
    target.edge_guard = out.edge_guard;
    target.clip = out.clip;
    target.clip_speed = out.clip_speed;
    target.signal = out.signal;
    target.attack = out.attack.filter(|a| {
        a.min.iter().chain(&a.max).chain(&[a.damage, a.push, a.lift, a.guarded_damage, a.guarded_push]).all(|v| v.is_finite())
    });
    target.memory = out.memory;
}

/// The part of the speed `v` along one axis beyond the walking speed `w` the same way.
fn beyond(v: f32, w: f32) -> f32 {
    if v * w <= 0.0 { v } else if v.abs() > w.abs() { v - w } else { 0.0 }
}

/// The closest points between the segments `a0`–`a1` and `b0`–`b1`, one on each.
fn closest_points(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> (Vec2, Vec2) {
    let (d1, d2, r) = (a1 - a0, b1 - b0, a0 - b0);
    let (a, e, f) = (d1.length_squared(), d2.length_squared(), d2.dot(r));
    const TINY: f32 = 1e-8;
    let (s, t) = if a <= TINY && e <= TINY {
        (0.0, 0.0)
    } else if a <= TINY {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = d1.dot(r);
        if e <= TINY {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s = if denom > TINY { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let mut t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
            (s, t)
        }
    };
    (a0 + d1 * s, b0 + d2 * t)
}

/// `v` turned by `-angle` about +Z.
fn unturn(v: Vec3, angle: f32) -> Vec3 {
    let (sin, cos) = angle.sin_cos();
    Vec3::new(cos * v.x + sin * v.y, -sin * v.x + cos * v.y, v.z)
}

/// Moves a target one tick: on the ground it eases toward the velocity its brain wants, steps
/// up ledges and keeps to its edge guard. Returns its velocity into a wall it hit sideways this
/// tick, beyond what it was walking at, if any.
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
    let walk = target.walk.extend(0.0);
    if target.on_ground {
        // Drag pulls toward the walking speed rather than toward a stop.
        let below = target.pos.translate(Vec3::Z * -GROUND_PROBE).boxel();
        let keep = (-def.drag * terrain.friction(below) * dt).exp();
        target.velocity.x = walk.x + (target.velocity.x - walk.x) * keep;
        target.velocity.y = walk.y + (target.velocity.y - walk.y) * keep;
    }
    let delta = target.velocity * dt + (target.shove * dt).extend(0.0);
    let step = if target.on_ground { def.step_height } else { 0.0 };
    let (swept, stepped) = sweep_stepping(min, max, delta, step, cell, terrain);
    let moved = match target.edge_guard {
        Some(drop) if target.on_ground && stepped == 0.0 && target.velocity.z <= 0.0 => {
            guard_edges(min, max, cell, swept, drop, terrain)
        }
        _ => swept,
    };
    let mut into = Vec3::ZERO;
    for axis in 0..3 {
        if swept[axis] != delta[axis] {
            if axis != 2 {
                into[axis] = beyond(target.velocity[axis], walk[axis]);
            }
            let speed = target.velocity[axis].abs();
            target.velocity[axis] = if speed >= MIN_BOUNCE_SPEED { -target.velocity[axis] * def.bounce } else { 0.0 };
        } else if moved[axis] != swept[axis] {
            // Held back from an edge: no bounce, no slam.
            target.velocity[axis] = 0.0;
        }
    }
    target.blocked = (0..2).any(|axis| moved[axis] != delta[axis]);
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
            brain: None,
            width: 0.9,
            height: 1.3,
            step_height: 0.0,
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
            targets.step(StepContext { defs, brains: &[], crowd: None, player: None, tick: 0, dt: DT }, terrain);
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

    /// Walks at `walk` for ever, counting its thinks in memory slot 0 and adding up the damage
    /// it was told about in slot 1, with the last hurt's direction in slot 2 (x) and the
    /// player's offset in slot 3 (x).
    struct Walker {
        walk: Vec2,
        guard: Option<f32>,
    }

    impl api::Tunables for Walker {
        fn tunables(&self) -> Vec<api::Tunable> {
            Vec::new()
        }

        fn set_tunable(&mut self, _: &str, _: f32) -> bool {
            false
        }
    }

    impl Brain for Walker {
        fn think(&self, ctx: &BrainContext) -> api::BrainOutput {
            let mut memory = ctx.memory.clone();
            memory.resize(4, 0.0);
            memory[0] += 1.0;
            if let Some(h) = &ctx.hurt {
                memory[1] += h.damage;
                memory[2] = h.from.x;
            }
            memory[3] = ctx.player.as_ref().map_or(0.0, |p| p.offset.x);
            api::BrainOutput {
                walk: self.walk,
                yaw: 1.0,
                edge_guard: self.guard,
                clip: "walk".into(),
                clip_speed: 2.0,
                signal: format!("seen {}", ctx.others.len()),
                attack: None,
                memory,
            }
        }
    }

    /// Flat floor below z = 0 out to x < `floor_end` (a deep drop beyond), and from x >= `bump_x`
    /// a raised floor `bump` high.
    struct Steps {
        bump_x: i64,
        bump: f32,
        floor_end: i64,
    }

    impl Terrain for Steps {
        fn boxes(&self, pos: BoxelPos, out: &mut Vec<CellBox>) {
            let p = pos.0;
            if p.x >= self.floor_end {
                if p.z < -20 {
                    out.push(CellBox::FULL);
                }
                return;
            }
            if p.z < 0 {
                out.push(CellBox::FULL);
            } else if p.x >= self.bump_x {
                let top = self.bump - p.z as f32;
                if top >= 1.0 {
                    out.push(CellBox::FULL);
                } else if top > 0.0 {
                    out.push(CellBox { min: Vec3::ZERO, max: Vec3::new(1.0, 1.0, top) });
                }
            }
        }

        fn friction(&self, _: BoxelPos) -> f32 {
            1.0
        }
    }

    fn walker_def() -> TargetDef {
        TargetDef { step_height: 0.6, drag: 9.0, ..def() }
    }

    fn think_run(targets: &mut Targets, defs: &[TargetDef], brain: &Walker, terrain: &impl Terrain, seconds: f32) {
        let brains: [Option<&dyn Brain>; 1] = [Some(brain)];
        let mut player = standing_player(at(-3.0, 0.5, 0.0));
        for tick in 0..(seconds / DT).round() as u64 {
            let ctx = StepContext { defs, brains: &brains, crowd: None, player: Some(&mut player), tick, dt: DT };
            targets.step(ctx, terrain);
        }
    }

    fn east(guard: Option<f32>) -> Walker {
        Walker { walk: Vec2::new(1.5, 0.0), guard }
    }

    fn x_of(targets: &Targets) -> f32 {
        at(0.0, 0.0, 0.0).delta_to(targets.iter().next().unwrap().pos).x
    }

    #[test]
    fn a_walking_target_steps_up_a_slab_but_not_a_full_block() {
        let slab = Steps { bump_x: 2, bump: 0.5, floor_end: 1000 };
        let defs = vec![walker_def()];
        let (mut targets, _) = one(at(0.5, 0.5, 0.0));
        think_run(&mut targets, &defs, &east(None), &slab, 4.0);
        let t = targets.iter().next().unwrap();
        assert!(x_of(&targets) > 4.0, "walked on: {}", x_of(&targets));
        assert!((t.pos.offset().z - 0.5).abs() < 1e-3, "up on the slab: {}", t.pos.offset().z);

        let block = Steps { bump_x: 2, bump: 1.0, floor_end: 1000 };
        let (mut targets, _) = one(at(0.5, 0.5, 0.0));
        think_run(&mut targets, &defs, &east(None), &block, 4.0);
        assert!((x_of(&targets) - (2.0 - 0.45)).abs() < 1e-3, "stopped at the block: {}", x_of(&targets));
        let t = targets.iter().next().unwrap();
        assert!(t.blocked, "and its brain is told");
        assert_eq!(t.velocity.x, 0.0);
    }

    #[test]
    fn the_edge_guard_stops_it_at_a_ledge_and_without_one_it_walks_off() {
        let ledge = Steps { bump_x: 1000, bump: 0.0, floor_end: 3 };
        let defs = vec![walker_def()];
        let (mut targets, _) = one(at(0.5, 0.5, 0.0));
        think_run(&mut targets, &defs, &east(Some(0.6)), &ledge, 4.0);
        let t = targets.iter().next().unwrap();
        assert!(t.on_ground && t.pos.offset().z.abs() < 1e-3, "still up top");
        assert!(x_of(&targets) > 2.5 && x_of(&targets) < 3.0 + 0.45, "at the edge: {}", x_of(&targets));
        assert!(t.blocked);

        let (mut targets, _) = one(at(0.5, 0.5, 0.0));
        think_run(&mut targets, &defs, &east(None), &ledge, 4.0);
        assert!(at(0.0, 0.0, 0.0).delta_to(targets.iter().next().unwrap().pos).z < -5.0, "fell off");
    }

    #[test]
    fn walking_into_a_wall_is_no_slam_but_being_knocked_into_one_is() {
        let terrain = Room { wall_x: 3, ice_y: 1000 };
        let defs = vec![TargetDef { slam_speed: 1.0, ..walker_def() }];
        let (mut targets, _) = one(at(0.5, 0.5, 0.0));
        think_run(&mut targets, &defs, &Walker { walk: Vec2::new(3.0, 0.0), guard: None }, &terrain, 3.0);
        assert!(targets.take_slams().is_empty(), "walking at 3 m/s into a wall");
        // Thrown at the wall faster than it walks.
        targets.list[0].velocity = Vec3::new(9.0, 0.0, 0.0);
        targets.list[0].on_ground = false;
        think_run(&mut targets, &defs, &Walker { walk: Vec2::new(3.0, 0.0), guard: None }, &terrain, 0.2);
        let slams = targets.take_slams();
        assert!(slams.iter().any(|s| s.speed > 1.0), "{slams:?}");
    }

    #[test]
    fn brains_get_their_memory_back_hurts_and_the_player_and_set_the_clip_and_facing() {
        let defs = vec![walker_def()];
        let (mut targets, _) = one(at(0.5, 0.5, 0.0));
        let still = Walker { walk: Vec2::ZERO, guard: None };
        think_run(&mut targets, &defs, &still, &room(), 0.5);
        let t = targets.iter().next().unwrap();
        assert_eq!(t.memory[0], 30.0, "a think a tick, its memory kept");
        assert_eq!(t.memory[3], -3.5, "the player's offset, from its feet to theirs");
        assert_eq!((t.clip.as_str(), t.clip_speed, t.yaw), ("walk", 2.0, 1.0));
        strike_east(&mut targets, &defs);
        strike_east(&mut targets, &defs);
        think_run(&mut targets, &defs, &still, &room(), DT * 2.0);
        let t = targets.iter().next().unwrap();
        assert_eq!(t.memory[1], 10.0, "both hits' damage, told once");
        assert_eq!(t.memory[2], -1.0, "hit from the west");
    }

    #[test]
    fn a_respawned_target_starts_with_empty_memory() {
        let defs = vec![TargetDef { health: 1.0, respawn_time: 0.1, ..walker_def() }];
        let mut targets = Targets::default();
        targets.add_spot(TargetKind(0), at(0.5, 0.5, 0.0), 0.0, &defs[0]);
        let still = Walker { walk: Vec2::ZERO, guard: None };
        think_run(&mut targets, &defs, &still, &room(), 0.5);
        assert!(strike_east(&mut targets, &defs).unwrap().broke);
        think_run(&mut targets, &defs, &still, &room(), 0.1 + DT);
        let t = targets.iter().next().expect("back");
        let thinks = t.memory.first().copied().unwrap_or(0.0);
        assert!(thinks <= 2.0, "thought afresh: {thinks}");
    }

    #[test]
    fn a_hit_knocks_a_walking_target_back_and_it_walks_again_once_down() {
        let defs = vec![walker_def()];
        let (mut targets, _) = one(at(0.5, 0.5, 0.0));
        let west = Walker { walk: Vec2::new(-1.0, 0.0), guard: None };
        think_run(&mut targets, &defs, &west, &room(), 1.0);
        let before = x_of(&targets);
        strike_east(&mut targets, &defs);
        think_run(&mut targets, &defs, &west, &room(), 0.3);
        assert!(x_of(&targets) > before + 0.5, "knocked east against its walk");
        think_run(&mut targets, &defs, &west, &room(), 1.0);
        let t = targets.iter().next().unwrap();
        assert!(t.on_ground && (t.velocity.x + 1.0).abs() < 1e-2, "walking west again: {}", t.velocity.x);
    }

    #[test]
    fn without_a_brain_a_target_neither_walks_nor_picks_a_clip() {
        let defs = vec![walker_def()];
        let (mut targets, _) = one(at(0.5, 0.5, 0.0));
        think_run(&mut targets, &defs, &east(None), &room(), 0.5);
        run(&mut targets, &defs, &room(), 2.0);
        let t = targets.iter().next().unwrap();
        assert!(t.velocity.length() < 1e-4, "settled: {}", t.velocity);
        assert_eq!((t.clip.as_str(), t.walk), ("", Vec2::ZERO), "brains off: a dummy again");
    }

    /// A movement controller that only falls.
    #[derive(Clone)]
    struct Standing;

    impl api::Tunables for Standing {
        fn tunables(&self) -> Vec<api::Tunable> {
            Vec::new()
        }
        fn set_tunable(&mut self, _: &str, _: f32) -> bool {
            false
        }
    }

    impl api::MovementController for Standing {
        fn spawn(&self) -> Box<dyn api::MovementController> {
            Box::new(self.clone())
        }
        fn step(&mut self, ctx: &api::MoveContext) -> api::MoveOutput {
            api::MoveOutput { velocity: Vec3::new(0.0, 0.0, ctx.velocity.z - 25.0 * ctx.dt), ..Default::default() }
        }
    }

    fn standing_player(pos: WorldPos) -> Player {
        let shape = crate::Shape { width: 0.6, height: 1.8, eye_height: 1.6, step_height: 0.5 };
        Player::with_controller(shape, Box::new(Standing), pos)
    }

    fn crowd() -> CrowdDef {
        CrowdDef { sight: 10.0, push: 10.0, max_push: 3.0, player_weight: 1.0 }
    }

    /// Targets of `defs[0]`'s kind at `spots`, all facing east.
    fn many(spots: &[(f32, f32)], def: TargetDef) -> (Targets, Vec<TargetDef>) {
        let mut targets = Targets::default();
        for &(x, y) in spots {
            targets.add_spot(TargetKind(0), at(x, y, 0.0), 0.0, &def);
        }
        (targets, vec![def])
    }

    /// Steps the targets, and the player if any, with a crowd and `brains`.
    fn crowd_run(
        targets: &mut Targets,
        defs: &[TargetDef],
        brains: &[Option<&dyn Brain>],
        mut player: Option<&mut Player>,
        terrain: &impl Terrain,
        seconds: f32,
    ) {
        let crowd = crowd();
        for tick in 0..(seconds / DT).round() as u64 {
            if let Some(p) = player.as_deref_mut() {
                p.step(&crate::PlayerInput::default(), DT, terrain);
            }
            let ctx = StepContext { defs, brains, crowd: Some(&crowd), player: player.as_deref_mut(), tick, dt: DT };
            targets.step(ctx, terrain);
        }
    }

    fn abs_x(p: WorldPos) -> f32 {
        at(0.0, 0.0, 0.0).delta_to(p).x
    }

    fn level_gap(a: WorldPos, b: WorldPos) -> f32 {
        a.delta_to(b).truncate().length()
    }

    #[test]
    fn overlapping_targets_are_pushed_apart_and_come_to_rest() {
        let (mut targets, defs) = many(&[(0.5, 0.5), (0.8, 0.5)], def());
        crowd_run(&mut targets, &defs, &[], None, &room(), 2.0);
        let t: Vec<&Target> = targets.iter().collect();
        let gap = level_gap(t[0].pos, t[1].pos);
        assert!(gap >= 0.9 - PUSH_SLACK - 1e-3, "apart: {gap}");
        assert!(gap < 0.95, "but only just: {gap}");
        assert!(t.iter().all(|t| t.pos.delta_to(t.prev_pos).length() < 1e-5), "and still");
        assert_eq!(t[0].velocity.truncate(), Vec2::ZERO, "pushing gives no speed");
        assert!(targets.take_slams().is_empty());
    }

    #[test]
    fn the_heavier_body_moves_less() {
        let mut targets = Targets::default();
        let light = def();
        let heavy = TargetDef { weight: 6.0, ..def() };
        targets.add_spot(TargetKind(0), at(0.5, 0.5, 0.0), 0.0, &light);
        targets.add_spot(TargetKind(1), at(1.0, 0.5, 0.0), 0.0, &heavy);
        let defs = vec![light, heavy];
        crowd_run(&mut targets, &defs, &[], None, &room(), 2.0);
        let t: Vec<&Target> = targets.iter().collect();
        let (moved_light, moved_heavy) = (0.5 - abs_x(t[0].pos), abs_x(t[1].pos) - 1.0);
        assert!(moved_light > 0.0 && moved_heavy > 0.0);
        // Weights 2 and 6: the light one moves three times as far.
        assert!((moved_light / moved_heavy - 3.0).abs() < 0.1, "{moved_light} vs {moved_heavy}");
    }

    #[test]
    fn a_pile_settles_without_jitter_and_nothing_is_pushed_into_a_wall() {
        let terrain = Room { wall_x: 2, ice_y: 1000 };
        // Five in a heap against the wall.
        let spots = [(1.5, 0.5), (1.4, 0.6), (1.5, 0.7), (1.3, 0.5), (1.45, 0.55)];
        let (mut targets, defs) = many(&spots, def());
        let half = 0.45;
        for _ in 0..(6.0 / DT) as usize {
            crowd_run(&mut targets, &defs, &[], None, &terrain, DT);
            for t in targets.iter() {
                let x = at(0.0, 0.0, 0.0).delta_to(t.pos).x;
                assert!(x <= 2.0 - half + 1e-4, "never into the wall: {x}");
            }
        }
        let t: Vec<&Target> = targets.iter().collect();
        for (i, a) in t.iter().enumerate() {
            assert!(a.pos.delta_to(a.prev_pos).length() < 1e-4, "settled: {:?}", a.pos.delta_to(a.prev_pos));
            for b in &t[i + 1..] {
                assert!(level_gap(a.pos, b.pos) > 0.9 - 0.05, "spread out");
            }
        }
    }

    #[test]
    fn long_bodies_push_along_their_whole_length() {
        // Hit boxes 2 m long front to back, but a collision box only 0.9 wide.
        let long = TargetDef {
            hit_boxes: vec![api::HitBox { name: "body".into(), min: [-0.45, -1.0, 0.0], max: [0.45, 1.0, 1.3], damage: 1.0 }],
            ..def()
        };
        // Both facing east, so the bodies run east–west; the second's back is in the first's front.
        let (mut targets, defs) = many(&[(0.5, 0.5), (2.0, 0.5)], long);
        crowd_run(&mut targets, &defs, &[], None, &room(), 2.0);
        let t: Vec<&Target> = targets.iter().collect();
        assert!(level_gap(t[0].pos, t[1].pos) > 2.0 - 0.05, "{}", level_gap(t[0].pos, t[1].pos));
    }

    #[test]
    fn the_player_and_a_target_push_each_other() {
        let (mut targets, defs) = many(&[(0.5, 0.5)], def());
        let mut player = standing_player(at(0.8, 0.5, 0.0));
        crowd_run(&mut targets, &defs, &[], Some(&mut player), &room(), 2.0);
        let t = targets.iter().next().unwrap();
        let gap = level_gap(t.pos, player.pos());
        assert!(gap >= 0.45 + 0.3 - PUSH_SLACK - 1e-3, "apart: {gap}");
        // Player weight 1, target 2: the player moves twice as far.
        let (moved_target, moved_player) = (0.5 - abs_x(t.pos), abs_x(player.pos()) - 0.8);
        assert!((moved_player / moved_target - 2.0).abs() < 0.15, "{moved_player} vs {moved_target}");
        assert!(player.velocity().truncate().length() < 1e-5, "pushing gives the player no speed either");
    }

    #[test]
    fn bodies_at_different_heights_pass() {
        let mut targets = Targets::default();
        targets.add_spot(TargetKind(0), at(0.5, 0.5, 0.0), 0.0, &def());
        let defs = vec![def()];
        // Standing on a ledge above it.
        let mut player = standing_player(at(0.6, 0.5, 2.0));
        let terrain = Room { wall_x: 1000, ice_y: 1000 };
        let crowd = crowd();
        let ctx = StepContext { defs: &defs, brains: &[], crowd: Some(&crowd), player: Some(&mut player), tick: 0, dt: DT };
        targets.step(ctx, &terrain);
        assert_eq!(targets.iter().next().unwrap().shove, Vec2::ZERO);
    }

    #[test]
    fn brains_see_the_nearest_others_within_sight_with_their_signals() {
        let walker = Walker { walk: Vec2::ZERO, guard: None };
        let brains: [Option<&dyn Brain>; 1] = [Some(&walker)];
        // Eleven in a row 1.5 m apart, then one out of sight.
        let mut spots: Vec<(f32, f32)> = (0..11).map(|i| (0.5 + 1.5 * i as f32, 0.5)).collect();
        spots.push((40.0, 0.5));
        let (mut targets, defs) = many(&spots, walker_def());
        crowd_run(&mut targets, &defs, &brains, None, &room(), DT);
        let seen: Vec<&str> = targets.iter().map(|t| t.signal.as_str()).collect();
        assert_eq!(seen[0], "seen 6", "within 10 m: 1.5 to 9 m");
        assert_eq!(seen[5], "seen 8", "capped");
        assert_eq!(seen[11], "seen 0", "nobody near");

        let list = targets.neighbors(&defs, 10.0);
        let d: Vec<f32> = list[5].iter().map(|n| n.offset.x).collect();
        assert_eq!(d, vec![-1.5, 1.5, -3.0, 3.0, -4.5, 4.5, -6.0, 6.0], "nearest first, ties by age");
        assert_eq!(list[5][0].kind, "test:dummy");
        assert_eq!(list[5][0].signal, "seen 8", "last tick's signals");
        assert!(list[5][0].id < list[5][1].id);
    }

    #[test]
    fn without_a_crowd_they_neither_see_nor_push() {
        let walker = Walker { walk: Vec2::ZERO, guard: None };
        let brains: [Option<&dyn Brain>; 1] = [Some(&walker)];
        let (mut targets, defs) = many(&[(0.5, 0.5), (0.7, 0.5)], walker_def());
        for tick in 0..30 {
            targets.step(StepContext { defs: &defs, brains: &brains, crowd: None, player: None, tick, dt: DT }, &room());
        }
        let t: Vec<&Target> = targets.iter().collect();
        assert_eq!(t[0].signal, "seen 0");
        assert!((level_gap(t[0].pos, t[1].pos) - 0.2).abs() < 1e-4);
    }

    #[test]
    fn closest_points_between_segments() {
        let (p, q) = closest_points(Vec2::ZERO, Vec2::X * 2.0, Vec2::new(1.0, 1.0), Vec2::new(1.0, 3.0));
        assert_eq!((p, q), (Vec2::X, Vec2::new(1.0, 1.0)));
        let (p, q) = closest_points(Vec2::ZERO, Vec2::ZERO, Vec2::new(-1.0, 1.0), Vec2::new(1.0, 1.0));
        assert_eq!((p, q), (Vec2::ZERO, Vec2::Y), "a point and a segment");
        let (p, q) = closest_points(Vec2::ZERO, Vec2::X, Vec2::new(3.0, 0.0), Vec2::new(5.0, 0.0));
        assert_eq!((p, q), (Vec2::X, Vec2::new(3.0, 0.0)), "in line");
    }
}
