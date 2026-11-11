//! How far the world is drawn: chunks farther than the render distance from the camera are
//! hidden, and the camera's far plane follows the distance. Live-tunable, to find what the
//! machine can take.

use api::{CHUNK_SIZE, Tunable, Tunables};
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;

use crate::editing::ChunkMeshes;
use crate::{ChunkMesh, PlayerCamera, coords};

/// The view's tuning.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ViewTuning {
    /// Chunks whose centers are farther than this from the camera, in meters, aren't drawn.
    pub render_distance: f32,
    /// How the frame's light is fitted to the screen. Switching rebuilds the block and sky
    /// shaders, so it stalls for a moment once.
    pub tone_mapper: ToneMapper,
}

impl Default for ViewTuning {
    fn default() -> Self {
        Self { render_distance: 512.0, tone_mapper: ToneMapper::Aces }
    }
}

impl Tunables for ViewTuning {
    fn tunables(&self) -> Vec<Tunable> {
        vec![
            Tunable::new("render_distance", self.render_distance, 32.0, 4096.0),
            Tunable::new("tone_mapper", self.tone_mapper.index() as f32, 0.0, (ToneMapper::ALL.len() - 1) as f32),
        ]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "render_distance" => self.render_distance = value.max(0.0),
            "tone_mapper" => self.tone_mapper = ToneMapper::from_index(value),
            _ => return false,
        }
        true
    }
}

/// The tone mappers to pick from: Bevy's, by name. Saved by place in [`ToneMapper::ALL`], so
/// new ones go at the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToneMapper {
    /// Bevy's default: keeps hues, but pulls bright colors toward white.
    TonyMcMapface,
    /// The film industry's curve, fitted; what the stylised-sky demo used. More contrast and
    /// color in bright skies. The game's default (the user's pick, 2026-11-10).
    Aces,
    AgX,
    BlenderFilmic,
    Reinhard,
    ReinhardLuminance,
    SomewhatBoring,
    /// Colors as the shaders work them out, clipped at full brightness.
    None,
}

impl ToneMapper {
    pub const ALL: [ToneMapper; 8] = [
        ToneMapper::TonyMcMapface,
        ToneMapper::Aces,
        ToneMapper::AgX,
        ToneMapper::BlenderFilmic,
        ToneMapper::Reinhard,
        ToneMapper::ReinhardLuminance,
        ToneMapper::SomewhatBoring,
        ToneMapper::None,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ToneMapper::TonyMcMapface => "Tony McMapface (Bevy's own default)",
            ToneMapper::Aces => "ACES (the default)",
            ToneMapper::AgX => "AgX",
            ToneMapper::BlenderFilmic => "Blender Filmic",
            ToneMapper::Reinhard => "Reinhard",
            ToneMapper::ReinhardLuminance => "Reinhard (by brightness)",
            ToneMapper::SomewhatBoring => "Somewhat boring",
            ToneMapper::None => "none",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&t| t == self).unwrap_or(0)
    }

    pub fn from_index(value: f32) -> Self {
        Self::ALL[(value.round().max(0.0) as usize).min(Self::ALL.len() - 1)]
    }

    pub fn bevy(self) -> Tonemapping {
        match self {
            ToneMapper::TonyMcMapface => Tonemapping::TonyMcMapface,
            ToneMapper::Aces => Tonemapping::AcesFitted,
            ToneMapper::AgX => Tonemapping::AgX,
            ToneMapper::BlenderFilmic => Tonemapping::BlenderFilmic,
            ToneMapper::Reinhard => Tonemapping::Reinhard,
            ToneMapper::ReinhardLuminance => Tonemapping::ReinhardLuminance,
            ToneMapper::SomewhatBoring => Tonemapping::SomewhatBoringDisplayTransform,
            ToneMapper::None => Tonemapping::None,
        }
    }
}

/// Puts the picked tone mapper on the camera.
pub(crate) fn apply_tone_mapper(view: Res<ViewTuning>, mut cameras: Query<&mut Tonemapping, With<PlayerCamera>>) {
    for mut tonemapping in &mut cameras {
        tonemapping.set_if_neq(view.tone_mapper.bevy());
    }
}

/// Ways of drawing the view that can be switched, to measure what each costs (see the bench's
/// `switches`); not saved.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ViewSwitches {
    /// 4× multisample anti-aliasing, Bevy's default; off, edges are jagged but each pixel is
    /// shaded once however many triangles touch it.
    pub msaa: bool,
    /// Depth drawn first in a pass of its own, so the main pass shades only what ends up seen.
    pub depth_prepass: bool,
    /// Neighboring faces that look alike drawn as one bigger quad (see [`crate::greedy`]).
    /// Changing it remeshes every chunk.
    pub merge_faces: bool,
}

impl Default for ViewSwitches {
    fn default() -> Self {
        Self { msaa: true, depth_prepass: false, merge_faces: true }
    }
}

impl ViewSwitches {
    /// What differs from the default, for a perf recording; empty if nothing does.
    pub fn changed(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.msaa {
            out.push("msaa off");
        }
        if self.depth_prepass {
            out.push("depth prepass");
        }
        if !self.merge_faces {
            out.push("faces unmerged");
        }
        out
    }
}

/// Puts the switches on the camera, when they change or the camera is new.
pub(crate) fn apply_view_switches(
    switches: Res<ViewSwitches>,
    mut commands: Commands,
    cameras: Query<Entity, With<PlayerCamera>>,
    new: Query<(), Added<PlayerCamera>>,
) {
    if !(switches.is_changed() || !new.is_empty()) {
        return;
    }
    for camera in &cameras {
        let mut camera = commands.entity(camera);
        camera.insert(if switches.msaa { Msaa::Sample4 } else { Msaa::Off });
        if switches.depth_prepass {
            camera.insert(DepthPrepass);
        } else {
            camera.remove::<DepthPrepass>();
        }
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

    #[test]
    fn tone_mappers_round_trip_through_their_slider() {
        for t in ToneMapper::ALL {
            let mut view = ViewTuning::default();
            assert!(view.set_tunable("tone_mapper", t.index() as f32));
            assert_eq!(view.tone_mapper, t);
        }
        assert_eq!(ToneMapper::from_index(99.0), ToneMapper::None);
        assert_eq!(ToneMapper::from_index(-3.0), ToneMapper::TonyMcMapface);
        assert_eq!(ViewTuning::default().tone_mapper, ToneMapper::Aces);
    }
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
