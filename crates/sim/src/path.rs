//! Paths for creatures: the way over the terrain from where a body stands to somewhere else.
//!
//! The search is A* over standing spots, one per boxel column and floor: the middle of the
//! column at the height the body rests at. Moving to each of the eight neighboring columns is
//! tested with the same collision sweeps that move bodies, so slabs, stairs and every crafted
//! shape are walked as they really are: a body walks across, stepping up what it can step and
//! down as far as its paths may drop, or hops up a ledge too high to step. A diagonal needs both
//! of its sides walkable, so paths don't clip corners. Each search is bounded in the spots it
//! looks at and how far it goes; a goal out of reach gives the path to the nearest spot found.
//!
//! Like bodies, the search does its math relative to the starting cell, so it works the same at
//! any distance from zero.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use api::{I64Vec3, TargetDef, Vec2, Vec3, WorldPos};

use crate::player::{Terrain, clamp_axis, sweep_box, sweep_stepping};

/// The most standing spots one search looks at. A search that uses them all (a goal out of
/// reach) takes about 3.5 ms in a release build.
pub const SEARCH_SPOTS: usize = 1000;
/// How far, level in meters, a search goes from where it starts.
pub const SEARCH_RANGE: f32 = 48.0;
/// A spot this near the goal, level and up or down, in meters, has reached it.
const ARRIVE: f32 = 1.0;
const ARRIVE_HEIGHT: f32 = 1.0;
/// What a hop costs on top of the ground it covers, meters, so paths hop only to save walking.
const HOP_COST: f32 = 1.5;
/// What each meter of drop costs, so paths keep to the high ground when it's no longer.
const DROP_COST: f32 = 0.5;
/// How much of a hop's full height a path counts on, so a hop that only just clears in theory
/// isn't planned.
const HOP_MARGIN: f32 = 0.9;
/// How far below the feet still counts as standing on something.
const PROBE: f32 = 0.01;
/// How near a sweep has to come to where it was going, meters, to have got there.
const SLACK: f32 = 1e-3;
/// How long each piece of a straight walk test is, meters.
const STRIDE: f32 = 0.5;

/// How a body gets about, for finding its paths.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Walker {
    pub width: f32,
    pub height: f32,
    pub step_height: f32,
    /// How high a hop lifts it, meters; 0 never hops.
    pub hop_height: f32,
    /// The deepest drop its paths walk off, meters.
    pub max_drop: f32,
}

impl Walker {
    pub fn of(def: &TargetDef) -> Self {
        let hop_height = if def.hop_speed > 0.0 && def.gravity > 0.0 {
            def.hop_speed * def.hop_speed / (2.0 * def.gravity) * HOP_MARGIN
        } else {
            0.0
        };
        Walker { width: def.width, height: def.height, step_height: def.step_height, hop_height, max_drop: def.max_drop }
    }

    /// Its box with its feet at `feet`.
    fn bounds(&self, feet: Vec3) -> (Vec3, Vec3) {
        let half = self.width * 0.5;
        (feet + Vec3::new(-half, -half, 0.0), feet + Vec3::new(half, half, self.height))
    }

    /// Where it comes to rest from `feet`, falling at most `depth`; `None` if there's no ground
    /// that near.
    fn settle(&self, feet: Vec3, depth: f32, cell: I64Vec3, terrain: &impl Terrain) -> Option<Vec3> {
        let (min, max) = self.bounds(feet);
        let reach = depth + PROBE;
        let down = clamp_axis(2, -reach, min, max, cell, terrain);
        (down > -reach + 1e-5).then_some(feet + Vec3::Z * down)
    }

    /// Where it ends up moving level from `feet` to `to`, and how: walking, stepping up what it
    /// can and down at most `max_drop`, or else hopping. `None` if it can do neither.
    fn move_to(&self, feet: Vec3, to: Vec2, cell: I64Vec3, terrain: &impl Terrain) -> Option<(Vec3, Step)> {
        let delta = (to - feet.truncate()).extend(0.0);
        let (min, max) = self.bounds(feet);
        let (moved, _) = sweep_stepping(min, max, delta, self.step_height, cell, terrain);
        if (moved - delta).truncate().length() < SLACK {
            return self.settle(feet + moved, self.max_drop, cell, terrain).map(|at| (at, Step::Walk));
        }
        if self.hop_height <= self.step_height {
            return None;
        }
        let up = clamp_axis(2, self.hop_height, min, max, cell, terrain);
        let lift = Vec3::Z * up;
        let side = sweep_box(min + lift, max + lift, delta, cell, terrain);
        if (side - delta).truncate().length() >= SLACK {
            return None;
        }
        let at = self.settle(feet + lift + side, up + self.max_drop, cell, terrain)?;
        // Only worth a hop if it gets somewhere a step wouldn't.
        (at.z > feet.z + self.step_height).then_some((at, Step::Hop))
    }

    /// Whether it can walk straight from `from` to `to`, without hopping or dropping further
    /// than its paths may, and end up level with `to`.
    pub fn walks_straight(&self, from: WorldPos, to: WorldPos, terrain: &impl Terrain) -> bool {
        let cell = from.cell();
        let goal = WorldPos::new(cell, Vec3::ZERO).delta_to(to);
        let mut at = from.offset();
        loop {
            let left = goal.truncate() - at.truncate();
            let length = left.length();
            if length < SLACK {
                return (at.z - goal.z).abs() <= self.step_height.max(0.5);
            }
            let piece = left * (STRIDE.min(length) / length);
            let (min, max) = self.bounds(at);
            let (moved, _) = sweep_stepping(min, max, piece.extend(0.0), self.step_height, cell, terrain);
            if (moved.truncate() - piece).length() >= SLACK {
                return false;
            }
            match self.settle(at + moved, self.max_drop, cell, terrain) {
                Some(next) => at = next,
                None => return false,
            }
        }
    }
}

/// How a path gets to one of its points from the one before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Walk,
    /// A hop up a ledge too high to step.
    Hop,
}

/// A way over the terrain: feet positions, each with how it's reached from the one before. The
/// first is where the search started.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub points: Vec<(WorldPos, Step)>,
    /// It gets to the goal. Without, it ends at the nearest spot to it the search found.
    pub reaches: bool,
}

/// A standing spot the search has found.
struct Spot {
    feet: Vec3,
    parent: usize,
    step: Step,
    cost: f32,
    done: bool,
}

/// An entry in the open list: a spot and its estimated total cost, cheapest first.
#[derive(PartialEq)]
struct Open {
    estimate: f32,
    spot: usize,
}

impl Eq for Open {}

impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other.estimate.total_cmp(&self.estimate).then(other.spot.cmp(&self.spot))
    }
}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The eight neighboring columns: the four sides first, then the diagonals, each with the two
/// sides it passes between.
const SIDES: [(i64, i64); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];
const CORNERS: [(i64, i64, usize, usize); 4] = [(1, 1, 0, 1), (-1, 1, 2, 1), (-1, -1, 2, 3), (1, -1, 0, 3)];

/// Finds the way from `from` to `to` for `walker`, looking at no more than `spots` standing
/// spots. `None` when it can't move from `from` at all.
pub fn find(from: WorldPos, to: WorldPos, walker: &Walker, spots: usize, terrain: &impl Terrain) -> Option<Path> {
    let cell = from.cell();
    let base = WorldPos::new(cell, Vec3::ZERO);
    let goal = base.delta_to(to);
    let start = from.offset();
    let key = |feet: Vec3| I64Vec3::new(feet.x.floor() as i64, feet.y.floor() as i64, (feet.z + 1e-3).floor() as i64);
    let near = |feet: Vec3| (feet.truncate() - goal.truncate()).length();
    let arrived = |feet: Vec3| near(feet) <= ARRIVE && (feet.z - goal.z).abs() <= ARRIVE_HEIGHT;
    let closeness = |feet: Vec3| near(feet) + (feet.z - goal.z).abs();

    let mut list = vec![Spot { feet: start, parent: 0, step: Step::Walk, cost: 0.0, done: false }];
    let mut index: HashMap<I64Vec3, usize> = HashMap::from([(key(start), 0)]);
    let mut open = BinaryHeap::from([Open { estimate: near(start), spot: 0 }]);
    let (mut best, mut reached) = (0, arrived(start));
    let mut looked = 0;
    while let Some(Open { spot, .. }) = open.pop() {
        if reached || looked >= spots {
            break;
        }
        if list[spot].done {
            continue;
        }
        list[spot].done = true;
        looked += 1;
        let feet = list[spot].feet;
        let column = key(feet);
        let middle = |dx: i64, dy: i64| Vec2::new((column.x + dx) as f32 + 0.5, (column.y + dy) as f32 + 0.5);
        let sides: Vec<Option<(Vec3, Step)>> =
            SIDES.iter().map(|&(dx, dy)| walker.move_to(feet, middle(dx, dy), cell, terrain)).collect();
        let corners = CORNERS.iter().map(|&(dx, dy, a, b)| {
            let walkable = |s: &Option<(Vec3, Step)>| s.is_some_and(|(_, step)| step == Step::Walk);
            let both = walkable(&sides[a]) && walkable(&sides[b]);
            both.then(|| walker.move_to(feet, middle(dx, dy), cell, terrain)).flatten().filter(|&(_, s)| s == Step::Walk)
        });
        let moves: Vec<(Vec3, Step)> = sides.iter().copied().chain(corners.collect::<Vec<_>>()).flatten().collect();
        for (next, step) in moves {
            if (next.truncate() - start.truncate()).length() > SEARCH_RANGE {
                continue;
            }
            let drop = (feet.z - next.z - walker.step_height).max(0.0);
            let extra = if step == Step::Hop { HOP_COST } else { 0.0 };
            let cost = list[spot].cost + (next.truncate() - feet.truncate()).length() + extra + drop * DROP_COST;
            let i = match index.get(&key(next)) {
                Some(&i) if list[i].done || list[i].cost <= cost => continue,
                Some(&i) => {
                    list[i] = Spot { feet: next, parent: spot, step, cost, done: false };
                    i
                }
                None => {
                    list.push(Spot { feet: next, parent: spot, step, cost, done: false });
                    index.insert(key(next), list.len() - 1);
                    list.len() - 1
                }
            };
            if closeness(next) < closeness(list[best].feet) {
                best = i;
            }
            if arrived(next) {
                best = i;
                reached = true;
            }
            open.push(Open { estimate: cost + near(next), spot: i });
        }
    }
    if list.len() == 1 && !reached && looked > 0 {
        return None;
    }
    let mut points = Vec::new();
    let mut at = best;
    loop {
        let s = &list[at];
        points.push((base.translate(s.feet), s.step));
        if at == 0 {
            break;
        }
        at = s.parent;
    }
    points.reverse();
    Some(Path { points, reaches: reached })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CellBox;
    use api::BoxelPos;
    use std::collections::HashSet;

    /// A floor below z = 0, with solid columns of the given heights and holes through the floor.
    #[derive(Default)]
    struct Ground {
        columns: HashMap<(i64, i64), i64>,
        holes: HashSet<(i64, i64)>,
        slabs: HashSet<(i64, i64)>,
    }

    impl Ground {
        fn column(mut self, x: i64, y: i64, high: i64) -> Self {
            self.columns.insert((x, y), high);
            self
        }

        /// A wall `high` blocks tall along x = `x`, from `y0` to `y1`.
        fn wall(mut self, x: i64, y0: i64, y1: i64, high: i64) -> Self {
            for y in y0..=y1 {
                self.columns.insert((x, y), high);
            }
            self
        }

        fn hole(mut self, x: i64, y: i64) -> Self {
            self.holes.insert((x, y));
            self
        }

        fn slab(mut self, x: i64, y: i64) -> Self {
            self.slabs.insert((x, y));
            self
        }
    }

    impl Terrain for Ground {
        fn boxes(&self, pos: BoxelPos, out: &mut Vec<CellBox>) {
            let (x, y, z) = (pos.0.x, pos.0.y, pos.0.z);
            let high = self.columns.get(&(x, y)).copied().unwrap_or(0);
            if (z < 0 && !self.holes.contains(&(x, y))) || (z >= 0 && z < high) || (z < -20) {
                out.push(CellBox::FULL);
            } else if z == high && self.slabs.contains(&(x, y)) {
                out.push(CellBox { min: Vec3::ZERO, max: Vec3::new(1.0, 1.0, 0.5) });
            }
        }

        fn friction(&self, _: BoxelPos) -> f32 {
            1.0
        }
    }

    fn cow() -> Walker {
        Walker { width: 0.9, height: 1.3, step_height: 0.6, hop_height: 1.1, max_drop: 2.2 }
    }

    fn at(x: f32, y: f32, z: f32) -> WorldPos {
        WorldPos::new(I64Vec3::ZERO, Vec3::new(x, y, z))
    }

    fn path(ground: &Ground, walker: Walker, from: WorldPos, to: WorldPos) -> Path {
        find(from, to, &walker, SEARCH_SPOTS, ground).expect("it can move")
    }

    fn hops(p: &Path) -> usize {
        p.points.iter().filter(|(_, s)| *s == Step::Hop).count()
    }

    /// Path length, level.
    fn length(p: &Path) -> f32 {
        p.points.windows(2).map(|w| w[0].0.delta_to(w[1].0).truncate().length()).sum()
    }

    #[test]
    fn open_ground_goes_straight() {
        let p = path(&Ground::default(), cow(), at(0.5, 0.5, 0.0), at(10.5, 0.5, 0.0));
        assert!(p.reaches);
        // It stops within a meter of the goal.
        assert!((length(&p) - 9.0).abs() < 0.6, "{}", length(&p));
        assert_eq!(hops(&p), 0);
    }

    #[test]
    fn it_goes_round_a_wall() {
        let g = Ground::default().wall(5, -4, 4, 3);
        let p = path(&g, cow(), at(0.5, 0.5, 0.0), at(10.5, 0.5, 0.0));
        assert!(p.reaches);
        assert!(p.points.iter().any(|(q, _)| q.delta_to(at(5.5, 0.5, 0.0)).truncate().length() > 4.0), "it went round");
        assert!(p.points.iter().all(|(q, _)| q.offset().z < 0.01 || q.cell().z == 0), "never on the wall");
    }

    #[test]
    fn it_goes_through_a_gap_one_block_wide() {
        // A long wall with a gap at y = 0.
        let g = Ground::default().wall(5, -40, -1, 3).wall(5, 1, 40, 3);
        let p = path(&g, cow(), at(0.5, 0.5, 0.0), at(10.5, 0.5, 0.0));
        assert!(p.reaches);
        assert!(length(&p) < 12.0, "through the gap, not round: {}", length(&p));
    }

    #[test]
    fn it_hops_up_a_block_and_not_without_a_hop() {
        // A step of one block across the whole way.
        let mut g = Ground::default();
        for y in -50..50 {
            for x in 5..15 {
                g = g.column(x, y, 1);
            }
        }
        let p = path(&g, cow(), at(0.5, 0.5, 0.0), at(10.5, 0.5, 1.0));
        assert!(p.reaches);
        assert_eq!(hops(&p), 1);
        let p = path(&g, Walker { hop_height: 0.0, ..cow() }, at(0.5, 0.5, 0.0), at(10.5, 0.5, 1.0));
        assert!(!p.reaches, "can't get up without hopping");
    }

    #[test]
    fn slabs_are_stepped_not_hopped() {
        // A staircase of half blocks: a slab, a block, a block and a slab, two blocks.
        let g = Ground::default().slab(3, 0).column(4, 0, 1).column(5, 0, 1).slab(5, 0).column(6, 0, 2);
        // Walled in on both sides, so the steps are the only way.
        let g = (3..7).fold(g, |g, x| g.column(x, 1, 9).column(x, -1, 9));
        let p = path(&g, cow(), at(0.5, 0.5, 0.0), at(6.5, 0.5, 2.0));
        assert!(p.reaches, "{:?}", p.points);
        assert_eq!(hops(&p), 0);
    }

    #[test]
    fn it_wont_walk_off_a_drop_deeper_than_it_may() {
        // A pit 5 deep in front, wide enough that going round is a long way.
        let mut g = Ground::default();
        for y in -60..60 {
            for x in 3..6 {
                g = g.hole(x, y);
            }
        }
        let p = path(&g, cow(), at(0.5, 0.5, 0.0), at(4.5, 0.5, -20.0));
        assert!(!p.reaches, "the floor of the pit is too far down");
        assert!(p.points.iter().all(|(q, _)| q.delta_to(at(0.0, 0.0, 0.0)).z.abs() < 0.01));
        // A shallow one is fine.
        let shallow = Ground { holes: HashSet::new(), ..Default::default() };
        let p = path(&shallow, cow(), at(0.5, 0.5, 2.0), at(4.5, 0.5, 0.0));
        assert!(p.reaches);
    }

    #[test]
    fn a_pillar_out_of_reach_gives_the_nearest_spot() {
        let g = Ground::default().column(6, 0, 3);
        let p = path(&g, cow(), at(0.5, 0.5, 0.0), at(6.5, 0.5, 3.0));
        assert!(!p.reaches);
        let end = p.points.last().unwrap().0;
        assert!(end.delta_to(at(6.5, 0.5, 0.0)).truncate().length() < 1.5, "next to the pillar: {:?}", end);
    }

    #[test]
    fn a_search_looks_at_no_more_than_its_budget() {
        // Walled in: the goal is outside a closed ring, so it searches everything it can.
        let mut g = Ground::default();
        for i in -20..=20 {
            g = g.column(i, -20, 5).column(i, 20, 5).column(-20, i, 5).column(20, i, 5);
        }
        let p = find(at(0.5, 0.5, 0.0), at(30.5, 0.5, 0.0), &cow(), 100, &g).unwrap();
        assert!(!p.reaches);
        assert!(p.points.len() < 40);
    }

    #[test]
    fn a_straight_walk_stops_at_walls_ledges_and_cliffs() {
        let g = Ground::default().column(4, 0, 1).hole(0, 4);
        let c = cow();
        assert!(c.walks_straight(at(0.5, 0.5, 0.0), at(3.5, 0.5, 0.0), &g));
        assert!(!c.walks_straight(at(0.5, 0.5, 0.0), at(6.5, 0.5, 0.0), &g), "a block in the way");
        // A hole 20 deep across the way.
        assert!(!c.walks_straight(at(0.5, 2.5, 0.0), at(0.5, 6.5, 0.0), &g));
    }
}
