//! Boxel raycasting: walks a ray cell by cell and stops at the first box it hits. Boxes come
//! from the caller, so a boxel can hold several partial shapes.
//!
//! The walk happens relative to the ray's own cell, so it is the same at any distance from zero.

use api::{BoxelPos, I64Vec3, Vec3, WorldPos};

use crate::CellBox;

/// Where a ray stopped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// The boxel that was hit.
    pub pos: BoxelPos,
    /// Which of the boxel's occupants was hit, as the caller numbered its boxes.
    pub occupant: usize,
    /// The outward normal of the face the ray entered through, one unit along one axis.
    /// Zero if the ray started inside the box.
    pub normal: I64Vec3,
    /// Distance along the ray to the face it entered through, in meters.
    pub distance: f32,
    /// Where the ray met the face, in boxel space (0 to 1 from the boxel's minimum corner).
    pub point: Vec3,
}

impl Hit {
    /// The boxel on the other side of the face that was hit, next to `pos`.
    pub fn adjacent(&self) -> BoxelPos {
        self.pos.offset(self.normal)
    }
}

/// Casts a ray from `origin` along `dir` for up to `max_distance` meters and returns the first
/// box it hits. `boxes` adds each boxel's boxes, in boxel space, tagged with which occupant
/// they belong to. Uses Amanatides and Woo's cell walk, so no boxel is skipped.
pub fn raycast(
    origin: WorldPos,
    dir: Vec3,
    max_distance: f32,
    boxes: impl Fn(BoxelPos, &mut Vec<(CellBox, usize)>),
) -> Option<Hit> {
    let dir = dir.normalize_or_zero();
    let start = origin.cell();
    let offset = origin.offset();
    let mut found = Vec::new();
    let mut visit = |rel: I64Vec3| -> Option<Hit> {
        let pos = BoxelPos(start + rel);
        found.clear();
        boxes(pos, &mut found);
        // The ray's origin in this boxel's space.
        let from = offset - rel.as_vec3();
        let mut best: Option<Hit> = None;
        for &(b, occupant) in &found {
            if let Some((distance, normal)) = ray_box(from, dir, b)
                && distance <= max_distance
                && best.is_none_or(|h| distance < h.distance)
            {
                let point = from + dir * distance;
                best = Some(Hit { pos, occupant, normal, distance, point });
            }
        }
        best
    };

    if let Some(hit) = visit(I64Vec3::ZERO) {
        return Some(hit);
    }
    if dir == Vec3::ZERO {
        return None;
    }

    let mut step = [0i64; 3];
    // Distance along the ray to the next cell border on each axis, and between borders.
    let mut next = [f32::INFINITY; 3];
    let mut between = [f32::INFINITY; 3];
    for axis in 0..3 {
        let d = dir[axis];
        if d > 0.0 {
            step[axis] = 1;
            next[axis] = (1.0 - offset[axis]) / d;
            between[axis] = 1.0 / d;
        } else if d < 0.0 {
            step[axis] = -1;
            next[axis] = offset[axis] / -d;
            between[axis] = 1.0 / -d;
        }
    }

    let mut rel = [0i64; 3];
    loop {
        let axis = (0..3).min_by(|&a, &b| next[a].total_cmp(&next[b])).expect("three axes");
        if next[axis] > max_distance {
            return None;
        }
        rel[axis] += step[axis];
        next[axis] += between[axis];
        if let Some(hit) = visit(I64Vec3::from_array(rel)) {
            return Some(hit);
        }
    }
}

/// Where a ray from `from` along unit `dir` enters box `b`: the distance and the outward
/// normal of the face, or distance 0 and no normal if it starts inside. `None` if it misses.
pub(crate) fn ray_box(from: Vec3, dir: Vec3, b: CellBox) -> Option<(f32, I64Vec3)> {
    let (mut enter, mut exit) = (f32::NEG_INFINITY, f32::INFINITY);
    let mut enter_axis = None;
    for axis in 0..3 {
        let (o, d) = (from[axis], dir[axis]);
        if d == 0.0 {
            if o < b.min[axis] || o > b.max[axis] {
                return None;
            }
            continue;
        }
        let (t0, t1) = ((b.min[axis] - o) / d, (b.max[axis] - o) / d);
        let (near, far) = if t0 < t1 { (t0, t1) } else { (t1, t0) };
        if near > enter {
            enter = near;
            enter_axis = Some(axis);
        }
        exit = exit.min(far);
    }
    if enter > exit || exit < 0.0 {
        return None;
    }
    match enter_axis {
        Some(axis) if enter >= 0.0 => {
            let mut normal = [0i64; 3];
            normal[axis] = if dir[axis] > 0.0 { -1 } else { 1 };
            Some((enter, I64Vec3::from_array(normal)))
        }
        _ => Some((0.0, I64Vec3::ZERO)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32, z: f32) -> WorldPos {
        WorldPos::new(I64Vec3::ZERO, Vec3::new(x, y, z))
    }

    /// Full boxels wherever `solid` says.
    fn full(solid: impl Fn(BoxelPos) -> bool) -> impl Fn(BoxelPos, &mut Vec<(CellBox, usize)>) {
        move |p, out| {
            if solid(p) {
                out.push((CellBox::FULL, 0));
            }
        }
    }

    fn only(target: BoxelPos) -> impl Fn(BoxelPos, &mut Vec<(CellBox, usize)>) {
        full(move |p| p == target)
    }

    #[test]
    fn hits_each_face_with_the_right_normal() {
        let block = BoxelPos::new(0, 0, 0);
        let cases = [
            (at(3.5, 0.5, 0.5), -Vec3::X, I64Vec3::X),
            (at(-2.5, 0.5, 0.5), Vec3::X, I64Vec3::NEG_X),
            (at(0.5, 3.5, 0.5), -Vec3::Y, I64Vec3::Y),
            (at(0.5, -2.5, 0.5), Vec3::Y, I64Vec3::NEG_Y),
            (at(0.5, 0.5, 3.5), -Vec3::Z, I64Vec3::Z),
            (at(0.5, 0.5, -2.5), Vec3::Z, I64Vec3::NEG_Z),
        ];
        for (origin, dir, normal) in cases {
            let hit = raycast(origin, dir, 10.0, only(block)).expect("should hit");
            assert_eq!(hit.pos, block);
            assert_eq!(hit.normal, normal, "from {origin:?}");
            assert!((hit.distance - 2.5).abs() < 1e-5, "distance {}", hit.distance);
            assert_eq!(hit.adjacent(), block.offset(normal));
        }
    }

    #[test]
    fn stops_at_the_first_block() {
        let wall = |p: BoxelPos| p.0.x >= 3;
        let hit = raycast(at(0.5, 0.5, 0.5), Vec3::X, 10.0, full(wall)).unwrap();
        assert_eq!(hit.pos, BoxelPos::new(3, 0, 0));
    }

    #[test]
    fn diagonal_rays_do_not_skip_cells() {
        // A ray just above the diagonal passes through (1, 0) before (1, 1), and must hit it.
        let hit = raycast(at(0.5, 0.5, 0.5), Vec3::new(1.0, 0.999, 0.0), 10.0, only(BoxelPos::new(1, 0, 0)));
        assert!(hit.is_some());
        let hit = raycast(at(0.5, 0.4, 0.5), Vec3::new(1.0, 1.0, 0.0), 10.0, only(BoxelPos::new(1, 0, 0))).unwrap();
        assert_eq!(hit.normal, I64Vec3::NEG_X);
    }

    #[test]
    fn respects_reach() {
        let block = BoxelPos::new(5, 0, 0);
        assert!(raycast(at(0.5, 0.5, 0.5), Vec3::X, 4.4, only(block)).is_none());
        assert!(raycast(at(0.5, 0.5, 0.5), Vec3::X, 4.5, only(block)).is_some());
        assert!(raycast(at(0.5, 0.5, 0.5), Vec3::ZERO, 10.0, only(block)).is_none());
    }

    #[test]
    fn starting_inside_a_block_hits_it_with_no_face() {
        let hit = raycast(at(0.5, 0.5, 0.5), Vec3::X, 5.0, full(|_| true)).unwrap();
        assert_eq!(hit.pos, BoxelPos::new(0, 0, 0));
        assert_eq!(hit.normal, I64Vec3::ZERO);
    }

    #[test]
    fn works_at_negative_coordinates() {
        let block = BoxelPos::new(-3, -1, -2);
        let hit = raycast(at(-0.5, -0.5, -1.5), Vec3::new(-1.0, -0.2, 0.0), 10.0, only(block)).unwrap();
        assert_eq!(hit.pos, block);
        assert_eq!(hit.normal, I64Vec3::X);
    }

    #[test]
    fn identical_in_the_millions() {
        let shift = I64Vec3::new(3_000_000, -2_000_000, 1_000_000);
        let solid = |p: BoxelPos| p.0.z < -1 || p.0.x >= 4;
        let far_solid = move |p: BoxelPos| solid(BoxelPos(p.0 - shift));
        let near = at(0.3, 0.7, 1.62);
        let far = WorldPos::new(near.cell() + shift, near.offset());
        for i in 0..200 {
            let a = i as f32 * 0.1;
            let dir = Vec3::new(a.cos(), a.sin(), -0.4 - (i % 7) as f32 * 0.1);
            let h1 = raycast(near, dir, 6.0, full(solid));
            let h2 = raycast(far, dir, 6.0, full(far_solid));
            assert_eq!(
                h1.map(|h| (h.pos.0 + shift, h.normal, h.distance, h.point)),
                h2.map(|h| (h.pos.0, h.normal, h.distance, h.point))
            );
        }
    }

    #[test]
    fn partial_boxes_are_hit_where_they_are() {
        // A bottom slab: a ray level with its top half passes over it, one lower hits its side.
        let slab = CellBox { min: Vec3::ZERO, max: Vec3::new(1.0, 1.0, 0.5) };
        let boxes = move |p: BoxelPos, out: &mut Vec<(CellBox, usize)>| {
            if p == BoxelPos::new(3, 0, 0) {
                out.push((slab, 0));
            }
        };
        assert!(raycast(at(0.5, 0.5, 0.75), Vec3::X, 10.0, boxes).is_none());
        let hit = raycast(at(0.5, 0.5, 0.25), Vec3::X, 10.0, boxes).unwrap();
        assert_eq!((hit.pos, hit.normal), (BoxelPos::new(3, 0, 0), I64Vec3::NEG_X));
        assert!((hit.distance - 2.5).abs() < 1e-5);
        assert!((hit.point - Vec3::new(0.0, 0.5, 0.25)).length() < 1e-5, "{}", hit.point);

        // From above, it hits the slab's top inside the boxel, not the boxel's top.
        let hit = raycast(at(3.5, 0.5, 2.0), -Vec3::Z, 10.0, boxes).unwrap();
        assert_eq!(hit.normal, I64Vec3::Z);
        assert!((hit.distance - 1.5).abs() < 1e-5);
        assert!((hit.point.z - 0.5).abs() < 1e-5);
    }

    #[test]
    fn the_nearest_occupant_in_a_boxel_wins() {
        // A post through a bottom slab: from above, the post's top is nearer than the slab's.
        let slab = CellBox { min: Vec3::ZERO, max: Vec3::new(1.0, 1.0, 0.5) };
        let post = CellBox { min: Vec3::new(0.375, 0.375, 0.0), max: Vec3::new(0.625, 0.625, 1.0) };
        let boxes = move |p: BoxelPos, out: &mut Vec<(CellBox, usize)>| {
            if p == BoxelPos::new(0, 0, 0) {
                out.extend([(slab, 0), (post, 1)]);
            }
        };
        assert_eq!(raycast(at(0.5, 0.5, 3.0), -Vec3::Z, 10.0, boxes).unwrap().occupant, 1);
        assert_eq!(raycast(at(0.2, 0.2, 3.0), -Vec3::Z, 10.0, boxes).unwrap().occupant, 0);
    }
}
