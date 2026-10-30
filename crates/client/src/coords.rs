//! The render boundary. Engine and mod code are Z-up; Bevy is Y-up. This module is the
//! only place the two meet, and the only place world positions become render floats.

use api::{I64Vec3, WorldPos, look_direction};
use bevy::math::{Quat, Vec3};

/// Converts a Z-up vector (+X east, +Y north, +Z up) to Bevy's Y-up (+X east, +Y up,
/// -Z north). A pure rotation, so handedness and triangle winding are preserved.
pub fn to_render(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y)
}

/// Converts a Bevy Y-up vector back to Z-up.
pub fn from_render(v: Vec3) -> Vec3 {
    Vec3::new(v.x, -v.z, v.y)
}

/// Where `pos` lands in render space, relative to a world view's origin cell. Only small
/// numbers reach the GPU as long as the origin is kept near the camera.
pub fn render_translation(pos: WorldPos, origin: I64Vec3) -> Vec3 {
    to_render((pos.cell() - origin).as_vec3() + pos.offset())
}

/// The Bevy camera rotation for a Z-up yaw and pitch.
pub fn look_rotation(yaw: f32, pitch: f32) -> Quat {
    bevy::prelude::Transform::default()
        .looking_to(to_render(look_direction(yaw, pitch)), Vec3::Y)
        .rotation
}

/// The Bevy rotation for turning `angle` radians counterclockwise around Z-up's +Z.
pub fn turn_about_up(angle: f32) -> Quat {
    Quat::from_axis_angle(to_render(Vec3::Z), angle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn axes_map_to_bevy() {
        assert_eq!(to_render(Vec3::X), Vec3::X); // east stays +X
        assert_eq!(to_render(Vec3::Z), Vec3::Y); // up becomes +Y
        assert_eq!(to_render(Vec3::Y), -Vec3::Z); // north becomes -Z, Bevy's forward
    }

    #[test]
    fn round_trips() {
        let v = Vec3::new(1.5, -2.0, 3.25);
        assert_eq!(from_render(to_render(v)), v);
    }

    #[test]
    fn preserves_handedness() {
        // X × Y = Z must hold on both sides of the boundary.
        let cross = to_render(Vec3::X).cross(to_render(Vec3::Y));
        assert_eq!(cross, to_render(Vec3::Z));
    }

    #[test]
    fn translation_is_relative_to_the_origin() {
        let pos = WorldPos::new(I64Vec3::new(3_000_010, -2_000_000, 1_002), Vec3::new(0.5, 0.25, 0.0));
        let origin = I64Vec3::new(3_000_000, -2_000_000, 1_000);
        assert_eq!(render_translation(pos, origin), Vec3::new(10.5, 2.0, -0.25));
    }

    #[test]
    fn turning_about_up_is_counterclockwise_seen_from_above() {
        let east = to_render(Vec3::X);
        assert!(close(from_render(turn_about_up(std::f32::consts::FRAC_PI_2) * east), Vec3::Y));
    }

    #[test]
    fn camera_looks_where_yaw_and_pitch_say() {
        let forward = |q: Quat| from_render(q * Vec3::NEG_Z);
        assert!(close(forward(look_rotation(0.0, 0.0)), Vec3::X));
        assert!(close(forward(look_rotation(std::f32::consts::FRAC_PI_2, 0.0)), Vec3::Y));
        let up45 = forward(look_rotation(0.0, std::f32::consts::FRAC_PI_4));
        assert!(close(up45, Vec3::new(1.0, 0.0, 1.0).normalize()));
    }
}
