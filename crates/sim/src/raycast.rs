//! Boxel raycasting: walks a ray cell by cell and stops at the first one that counts as a hit.
//!
//! The walk happens relative to the ray's own cell, so it is the same at any distance from zero.

use api::{BoxelPos, I64Vec3, Vec3, WorldPos};

/// Where a ray stopped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// The boxel that was hit.
    pub pos: BoxelPos,
    /// The outward normal of the face the ray entered through, one unit along one axis.
    /// Zero if the ray started inside the boxel.
    pub normal: I64Vec3,
    /// Distance along the ray to the face it entered through, in meters.
    pub distance: f32,
}

impl Hit {
    /// The empty boxel in front of the face that was hit, where a placed block goes.
    pub fn adjacent(&self) -> BoxelPos {
        self.pos.offset(self.normal)
    }
}

/// Casts a ray from `origin` along `dir` for up to `max_distance` meters and returns the first
/// boxel for which `hits` is true. Uses Amanatides and Woo's cell walk, so no boxel is skipped.
pub fn raycast(origin: WorldPos, dir: Vec3, max_distance: f32, hits: impl Fn(BoxelPos) -> bool) -> Option<Hit> {
    let dir = dir.normalize_or_zero();
    let start = origin.cell();
    if hits(BoxelPos(start)) {
        return Some(Hit { pos: BoxelPos(start), normal: I64Vec3::ZERO, distance: 0.0 });
    }
    if dir == Vec3::ZERO {
        return None;
    }

    let offset = origin.offset();
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
        let distance = next[axis];
        if distance > max_distance {
            return None;
        }
        rel[axis] += step[axis];
        next[axis] += between[axis];
        let pos = BoxelPos(start + I64Vec3::from_array(rel));
        if hits(pos) {
            let mut normal = [0i64; 3];
            normal[axis] = -step[axis];
            return Some(Hit { pos, normal: I64Vec3::from_array(normal), distance });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32, z: f32) -> WorldPos {
        WorldPos::new(I64Vec3::ZERO, Vec3::new(x, y, z))
    }

    fn only(target: BoxelPos) -> impl Fn(BoxelPos) -> bool {
        move |p| p == target
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
        let hit = raycast(at(0.5, 0.5, 0.5), Vec3::X, 10.0, wall).unwrap();
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
        let hit = raycast(at(0.5, 0.5, 0.5), Vec3::X, 5.0, |_| true).unwrap();
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
            let h1 = raycast(near, dir, 6.0, solid);
            let h2 = raycast(far, dir, 6.0, far_solid);
            assert_eq!(h1.map(|h| (h.pos.0 + shift, h.normal, h.distance)), h2.map(|h| (h.pos.0, h.normal, h.distance)));
        }
    }
}
