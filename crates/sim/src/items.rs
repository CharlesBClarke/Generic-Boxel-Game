//! Loose items: an amount of a material lying in the world, dropped by a broken block. Each is
//! a small box that falls, bounces and slides to a stop against the boxes of solid blocks, and
//! drifts toward a body that comes close enough to pick it up.
//!
//! Like the player, each item does its collision math relative to its own cell, so it behaves
//! the same at any distance from zero.

use api::{ItemDropDef, Vec3, WorldPos};

use crate::player::{Terrain, clamp_axis, sweep_box};
use crate::{Drop, FULL_VOLUME};

/// How far below an item still counts as resting on something.
const GROUND_PROBE: f32 = 0.01;
/// Slower hits than this, in meters per second, stop an item instead of bouncing it, so it
/// settles instead of buzzing on the floor.
const MIN_BOUNCE_SPEED: f32 = 1.0;
/// How quickly a pulled item's velocity turns toward the body, per second.
const PULL_RESPONSE: f32 = 12.0;
/// The smallest and largest an item gets, as a share of a full block's worth of size.
const MIN_SCALE: f32 = 0.5;
const MAX_SCALE: f32 = 2.0;
/// The largest an item gets, in meters, so it always fits in a boxel.
const MAX_SIZE: f32 = 0.9;

/// One loose item.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Unique among the items of one simulation, and never reused, so a renderer can follow it.
    pub id: u64,
    pub drop: Drop,
    /// The center of its box.
    pub pos: WorldPos,
    pub prev_pos: WorldPos,
    pub velocity: Vec3,
    pub on_ground: bool,
    /// Being pulled toward a body this tick.
    pub pulled: bool,
}

impl Item {
    /// Its edge length: `def.size` for a full block's worth, scaled by the cube root of the
    /// amount so the volume it shows follows the volume it holds, within limits.
    pub fn size(&self, def: &ItemDropDef) -> f32 {
        let scale = (self.drop.amount as f32 / FULL_VOLUME as f32).cbrt().clamp(MIN_SCALE, MAX_SCALE);
        (def.size * scale).min(MAX_SIZE)
    }
}

/// A body that picks up items: a box from its feet, `width` across and `height` tall.
#[derive(Clone, Copy, Debug)]
pub struct Collector {
    pub feet: WorldPos,
    pub width: f32,
    pub height: f32,
}

impl Collector {
    /// How far `pos` is from the body's box, and the direction from `pos` to its middle.
    fn reach(&self, pos: WorldPos) -> (f32, Vec3) {
        let rel = self.feet.delta_to(pos);
        let half = self.width * 0.5;
        let closest = rel.clamp(Vec3::new(-half, -half, 0.0), Vec3::new(half, half, self.height));
        let middle = Vec3::new(0.0, 0.0, self.height * 0.5);
        ((rel - closest).length(), (middle - rel).normalize_or_zero())
    }
}

#[derive(Debug, Default)]
pub struct Items {
    list: Vec<Item>,
    next_id: u64,
}

impl Items {
    /// Drops `drop` with its center at `pos`, moving at `velocity`.
    pub fn spawn(&mut self, drop: Drop, pos: WorldPos, velocity: Vec3) {
        self.list.push(Item {
            id: self.next_id,
            drop,
            pos,
            prev_pos: pos,
            velocity,
            on_ground: false,
            pulled: false,
        });
        self.next_id += 1;
    }

    pub fn iter(&self) -> impl Iterator<Item = &Item> {
        self.list.iter()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Advances every item one tick of `dt` seconds: picks up what's close enough to
    /// `collector`, pulls what's in range, moves the rest under gravity, then merges items of
    /// the same material that ended up close together. Returns what was picked up.
    pub fn step(
        &mut self,
        def: &ItemDropDef,
        dt: f32,
        terrain: &impl Terrain,
        collector: Option<Collector>,
    ) -> Vec<Drop> {
        let mut picked = Vec::new();
        self.list.retain(|item| {
            let close = collector.is_some_and(|c| c.reach(item.pos).0 <= def.pickup_radius);
            if close {
                picked.push(item.drop);
            }
            !close
        });
        for item in &mut self.list {
            step_item(item, def, dt, terrain, collector);
        }
        self.merge(def.merge_radius);
        picked
    }

    /// Merges items of the same material closer than `radius`, each into the one holding more
    /// (the older on a tie), which keeps its place and motion.
    fn merge(&mut self, radius: f32) {
        let mut gone = vec![false; self.list.len()];
        for i in 0..self.list.len() {
            for j in i + 1..self.list.len() {
                if gone[i] || gone[j] {
                    continue;
                }
                let (a, b) = (&self.list[i], &self.list[j]);
                if a.drop.material != b.drop.material || a.pos.delta_to(b.pos).length() > radius {
                    continue;
                }
                let (keep, lose) = if b.drop.amount > a.drop.amount { (j, i) } else { (i, j) };
                let amount = self.list[lose].drop.amount;
                let kept = &mut self.list[keep].drop.amount;
                *kept = kept.saturating_add(amount);
                gone[lose] = true;
            }
        }
        let mut index = 0;
        self.list.retain(|_| {
            index += 1;
            !gone[index - 1]
        });
    }
}

fn step_item(item: &mut Item, def: &ItemDropDef, dt: f32, terrain: &impl Terrain, collector: Option<Collector>) {
    item.prev_pos = item.pos;
    let pull = collector.map(|c| c.reach(item.pos)).filter(|&(distance, _)| distance <= def.pull_radius);
    item.pulled = pull.is_some();
    if let Some((_, toward)) = pull {
        let want = toward * def.pull_speed;
        item.velocity += (want - item.velocity) * (1.0 - (-PULL_RESPONSE * dt).exp());
    } else {
        item.velocity.z -= def.gravity * dt;
        if item.on_ground {
            let keep = (-def.drag * dt).exp();
            item.velocity.x *= keep;
            item.velocity.y *= keep;
        }
    }

    let half = Vec3::splat(item.size(def) * 0.5);
    let offset = item.pos.offset();
    let (min, max) = (offset - half, offset + half);
    let cell = item.pos.cell();
    let delta = item.velocity * dt;
    let moved = sweep_box(min, max, delta, cell, terrain);
    for axis in 0..3 {
        if moved[axis] != delta[axis] {
            let speed = item.velocity[axis].abs();
            item.velocity[axis] =
                if speed >= MIN_BOUNCE_SPEED && !item.pulled { -item.velocity[axis] * def.bounce } else { 0.0 };
        }
    }
    item.pos = item.pos.translate(moved);
    let (min, max) = (min + moved, max + moved);
    item.on_ground = item.velocity.z <= 0.0 && clamp_axis(2, -GROUND_PROBE, min, max, cell, terrain) > -GROUND_PROBE;
}

/// The velocity a dropped item pops out at: `def.pop_speed` upward, plus a third of that
/// sideways in a direction picked from `seed`, so a row of breaks doesn't stack items in a line.
pub fn pop_velocity(def: &ItemDropDef, seed: u64) -> Vec3 {
    // A cheap integer hash (splitmix64's finalizer), the same on every machine.
    let mut x = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    let angle = (x >> 40) as f32 / (1u64 << 24) as f32 * std::f32::consts::TAU;
    let side = def.pop_speed / 3.0;
    Vec3::new(angle.cos() * side, angle.sin() * side, def.pop_speed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockRegistry, CellBox};
    use api::{BlockDef, BoxelPos, I64Vec3};

    const DT: f32 = 1.0 / 60.0;

    fn def() -> ItemDropDef {
        ItemDropDef {
            pop_speed: 3.0,
            gravity: 20.0,
            bounce: 0.4,
            drag: 8.0,
            size: 0.25,
            pull_radius: 2.0,
            pull_speed: 8.0,
            pickup_radius: 0.3,
            merge_radius: 1.0,
        }
    }

    /// A floor filling every boxel below z = 0, around `origin`.
    struct Floor {
        origin: I64Vec3,
    }

    impl Terrain for Floor {
        fn boxes(&self, pos: BoxelPos, out: &mut Vec<CellBox>) {
            if pos.0.z < self.origin.z {
                out.push(CellBox::FULL);
            }
        }

        fn friction(&self, _: BoxelPos) -> f32 {
            1.0
        }
    }

    fn materials() -> (Drop, Drop) {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(BlockDef::new("test:stone", true, [0; 3]).with_material("test:stone")).unwrap();
        let dirt = blocks.register(BlockDef::new("test:dirt", true, [0; 3]).with_material("test:dirt")).unwrap();
        (blocks.drop(stone).unwrap(), blocks.drop(dirt).unwrap())
    }

    fn at(origin: I64Vec3, x: f32, y: f32, z: f32) -> WorldPos {
        WorldPos::new(origin, Vec3::new(x, y, z))
    }

    fn height(origin: I64Vec3, pos: WorldPos) -> f32 {
        WorldPos::new(origin, Vec3::ZERO).delta_to(pos).z
    }

    #[test]
    fn falls_bounces_lower_each_time_and_settles() {
        let origin = I64Vec3::ZERO;
        let (stone, _) = materials();
        let mut items = Items::default();
        items.spawn(stone, at(origin, 0.5, 0.5, 3.0), Vec3::ZERO);
        let floor = Floor { origin };
        let mut peaks = Vec::new();
        let mut rising = false;
        let mut last = 3.0;
        for _ in 0..600 {
            items.step(&def(), DT, &floor, None);
            let h = height(origin, items.iter().next().unwrap().pos);
            if rising && h < last {
                peaks.push(last);
            }
            rising = h > last;
            last = h;
        }
        assert!(!peaks.is_empty(), "it bounced");
        assert!(peaks.windows(2).all(|w| w[1] < w[0]), "each bounce is lower: {peaks:?}");
        let item = items.iter().next().unwrap();
        assert!(item.on_ground);
        assert_eq!(item.velocity, Vec3::ZERO);
        assert!((last - item.size(&def()) * 0.5).abs() < 1e-3, "rests on the floor, at {last}");
    }

    #[test]
    fn slides_to_a_stop() {
        let origin = I64Vec3::ZERO;
        let (stone, _) = materials();
        let mut items = Items::default();
        items.spawn(stone, at(origin, 0.5, 0.5, 0.125), Vec3::new(4.0, 0.0, 0.0));
        let floor = Floor { origin };
        for _ in 0..300 {
            items.step(&def(), DT, &floor, None);
        }
        let item = items.iter().next().unwrap();
        assert!(item.velocity.length() < 1e-3);
        assert!(item.pos.delta_to(at(origin, 0.5, 0.5, 0.125)).length() > 0.1, "it slid first");
    }

    #[test]
    fn a_nearby_body_pulls_items_in_and_picks_them_up() {
        let origin = I64Vec3::ZERO;
        let (stone, _) = materials();
        let mut items = Items::default();
        items.spawn(stone, at(origin, 1.8, 0.5, 0.125), Vec3::ZERO);
        let floor = Floor { origin };
        let body = Collector { feet: at(origin, 0.0, 0.5, 0.0), width: 0.6, height: 1.8 };
        let mut picked = Vec::new();
        for _ in 0..120 {
            picked.extend(items.step(&def(), DT, &floor, Some(body)));
        }
        assert!(items.is_empty());
        assert_eq!(picked, [stone]);

        // Out of range nothing moves toward it.
        items.spawn(stone, at(origin, 5.0, 0.5, 0.125), Vec3::ZERO);
        for _ in 0..120 {
            assert!(items.step(&def(), DT, &floor, Some(body)).is_empty());
        }
        assert!(!items.iter().next().unwrap().pulled);
    }

    #[test]
    fn close_items_of_one_material_merge_into_the_larger() {
        let origin = I64Vec3::ZERO;
        let (stone, dirt) = materials();
        let half = Drop { amount: FULL_VOLUME / 2, ..stone };
        let mut items = Items::default();
        items.spawn(half, at(origin, 0.5, 0.5, 0.125), Vec3::ZERO);
        items.spawn(stone, at(origin, 1.2, 0.5, 0.125), Vec3::ZERO);
        items.spawn(dirt, at(origin, 0.9, 0.5, 0.125), Vec3::ZERO);
        items.spawn(stone, at(origin, 4.0, 0.5, 0.125), Vec3::ZERO);
        items.step(&def(), DT, &Floor { origin }, None);
        let left: Vec<_> = items.iter().map(|i| (i.id, i.drop)).collect();
        let merged = Drop { amount: FULL_VOLUME * 3 / 2, ..stone };
        assert_eq!(left, [(1, merged), (2, dirt), (3, stone)], "the far stone and the dirt stay apart");
        assert!(items.iter().next().unwrap().size(&def()) > def().size, "a bigger item is drawn bigger");
    }

    #[test]
    fn behaves_the_same_far_from_zero() {
        let (stone, _) = materials();
        let run = |origin: I64Vec3| {
            let mut items = Items::default();
            items.spawn(stone, at(origin, 0.3, 0.6, 2.0), Vec3::new(1.5, -0.7, 2.0));
            let body = Collector { feet: at(origin, 2.5, 0.5, 0.0), width: 0.6, height: 1.8 };
            let floor = Floor { origin };
            let mut path = Vec::new();
            for _ in 0..240 {
                items.step(&def(), DT, &floor, Some(body));
                path.extend(items.iter().map(|i| WorldPos::new(origin, Vec3::ZERO).delta_to(i.pos)));
            }
            path
        };
        let near = run(I64Vec3::ZERO);
        let far = run(I64Vec3::new(3_000_000, -2_000_000, 1_000));
        assert_eq!(near.len(), far.len());
        for (a, b) in near.iter().zip(&far) {
            assert!((*a - *b).length() < 1e-4, "{a} vs {b}");
        }
    }

    #[test]
    fn pops_mostly_up_in_varied_directions() {
        let v = pop_velocity(&def(), 7);
        assert_eq!(v.z, def().pop_speed);
        assert!((v.truncate().length() - def().pop_speed / 3.0).abs() < 1e-4);
        assert_ne!(pop_velocity(&def(), 8).truncate(), v.truncate());
    }
}
