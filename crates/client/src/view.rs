//! How far the world is drawn: chunks farther than the render distance from the camera are
//! hidden, the camera's far plane follows the distance, and the server keeps that much terrain
//! loaded around the player. Live-tunable, to find what the
//! machine can take.

use api::{CHUNK_SIZE, Tunable, Tunables};
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;

use crate::editing::ChunkMeshes;
use crate::render_scale::{Resolution, Upscale};
use crate::{ChunkMesh, PlayerCamera, coords};

/// The view's tuning.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ViewTuning {
    /// Full detail reaches this far, in meters: chunks farther than this from the camera aren't
    /// drawn, and past it far terrain takes over (see [`crate::far`]).
    pub render_distance: f32,
    /// Far terrain reaches this far, in meters; none if it's no farther than the render
    /// distance.
    pub far_distance: f32,
    /// How many pixels wide far terrain's cells may look before a finer level takes over (see
    /// [`sim::far::Reach::pixels`]): lower is finer, for more triangles.
    pub far_pixels: f32,
    /// How much a build must fill a far cell to keep it (see [`sim::far::Reach::hold`]).
    pub far_hold: f32,
    /// How many levels finer far cells are sampled for their mix of materials (see
    /// [`sim::far::Reach::mix`]).
    pub far_mix: u8,
    /// Pixels a meter-wide thing one meter away covers: from the window's height and the
    /// field of view at rest, kept up to date by [`update_screen_scale`]; not tuned.
    pub screen: f32,
    /// How much of the darkness ambient occlusion puts on a hillside's steps far terrain
    /// takes, so it's as dark as full detail where they meet: 1 is what the AO tuning works
    /// out to, 0 none (see `far_shade` in `solid.wgsl`).
    pub far_crease: f32,
    /// How the frame's light is fitted to the screen. Switching rebuilds the block and sky
    /// shaders, so it stalls for a moment once.
    pub tone_mapper: ToneMapper,
    /// How big the world is drawn, stretched to the window (see [`crate::render_scale`]).
    pub resolution: Resolution,
    /// How a world drawn smaller than the window is stretched to it.
    pub upscale: Upscale,
}

impl Default for ViewTuning {
    fn default() -> Self {
        // Full detail to 256 m: the user's pick for far terrain (2026-11-12).
        Self { render_distance: 256.0, far_distance: 2048.0, far_pixels: sim::far::DEFAULT_PIXELS, far_hold: 1.0, far_mix: 1, screen: sim::far::Reach::default().screen, far_crease: 1.0, tone_mapper: ToneMapper::Aces, resolution: Resolution::Native, upscale: Upscale::Smooth }
    }
}

/// The tuning panel's groups for [`ViewTuning`]'s values.
const VIEW_GROUPS: &[(&str, &[&str])] = &[
];
/// What each of [`ViewTuning`]'s values does, for the tuning panel.
const VIEW_HELP: &[(&str, &str)] = &[
    ("render_distance", "Full detail reaches this far, meters; far terrain takes over past it."),
    ("far_distance", "Far terrain reaches this far, meters, coarser the farther; none if no farther than the render distance."),
    ("far_crease", "How dark far terrain's slopes are next to full detail's, whose steps ambient occlusion darkens: 1 is worked out from the AO tuning, 0 is no darkening. Tune it at the edge of full detail on a hillside."),
    ("far_pixels", "How many pixels wide far terrain's cells may look before a finer level takes over: lower is finer and pops less, but draws more triangles."),
    ("far_hold", "How much of a far cell a build must fill to keep it, in boxel-thick planes right across it: 1 keeps one-boxel walls and floors far away, 0 only builds filling more than half a cell. Rebuilds far terrain."),
    ("far_mix", "How many levels finer far cells are sampled to mix their materials' colors: 0 off, 1 on (far terrain takes about four times as long to build). Rebuilds far terrain."),
    ("tone_mapper", "How the frame's light is fitted to the screen. Switching rebuilds the shaders, a moment's stall."),
    ("resolution", "How big the world is drawn, stretched to fill the window; lower buys frame rate. Menus and text stay sharp."),
    ("upscale", "How a world drawn smaller than the window is stretched: smooth, or pixelated."),
];

impl Tunables for ViewTuning {
    fn tunables(&self) -> Vec<Tunable> {
        api::tuning::described(vec![
            Tunable::new("render_distance", self.render_distance, 32.0, 4096.0),
            Tunable::new("far_distance", self.far_distance, 0.0, 8192.0),
            Tunable::new("far_crease", self.far_crease, 0.0, 2.0),
            Tunable::new("far_pixels", self.far_pixels, *sim::far::PIXELS.start(), *sim::far::PIXELS.end()),
            Tunable::new("far_hold", self.far_hold, 0.0, 4.0),
            Tunable::new("far_mix", self.far_mix as f32, 0.0, sim::far::MAX_MIX as f32),
            Tunable::new("tone_mapper", self.tone_mapper.index() as f32, 0.0, (ToneMapper::ALL.len() - 1) as f32),
            Tunable::new("resolution", self.resolution.index() as f32, 0.0, (Resolution::ALL.len() - 1) as f32),
            Tunable::new("upscale", self.upscale.index() as f32, 0.0, (Upscale::ALL.len() - 1) as f32),
        ], VIEW_GROUPS, VIEW_HELP)
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "render_distance" => self.render_distance = value.max(0.0),
            "far_distance" => self.far_distance = value.max(0.0),
            "far_crease" => self.far_crease = value.clamp(0.0, 2.0),
            "far_pixels" => self.far_pixels = value.clamp(*sim::far::PIXELS.start(), *sim::far::PIXELS.end()),
            "far_hold" => self.far_hold = value.clamp(0.0, 4.0),
            "far_mix" => self.far_mix = value.round().clamp(0.0, sim::far::MAX_MIX as f32) as u8,
            "tone_mapper" => self.tone_mapper = ToneMapper::from_index(value),
            "resolution" => self.resolution = Resolution::from_index(value),
            "upscale" => self.upscale = Upscale::from_index(value),
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
    /// 4× multisample anti-aliasing. Off by default (Bevy's is on): it cost ~11 ms at 720p on
    /// the Iris Xe and matters little for the blocky look; off, edges are jagged but each pixel
    /// is shaded once however many triangles touch it.
    pub msaa: bool,
    /// Depth drawn first in a pass of its own, so the main pass shades only what ends up seen.
    pub depth_prepass: bool,
    /// Neighboring faces that look alike drawn as one bigger quad (see [`crate::greedy`]).
    /// Changing it remeshes every chunk.
    pub merge_faces: bool,
    /// Terrain drawn coarsely past the render distance, out to the far distance (see
    /// [`crate::far`]).
    pub far_terrain: bool,
}

impl Default for ViewSwitches {
    fn default() -> Self {
        Self { msaa: false, depth_prepass: false, merge_faces: true, far_terrain: true }
    }
}

impl ViewSwitches {
    /// What differs from the default, for a perf recording; empty if nothing does.
    pub fn changed(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.msaa {
            out.push("msaa on");
        }
        if self.depth_prepass {
            out.push("depth prepass");
        }
        if !self.merge_faces {
            out.push("faces unmerged");
        }
        if !self.far_terrain {
            out.push("far terrain off");
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
    /// Chunks with geometry drawn, far ones included.
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

/// Tells the server how far the view reaches whenever that changes, so it keeps that much
/// terrain loaded and builds far terrain out to the far distance.
pub(crate) fn send_view_distance(
    view: Res<ViewTuning>,
    switches: Res<ViewSwitches>,
    mut sent: Local<Option<sim::far::Reach>>,
    mut link: ResMut<crate::world::Link>,
    mut world: ResMut<crate::world::ClientWorld>,
) {
    let reach = crate::far::reach(&view, &switches)
        .unwrap_or(sim::far::Reach { detail: view.render_distance, far: 0.0, ..crate::far::reach_of(&view) });
    if *sent != Some(reach) {
        *sent = Some(reach);
        link.command(protocol::Command::ViewDistance(reach), &mut world);
    }
}

/// Keeps [`ViewTuning::screen`] up to date with the window's height and the field of view at
/// rest (not the widening with speed, which would rebuild what far terrain wants as the
/// player runs). Changes under 2% are let go.
pub(crate) fn update_screen_scale(
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    feel: Res<crate::feel::CameraFeel>,
    mut view: ResMut<ViewTuning>,
) {
    let screen = sim::far::Reach::screen_scale(window.physical_height() as f32, feel.fov);
    if (screen / view.screen.max(1.0) - 1.0).abs() > 0.02 {
        view.screen = screen;
    }
}

/// How far the view reaches, for the far plane and the fog: the far distance if far terrain
/// is on, or else the render distance.
pub fn view_reach(view: &ViewTuning, switches: &ViewSwitches) -> f32 {
    crate::far::reach(view, switches).map_or(view.render_distance, |r| r.far)
}

/// Shows the chunks to draw and hides the rest, and sets the far plane. With far terrain on,
/// the streamed chunks that show a column's surface (see [`sim::tops::surface_rows`]) are drawn
/// where the quadtree picked the column (see [`crate::far::pick_far`]), far terrain standing
/// in for them elsewhere; the streamed chunks under and over those are always drawn, so a
/// tunnel or a tower shows whatever the surface is drawn as. Other chunks, and all of them
/// with far terrain off, within the render distance.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_render_distance(
    view: Res<ViewTuning>,
    switches: Res<ViewSwitches>,
    meshes: Option<Res<ChunkMeshes>>,
    far: Res<crate::far::FarTerrain>,
    bg: Res<crate::meshing::BackgroundMesher>,
    mut stats: ResMut<ViewStats>,
    mut camera: Single<(&Transform, &mut Projection), With<PlayerCamera>>,
    mut chunks: Query<(&ChunkMesh, &Transform, &mut Visibility), Without<PlayerCamera>>,
) {
    let (eye, projection) = &mut *camera;
    let eye = eye.translation;
    if let Some(p) = crate::shadows::perspective_mut(projection) {
        let far = view_reach(&view, &switches) + CHUNK_SIZE as f32 * 2.0;
        if (p.far - far).abs() > 0.5 {
            p.far = far;
        }
    }
    let mut drawn = ViewStats::default();
    for (chunk, transform, mut visibility) in &mut chunks {
        let pos = chunk.0;
        let show = if far.active && bg.is_streamed(pos) {
            far.ordinary.contains(&(pos.0.x, pos.0.y)) || !bg.shows_surface(pos)
        } else {
            within(transform.translation, eye, view.render_distance)
        };
        visibility.set_if_neq(if show { Visibility::Inherited } else { Visibility::Hidden });
        if show {
            drawn.drawn += 1;
            drawn.triangles += meshes.as_ref().and_then(|m| m.quads.get(&pos)).copied().unwrap_or(0) * 2;
        }
    }
    // Far terrain counts too, so the readouts and the bench see everything drawn.
    drawn.drawn += far.stats.chunks;
    drawn.triangles += far.stats.triangles;
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
