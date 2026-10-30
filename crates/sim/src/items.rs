//! Loose items: an amount of a material lying in the world, dropped by a broken block. Each is
//! a small copy of the block it came from, colliding as the box around that shape: it falls,
//! bounces and slides to a stop against the boxes of solid blocks, and drifts toward a body
//! that comes close enough to pick it up.
//!
//! Like the player, each item does its collision math relative to its own cell, so it behaves
//! the same at any distance from zero.

use api::{ItemDropDef, Vec3, WorldPos};

use crate::player::{Terrain, clamp_axis, sweep_box};
use crate::{BlockId, BlockRegistry, CellBox, Drop};

/// How far below an item still counts as resting on something.
const GROUND_PROBE: f32 = 0.01;
/// Slower hits than this, in meters per second, stop an item instead of bouncing it, so it
/// settles instead of buzzing on the floor.
const MIN_BOUNCE_SPEED: f32 = 1.0;
/// How quickly a pulled item's velocity turns toward the body, per second.
const PULL_RESPONSE: f32 = 12.0;
/// The smallest and largest an item is drawn, as a share of its box.
const MIN_SCALE: f32 = 0.5;
const MAX_SCALE: f32 = 2.0;
/// The longest an item's box gets, in meters, so it always fits in a boxel.
const MAX_SIZE: f32 = 0.9;

/// One loose item.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Unique among the items of one simulation, and never reused, so a renderer can follow it.
    pub id: u64,
    pub drop: Drop,
    /// The block it looks like: the one that broke, or once merged, its material's own block.
    pub block: BlockId,
    /// That block's shape bounds, unturned, in boxel units.
    pub bounds: CellBox,
    /// That block's volume, in [`crate::FULL_VOLUME`] units.
    pub block_volume: u32,
    /// The center of its box.
    pub pos: WorldPos,
    pub prev_pos: WorldPos,
    pub velocity: Vec3,
    pub on_ground: bool,
    /// Being pulled toward a body this tick.
    pub pulled: bool,
}

impl Item {
    /// How much bigger than its box it's drawn: the cube root of how many of its block's worth it
    /// holds, so the volume it shows follows the volume it holds, within limits. Only for
    /// drawing; its box stays the same.
    pub fn growth(&self) -> f32 {
        (self.drop.amount as f32 / self.block_volume.max(1) as f32).cbrt().clamp(MIN_SCALE, MAX_SCALE)
    }

    /// How much smaller than its block its box is: `def.size`, or less if the box would be too
    /// big to fit in a boxel. Merging never changes it, so a merged item can't grow into a wall.
    pub fn scale(&self, def: &ItemDropDef) -> f32 {
        let longest = (self.bounds.max - self.bounds.min).max_element();
        def.size.min(MAX_SIZE / longest)
    }

    /// The size of its box, in meters.
    pub fn extent(&self, def: &ItemDropDef) -> Vec3 {
        (self.bounds.max - self.bounds.min) * self.scale(def)
    }
}

/// What an item of `block`'s shape needs: its bounds and volume.
fn shape_of(block: BlockId, blocks: &BlockRegistry) -> (CellBox, u32) {
    let shape = blocks.get(block).map_or(&[][..], |def| &def.shape[..]);
    let boxes: Vec<_> = shape.iter().map(|&b| CellBox::from_block_box(b)).collect();
    (CellBox::bounds(&boxes).unwrap_or(CellBox::FULL), crate::volume(shape).max(1))
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
    /// Drops `drop`, looking like `block`, with its center at `pos`, moving at `velocity`.
    pub fn spawn(&mut self, drop: Drop, block: BlockId, blocks: &BlockRegistry, pos: WorldPos, velocity: Vec3) {
        let (bounds, block_volume) = shape_of(block, blocks);
        self.list.push(Item {
            id: self.next_id,
            drop,
            block,
            bounds,
            block_volume,
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
        blocks: &BlockRegistry,
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
        self.merge(def, blocks);
        picked
    }

    /// Merges items of the same material closer than `def.merge_radius`, each into the one
    /// holding more (the older on a tie), which keeps its place and motion and becomes the
    /// material's own block. If that changes its box, the box keeps its bottom where it was, so
    /// it stays on what it rested on.
    fn merge(&mut self, def: &ItemDropDef, blocks: &BlockRegistry) {
        let radius = def.merge_radius;
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
                let kept = &mut self.list[keep];
                let half_before = kept.extent(def).z * 0.5;
                kept.drop.amount = kept.drop.amount.saturating_add(amount);
                kept.block = blocks.material_block(kept.drop.material);
                (kept.bounds, kept.block_volume) = shape_of(kept.block, blocks);
                let grow = kept.extent(def).z * 0.5 - half_before;
                kept.pos = kept.pos.translate(Vec3::Z * grow);
                kept.prev_pos = kept.prev_pos.translate(Vec3::Z * grow);
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

    let half = item.extent(def) * 0.5;
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

    /// Stone, a stone slab and dirt.
    struct Fixture {
        blocks: BlockRegistry,
        stone_block: BlockId,
        slab_block: BlockId,
        dirt_block: BlockId,
        stone: Drop,
        dirt: Drop,
    }

    fn fixture() -> Fixture {
        let mut blocks = BlockRegistry::default();
        let slab_shape = vec![api::BlockBox::new([0, 0, 0], [16, 16, 8])];
        // The slab names stone first, so stone's own block is still the full one.
        let slab_block = blocks
            .register(BlockDef::new("test:stone_slab", true, [0; 3]).with_shape(slab_shape).with_material("test:stone"))
            .unwrap();
        let stone_block = blocks.register(BlockDef::new("test:stone", true, [0; 3]).with_material("test:stone")).unwrap();
        let dirt_block = blocks.register(BlockDef::new("test:dirt", true, [0; 3]).with_material("test:dirt")).unwrap();
        let stone = blocks.drop(stone_block).unwrap();
        let dirt = blocks.drop(dirt_block).unwrap();
        Fixture { blocks, stone_block, slab_block, dirt_block, stone, dirt }
    }

    impl Fixture {
        fn spawn(&self, items: &mut Items, drop: Drop, block: BlockId, pos: WorldPos, velocity: Vec3) {
            items.spawn(drop, block, &self.blocks, pos, velocity);
        }

        fn step(&self, items: &mut Items, terrain: &impl Terrain, collector: Option<Collector>) -> Vec<Drop> {
            items.step(&def(), &self.blocks, DT, terrain, collector)
        }
    }

    fn at(origin: I64Vec3, x: f32, y: f32, z: f32) -> WorldPos {
        WorldPos::new(origin, Vec3::new(x, y, z))
    }

    fn height(origin: I64Vec3, pos: WorldPos) -> f32 {
        WorldPos::new(origin, Vec3::ZERO).delta_to(pos).z
    }

    /// Drops one item from `z` onto the floor and lets it settle. Returns the heights where it
    /// peaked after each bounce, and the item at rest.
    fn drop_and_settle(f: &Fixture, drop: Drop, block: BlockId) -> (Vec<f32>, Item) {
        let origin = I64Vec3::ZERO;
        let mut items = Items::default();
        f.spawn(&mut items, drop, block, at(origin, 0.5, 0.5, 3.0), Vec3::ZERO);
        let floor = Floor { origin };
        let mut peaks = Vec::new();
        let mut rising = false;
        let mut last = 3.0;
        for _ in 0..600 {
            f.step(&mut items, &floor, None);
            let h = height(origin, items.iter().next().unwrap().pos);
            if rising && h < last {
                peaks.push(last);
            }
            rising = h > last;
            last = h;
        }
        (peaks, items.iter().next().unwrap().clone())
    }

    #[test]
    fn falls_bounces_lower_each_time_and_settles() {
        let f = fixture();
        let (peaks, item) = drop_and_settle(&f, f.stone, f.stone_block);
        assert!(!peaks.is_empty(), "it bounced");
        assert!(peaks.windows(2).all(|w| w[1] < w[0]), "each bounce is lower: {peaks:?}");
        assert!(item.on_ground);
        assert_eq!(item.velocity, Vec3::ZERO);
        let rest = height(I64Vec3::ZERO, item.pos);
        assert!((rest - item.extent(&def()).z * 0.5).abs() < 1e-3, "rests on the floor, at {rest}");
    }

    #[test]
    fn a_slab_drops_as_a_small_slab_that_lies_low() {
        let f = fixture();
        let slab = f.blocks.drop(f.slab_block).unwrap();
        let (_, item) = drop_and_settle(&f, slab, f.slab_block);
        assert_eq!(item.block, f.slab_block);
        let size = def().size;
        assert!((item.extent(&def()) - Vec3::new(size, size, size / 2.0)).length() < 1e-5, "a slab's shape at the same scale");
        let rest = height(I64Vec3::ZERO, item.pos);
        assert!((rest - size / 4.0).abs() < 1e-3, "its middle is a quarter of the size up, at {rest}");
    }

    #[test]
    fn slides_to_a_stop() {
        let origin = I64Vec3::ZERO;
        let f = fixture();
        let mut items = Items::default();
        f.spawn(&mut items, f.stone, f.stone_block, at(origin, 0.5, 0.5, 0.125), Vec3::new(4.0, 0.0, 0.0));
        let floor = Floor { origin };
        for _ in 0..300 {
            f.step(&mut items, &floor, None);
        }
        let item = items.iter().next().unwrap();
        assert!(item.velocity.length() < 1e-3);
        assert!(item.pos.delta_to(at(origin, 0.5, 0.5, 0.125)).length() > 0.1, "it slid first");
    }

    #[test]
    fn a_nearby_body_pulls_items_in_and_picks_them_up() {
        let origin = I64Vec3::ZERO;
        let f = fixture();
        let mut items = Items::default();
        f.spawn(&mut items, f.stone, f.stone_block, at(origin, 1.8, 0.5, 0.125), Vec3::ZERO);
        let floor = Floor { origin };
        let body = Collector { feet: at(origin, 0.0, 0.5, 0.0), width: 0.6, height: 1.8 };
        let mut picked = Vec::new();
        for _ in 0..120 {
            picked.extend(f.step(&mut items, &floor, Some(body)));
        }
        assert!(items.is_empty());
        assert_eq!(picked, [f.stone]);

        // Out of range nothing moves toward it.
        f.spawn(&mut items, f.stone, f.stone_block, at(origin, 5.0, 0.5, 0.125), Vec3::ZERO);
        for _ in 0..120 {
            assert!(f.step(&mut items, &floor, Some(body)).is_empty());
        }
        assert!(!items.iter().next().unwrap().pulled);
    }

    #[test]
    fn close_items_of_one_material_merge_into_the_larger_as_the_materials_block() {
        let origin = I64Vec3::ZERO;
        let f = fixture();
        let slab = f.blocks.drop(f.slab_block).unwrap();
        let mut items = Items::default();
        let mut add = |drop, block, x| f.spawn(&mut items, drop, block, at(origin, x, 0.5, 0.125), Vec3::ZERO);
        add(slab, f.slab_block, 0.5);
        add(slab, f.slab_block, 1.2);
        add(f.dirt, f.dirt_block, 0.9);
        add(f.stone, f.stone_block, 4.0);
        f.step(&mut items, &Floor { origin }, None);
        let left: Vec<_> = items.iter().map(|i| (i.id, i.drop, i.block)).collect();
        assert_eq!(
            left,
            [(0, f.stone, f.stone_block), (2, f.dirt, f.dirt_block), (3, f.stone, f.stone_block)],
            "two slabs make a stone cube; the far stone and the dirt stay apart"
        );
        let merged = items.iter().next().unwrap();
        assert!((merged.extent(&def()) - Vec3::splat(def().size)).length() < 1e-5, "a cube at the same scale");

        // Merging more makes it bigger.
        let mut items = Items::default();
        f.spawn(&mut items, f.stone, f.stone_block, at(origin, 0.5, 0.5, 0.125), Vec3::ZERO);
        f.spawn(&mut items, f.stone, f.stone_block, at(origin, 1.0, 0.5, 0.125), Vec3::ZERO);
        f.step(&mut items, &Floor { origin }, None);
        assert!(items.iter().next().unwrap().growth() > 1.0);
    }

    #[test]
    fn a_merged_item_grows_up_from_the_floor_instead_of_sinking_into_it() {
        let origin = I64Vec3::ZERO;
        let f = fixture();
        let slab = f.blocks.drop(f.slab_block).unwrap();
        let size = def().size;
        let floor = Floor { origin };
        // Two resting slab items merging into a stone cube, and two cubes into a bigger cube.
        for (drop, block, rest) in [(slab, f.slab_block, size / 4.0), (f.stone, f.stone_block, size / 2.0)] {
            let mut items = Items::default();
            f.spawn(&mut items, drop, block, at(origin, 0.5, 0.5, rest), Vec3::ZERO);
            f.spawn(&mut items, drop, block, at(origin, 1.0, 0.5, rest), Vec3::ZERO);
            for _ in 0..120 {
                f.step(&mut items, &floor, None);
            }
            assert_eq!(items.len(), 1, "merged");
            let item = items.iter().next().unwrap();
            let bottom = height(origin, item.pos) - item.extent(&def()).z * 0.5;
            assert!(bottom.abs() < 1e-3, "{block:?} merged rests on the floor, its bottom at {bottom}");
            assert!(item.on_ground);
        }
    }

    /// Full solid boxels at the listed spots.
    struct Boxels(Vec<BoxelPos>);

    impl Terrain for Boxels {
        fn boxes(&self, pos: BoxelPos, out: &mut Vec<CellBox>) {
            if self.0.contains(&pos) {
                out.push(CellBox::FULL);
            }
        }

        fn friction(&self, _: BoxelPos) -> f32 {
            1.0
        }
    }

    #[test]
    fn merging_beside_a_wall_keeps_the_box_out_of_it_so_the_item_falls_when_its_floor_goes() {
        let origin = I64Vec3::ZERO;
        let f = fixture();
        // A block to rest on, a tall wall east of it, and a floor one boxel lower.
        let support = BoxelPos::new(0, 0, -1);
        let mut solid = vec![support];
        solid.extend((-1..4).map(|z| BoxelPos::new(1, 0, z)));
        solid.extend((-3..3).map(|x| BoxelPos::new(x, 0, -2)));
        let mut terrain = Boxels(solid);

        let mut items = Items::default();
        let rest = def().size / 2.0;
        // Right against the wall, and another to merge into it.
        f.spawn(&mut items, f.stone, f.stone_block, at(origin, 1.0 - rest - 0.01, 0.5, rest), Vec3::ZERO);
        f.spawn(&mut items, f.stone, f.stone_block, at(origin, 0.4, 0.5, rest), Vec3::ZERO);
        for _ in 0..60 {
            f.step(&mut items, &terrain, None);
        }
        assert_eq!(items.len(), 1, "merged");
        let item = items.iter().next().unwrap();
        assert!(item.growth() > 1.0, "a merged item is drawn bigger");
        assert!((item.extent(&def()) - Vec3::splat(def().size)).length() < 1e-5, "but its box stays one block's worth");
        let east = WorldPos::new(origin, Vec3::ZERO).delta_to(item.pos).x + item.extent(&def()).x * 0.5;
        assert!(east <= 1.0, "clear of the wall, its east side at {east}");

        terrain.0.retain(|&p| p != support);
        for _ in 0..120 {
            f.step(&mut items, &terrain, None);
        }
        let item = items.iter().next().unwrap();
        let bottom = height(origin, item.pos) - item.extent(&def()).z * 0.5;
        assert!((bottom + 1.0).abs() < 1e-3, "fell to the floor below, its bottom at {bottom}");
    }

    #[test]
    fn behaves_the_same_far_from_zero() {
        let f = fixture();
        let run = |origin: I64Vec3| {
            let mut items = Items::default();
            f.spawn(&mut items, f.stone, f.slab_block, at(origin, 0.3, 0.6, 2.0), Vec3::new(1.5, -0.7, 2.0));
            let body = Collector { feet: at(origin, 2.5, 0.5, 0.0), width: 0.6, height: 1.8 };
            let floor = Floor { origin };
            let mut path = Vec::new();
            for _ in 0..240 {
                f.step(&mut items, &floor, Some(body));
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
