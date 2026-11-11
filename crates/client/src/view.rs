//! How far the world is drawn: chunks farther than the render distance from the camera are
//! hidden, and the camera's far plane follows the distance. Live-tunable, to find what the
//! machine can take.

use api::{CHUNK_SIZE, Tunable, Tunables};
use bevy::prelude::*;

use crate::editing::ChunkMeshes;
use crate::{ChunkMesh, PlayerCamera, coords};

/// The view's tuning.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ViewTuning {
    /// Chunks whose centers are farther than this from the camera, in meters, aren't drawn.
    pub render_distance: f32,
}

impl Default for ViewTuning {
    fn default() -> Self {
        Self { render_distance: 512.0 }
    }
}

impl Tunables for ViewTuning {
    fn tunables(&self) -> Vec<Tunable> {
        vec![Tunable::new("render_distance", self.render_distance, 32.0, 4096.0)]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "render_distance" => self.render_distance = value.max(0.0),
            _ => return false,
        }
        true
    }
}

/// What the view draws, for the readout.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewStats {
    /// Chunks with geometry that are within the render distance.
    pub drawn: usize,
    /// Triangles in them.
    pub triangles: usize,
}

/// Whether a chunk whose lowest corner is at `corner` (render space, relative to the camera's
/// origin) is within `distance` of `eye`.
pub fn within(corner: Vec3, eye: Vec3, distance: f32) -> bool {
    let center = corner + coords::to_render(Vec3::splat(CHUNK_SIZE as f32 * 0.5));
    center.distance(eye) <= distance
}

/// Hides chunks beyond the render distance and shows the rest, and sets the far plane.
pub(crate) fn apply_render_distance(
    view: Res<ViewTuning>,
    meshes: Option<Res<ChunkMeshes>>,
    mut stats: ResMut<ViewStats>,
    mut camera: Single<(&Transform, &mut Projection), With<PlayerCamera>>,
    mut chunks: Query<(&ChunkMesh, &Transform, &mut Visibility), Without<PlayerCamera>>,
) {
    let (eye, projection) = &mut *camera;
    let eye = eye.translation;
    if let Some(p) = crate::shadows::perspective_mut(projection) {
        let far = view.render_distance + CHUNK_SIZE as f32 * 2.0;
        if (p.far - far).abs() > 0.5 {
            p.far = far;
        }
    }
    let mut drawn = ViewStats::default();
    for (chunk, transform, mut visibility) in &mut chunks {
        let show = within(transform.translation, eye, view.render_distance);
        visibility.set_if_neq(if show { Visibility::Inherited } else { Visibility::Hidden });
        if show {
            drawn.drawn += 1;
            drawn.triangles += meshes.as_ref().and_then(|m| m.quads.get(&chunk.0)).copied().unwrap_or(0) * 2;
        }
    }
    stats.set_if_neq(drawn);
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{ChunkPos, I64Vec3, WorldPos};

    /// A chunk's render-space corner, with the origin at the world's origin.
    fn corner(chunk: I64Vec3) -> Vec3 {
        coords::render_translation(WorldPos::at_boxel(ChunkPos(chunk).origin()), I64Vec3::ZERO)
    }

    #[test]
    fn chunks_within_the_distance_are_drawn() {
        // The eye at the center of chunk 0, 0, 0.
        let eye = coords::render_translation(WorldPos::at_boxel(api::BoxelPos::new(16, 16, 16)), I64Vec3::ZERO);
        assert!(within(corner(I64Vec3::ZERO), eye, 1.0));
        // Ten chunks north: 320 m away, center to center.
        assert!(within(corner(I64Vec3::new(0, 10, 0)), eye, 321.0));
        assert!(!within(corner(I64Vec3::new(0, 10, 0)), eye, 319.0));
        // And ten south, east and down are as far.
        for c in [I64Vec3::new(0, -10, 0), I64Vec3::new(10, 0, 0), I64Vec3::new(0, 0, -10)] {
            assert!(within(corner(c), eye, 321.0) && !within(corner(c), eye, 319.0), "{c:?}");
        }
    }
}
