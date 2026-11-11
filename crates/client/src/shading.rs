//! Light and air: a sun and the sky's light on every block face, the lines along creases and
//! ridges, fog that fades the far world into the sky, and the sky itself. All of it is worked
//! out in the shaders from one [`Atmosphere`], built from the live [`ShadingTuning`] and kept
//! in every block material and the sky's, so changing any of it never remeshes.

use api::{Tunable, Tunables};
use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, CompareFunction, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::asset::RenderAssetUsages;
use bevy::shader::ShaderRef;

use crate::RenderOrigin;
use crate::coords;
use crate::heights::Heights;
use crate::shadows::{ShadowTuning, Sun};
use crate::sky::{DaySky, SkyTuning};
use crate::solid::SolidMaterial;
use crate::view::ViewTuning;

/// The light and air, live-tunable. The sky's colors and the sun's come from the time of day
/// (see [`crate::sky`]); these scale and shape them.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ShadingTuning {
    /// The sun's elevation at noon, in degrees. At 58, 10:00 puts the sun where it sat before
    /// it moved (azimuth 150, about 50 up).
    pub sun_arc: f32,
    /// How bright the sun lights a face turned straight at it, at its highest.
    pub sun_strength: f32,
    /// Light from the sky on a face looking straight up at midday, whichever way the sun is.
    pub sky_light: f32,
    /// Light from below on a face looking straight down, as if bounced off the ground.
    pub ground_light: f32,
    /// How much the sky's light takes the sky's color (0 is white), and the ground's the
    /// horizon's.
    pub sky_tint: f32,
    /// The sky's colors, brighter or darker than the palette.
    pub sky_brightness: f32,
    /// How much warmer than the palette a low sun is; 0 is the palette as it is.
    pub warmth: f32,
    /// The sun's disc, as its radius in degrees, and how strong the glow around it is.
    pub sun_size: f32,
    pub sun_glow: f32,
    /// How many hours the moon runs behind the sun, and how high it gets.
    pub moon_lag: f32,
    pub moon_arc: f32,
    /// How bright the moon lights a face turned to it, at night. It casts no shadows.
    pub moon_strength: f32,
    /// The moon's disc, as its radius in degrees.
    pub moon_size: f32,
    /// How bright the stars are.
    pub stars: f32,
    /// Sky light at deepest night, so the open world stays faintly readable.
    pub night_light: f32,
    /// Draw this hour (0 to 24) instead of the world's time, to compare looks; below 0
    /// follows the world. The bench holds it, so every build sees the same sky.
    pub hold_hour: f32,
    /// Where fog starts and where it's thickest, as shares of the render distance.
    pub fog_start: f32,
    pub fog_end: f32,
    /// How thick the fog gets at its thickest: 1 hides everything there.
    pub fog: f32,
    /// How much darker a texel right against an inside crease gets.
    pub crease: f32,
    /// How much lighter a texel right on an outside ridge gets.
    pub ridge: f32,
    /// How many texels wide the crease and ridge lines are, fading off across them.
    pub edge_width: f32,
    /// How brightly light from blocks that give light (stored light's red, green and blue)
    /// lights a face, at full level.
    pub block_light: f32,
    /// How sky light fades with its stored level: brightness is the level's share of full to
    /// this power, so above 1 dims faster in from an opening.
    pub light_falloff: f32,
    /// The same for block light. Stored block light already dims by a share each boxel, so 1
    /// shows it as stored; above 1 dims faster away from a lamp.
    pub block_falloff: f32,
    /// Light every face gets however dark it is, so caves stay faintly readable.
    pub min_light: f32,
    /// Which parts are drawn, to measure what each costs. Not saved.
    pub parts: Parts,
}

/// Parts of the shading that can be switched off to see what they cost.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parts {
    /// Crease and ridge lines.
    pub edges: bool,
    pub fog: bool,
    pub sheen: bool,
    /// Sun and sky light; off, faces show their plain colors.
    pub light: bool,
    pub sky: bool,
    /// Stored light: off, sun and sky light every face as if under the open sky.
    pub stored: bool,
}

impl Default for Parts {
    fn default() -> Self {
        Self { edges: true, fog: true, sheen: true, light: true, sky: true, stored: true }
    }
}

impl Parts {
    /// The parts switched off, for a perf recording: `-` with all on.
    pub fn off(&self) -> String {
        let all = [
            ("edges", self.edges),
            ("fog", self.fog),
            ("sheen", self.sheen),
            ("light", self.light),
            ("sky", self.sky),
            ("stored light", self.stored),
        ];
        let off: Vec<&str> = all.iter().filter(|(_, on)| !on).map(|(name, _)| *name).collect();
        if off.is_empty() { "shading all on".into() } else { format!("shading off: {}", off.join(", ")) }
    }
}

impl Default for ShadingTuning {
    fn default() -> Self {
        Self {
            sun_arc: 58.0,
            sun_strength: 0.6,
            sky_light: 0.55,
            ground_light: 0.35,
            sky_tint: 0.3,
            sky_brightness: 1.0,
            warmth: 1.0,
            sun_size: 0.6,
            sun_glow: 1.0,
            moon_lag: 12.0,
            moon_arc: 48.0,
            moon_strength: 0.15,
            moon_size: 2.0,
            stars: 1.0,
            night_light: 0.05,
            hold_hour: -1.0,
            fog_start: 0.5,
            fog_end: 1.0,
            fog: 1.0,
            crease: 0.3,
            ridge: 0.12,
            edge_width: 2.0,
            block_light: 1.0,
            light_falloff: 1.4,
            block_falloff: 1.0,
            min_light: 0.03,
            parts: Parts::default(),
        }
    }
}

impl ShadingTuning {
    /// What [`DaySky::at`] needs.
    pub fn sky(&self) -> SkyTuning {
        SkyTuning {
            sun_arc: self.sun_arc,
            sun_strength: self.sun_strength,
            sky_light: self.sky_light,
            ground_light: self.ground_light,
            sky_tint: self.sky_tint,
            moon_lag: self.moon_lag,
            moon_arc: self.moon_arc,
            moon_strength: self.moon_strength,
            night_light: self.night_light,
            warmth: self.warmth,
            sky_brightness: self.sky_brightness,
        }
    }

    /// The hour to draw: the held one, or the world's.
    pub fn hour(&self, world: f64) -> f64 {
        if self.hold_hour >= 0.0 { self.hold_hour as f64 } else { world }
    }

    /// What the shaders get for the sky at one moment, with the render distance the fog is
    /// measured against.
    pub fn atmosphere(&self, sky: &DaySky, render_distance: f32) -> Atmosphere {
        let render = |v: Vec3| coords::to_render(v);
        let fog_start = self.fog_start.min(self.fog_end) * render_distance;
        let on = |b: bool| if b { 1.0 } else { 0.0 };
        Atmosphere {
            sun_direction: render(sky.sun()).extend(self.sun_size.to_radians().cos()),
            sun_color: sky.sun_light.extend(self.sun_glow),
            sun_hue: sky.sun_hue.extend(0.0),
            moon_direction: render(sky.moon).extend(self.moon_size.to_radians().cos()),
            moon_color: sky.moon_light.extend(sky.night),
            sky_light: sky.sky_light.extend(0.0),
            ground_light: sky.ground_light.extend(0.0),
            zenith: sky.zenith.extend(0.0),
            horizon: sky.horizon.extend(0.0),
            // The stars turn once a day around the pole.
            night: Vec4::new(sky.stars * self.stars, (sky.hours as f32 / 24.0) * std::f32::consts::TAU, 0.0, 0.0),
            fog: Vec4::new(fog_start, (self.fog_end * render_distance).max(fog_start + 1.0), self.fog, 0.0),
            edges: Vec4::new(self.crease, self.ridge, self.edge_width, 0.0),
            parts: Vec4::new(on(self.parts.edges), on(self.parts.fog), on(self.parts.sheen), on(self.parts.light)),
            stored: Vec4::new(self.block_light, self.light_falloff, self.block_falloff, self.min_light),
            stored_on: Vec4::new(on(self.parts.stored), 0.0, 0.0, 0.0),
            shadow: Vec4::ZERO,
            far: Vec4::ZERO,
            heights: Vec4::ZERO,
            fine_heights: Vec4::ZERO,
        }
    }
}

impl Tunables for ShadingTuning {
    fn tunables(&self) -> Vec<Tunable> {
        vec![
            Tunable::new("hold_hour", self.hold_hour, -1.0, 24.0),
            Tunable::new("sun_arc", self.sun_arc, 5.0, 90.0),
            Tunable::new("sun_strength", self.sun_strength, 0.0, 2.0),
            Tunable::new("sky_light", self.sky_light, 0.0, 2.0),
            Tunable::new("ground_light", self.ground_light, 0.0, 2.0),
            Tunable::new("sky_tint", self.sky_tint, 0.0, 1.0),
            Tunable::new("sky_brightness", self.sky_brightness, 0.0, 3.0),
            Tunable::new("warmth", self.warmth, 0.0, 2.0),
            Tunable::new("sun_size", self.sun_size, 0.1, 5.0),
            Tunable::new("sun_glow", self.sun_glow, 0.0, 3.0),
            Tunable::new("moon_lag", self.moon_lag, 0.0, 24.0),
            Tunable::new("moon_arc", self.moon_arc, 5.0, 90.0),
            Tunable::new("moon_strength", self.moon_strength, 0.0, 1.0),
            Tunable::new("moon_size", self.moon_size, 0.1, 5.0),
            Tunable::new("stars", self.stars, 0.0, 3.0),
            Tunable::new("night_light", self.night_light, 0.0, 0.5),
            Tunable::new("fog_start", self.fog_start, 0.0, 1.0),
            Tunable::new("fog_end", self.fog_end, 0.05, 1.5),
            Tunable::new("fog", self.fog, 0.0, 1.0),
            Tunable::new("crease", self.crease, 0.0, 1.0),
            Tunable::new("ridge", self.ridge, 0.0, 1.0),
            Tunable::new("edge_width", self.edge_width, 0.0, 8.0),
            Tunable::new("block_light", self.block_light, 0.0, 3.0),
            Tunable::new("light_falloff", self.light_falloff, 0.5, 4.0),
            Tunable::new("block_falloff", self.block_falloff, 0.25, 4.0),
            Tunable::new("min_light", self.min_light, 0.0, 0.3),
        ]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "hold_hour" => self.hold_hour = value,
            "sun_arc" => self.sun_arc = value,
            "sun_strength" => self.sun_strength = value,
            "sky_light" => self.sky_light = value,
            "ground_light" => self.ground_light = value,
            "sky_tint" => self.sky_tint = value,
            "sky_brightness" => self.sky_brightness = value,
            "warmth" => self.warmth = value,
            "sun_size" => self.sun_size = value,
            "sun_glow" => self.sun_glow = value,
            "moon_lag" => self.moon_lag = value,
            "moon_arc" => self.moon_arc = value,
            "moon_strength" => self.moon_strength = value,
            "moon_size" => self.moon_size = value,
            "stars" => self.stars = value,
            "night_light" => self.night_light = value,
            "fog_start" => self.fog_start = value,
            "fog_end" => self.fog_end = value,
            "fog" => self.fog = value,
            "crease" => self.crease = value,
            "ridge" => self.ridge = value,
            "edge_width" => self.edge_width = value,
            "block_light" => self.block_light = value,
            "light_falloff" => self.light_falloff = value,
            "block_falloff" => self.block_falloff = value,
            "min_light" => self.min_light = value,
            _ => return false,
        }
        true
    }
}

/// Everything the shaders know about light and air, in render space (Y up) and linear color.
/// Matches `Atmosphere` in `atmosphere.wgsl`.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct Atmosphere {
    /// Toward the sun; then the cosine of the sun disc's radius.
    pub sun_direction: Vec4,
    /// The sun's light on a face turned to it (zero once it's down); then how strong the glow
    /// around its disc is.
    pub sun_color: Vec4,
    /// The sun's hue at full brightness, for its disc and glow.
    pub sun_hue: Vec4,
    /// Toward the moon; then the cosine of its disc's radius.
    pub moon_direction: Vec4,
    /// The moon's light on a face turned to it (it casts no shadows); then how much it shows in
    /// the sky, 0 by day to 1 by night.
    pub moon_color: Vec4,
    /// Light on faces looking straight up, and straight down, blended between.
    pub sky_light: Vec4,
    pub ground_light: Vec4,
    /// The sky's color straight up and at the horizon.
    pub zenith: Vec4,
    pub horizon: Vec4,
    /// The night sky: how bright the stars are, and how far they've turned (radians).
    pub night: Vec4,
    /// Fog: starting distance, distance where it's thickest (meters), how thick there.
    pub fog: Vec4,
    /// Edges: crease darkness, ridge brightness, width in texels.
    pub edges: Vec4,
    /// Which parts are drawn, 1 or 0: edges, fog, sheen, light.
    pub parts: Vec4,
    /// Stored light: block light's strength, sky light's falloff power, block light's, and the
    /// least light.
    pub stored: Vec4,
    /// Stored light on (1) or off (0).
    pub stored_on: Vec4,
    /// Shadows: how far the maps reach, where they start fading into the far shadow (meters
    /// from the camera, along its view), the far shadow on (1) or off (0), the maps tinted by
    /// cascade (1) or not.
    pub shadow: Vec4,
    /// The far shadow: most height-map reads toward the sun, how far it looks, how soft, how far
    /// a column must rise above a ray to shade it.
    pub far: Vec4,
    /// Where the coarse height map lies, relative to the render origin: X and north of texel
    /// (0, 0)'s corner, the height its texels are measured from (render Y), and columns per
    /// texel.
    pub heights: Vec4,
    /// The same for the fine height map, a texel to each column.
    pub fine_heights: Vec4,
}

/// The sky: drawn over the whole screen at the far plane, so only where nothing else is.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
pub struct SkyMaterial {
    #[uniform(0)]
    pub atmosphere: Atmosphere,
}

impl Material for SkyMaterial {
    // A triangle already in clip space: it has no place in the world to cast a shadow from.
    fn enable_shadows() -> bool {
        false
    }

    fn enable_prepass() -> bool {
        false
    }

    fn vertex_shader() -> ShaderRef {
        shader("sky.wgsl")
    }

    fn fragment_shader() -> ShaderRef {
        shader("sky.wgsl")
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?];
        // At the far plane, behind everything; drawn wherever nothing nearer was.
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_write_enabled = false;
            depth.depth_compare = CompareFunction::GreaterEqual;
        }
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

fn shader(file: &str) -> ShaderRef {
    let path = match file {
        "sky.wgsl" => embedded_path!("sky.wgsl"),
        _ => unreachable!("an embedded shader"),
    };
    ShaderRef::Path(AssetPath::from_path_buf(path).with_source("embedded"))
}

/// Where the atmosphere is put into the materials; whatever moves the sun runs before it.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShadingSet;

/// Registers the sky, the shared shader code and the shading tuning.
pub struct ShadingPlugin;

impl Plugin for ShadingPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "atmosphere.wgsl");
        embedded_asset!(app, "sky.wgsl");
        app.add_plugins(MaterialPlugin::<SkyMaterial>::default())
            .init_resource::<ShadingTuning>()
            .add_systems(Startup, spawn_sky)
            .add_systems(PostUpdate, ((sync_atmosphere, show_sky).in_set(ShadingSet), tag_recordings));
    }
}

/// Marks the sky.
#[derive(Component)]
struct Sky;

fn show_sky(tuning: Res<ShadingTuning>, mut sky: Query<&mut Visibility, With<Sky>>) {
    for mut visibility in &mut sky {
        visibility.set_if_neq(if tuning.parts.sky { Visibility::Inherited } else { Visibility::Hidden });
    }
}

/// Says in each perf recording which parts were off.
fn tag_recordings(
    tuning: Res<ShadingTuning>,
    shadows: Res<ShadowTuning>,
    view: Res<crate::view::ViewSwitches>,
    mut tags: ResMut<crate::perf::PerfTags>,
) {
    let mut off = tuning.parts.off();
    let shadows_off = shadows.switches.off();
    if !shadows_off.is_empty() {
        off += &format!("; off: {}", shadows_off.join(", "));
    }
    let view = view.changed();
    if !view.is_empty() {
        off += &format!("; view: {}", view.join(", "));
    }
    if tags.0 != off {
        tags.0 = off;
    }
}

/// One triangle that covers the whole screen, in clip space.
fn spawn_sky(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<SkyMaterial>>) {
    let triangle = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[-1.0, -1.0, 0.0], [3.0, -1.0, 0.0], [-1.0, 3.0, 0.0]]);
    commands.spawn((
        Sky,
        Mesh3d(meshes.add(triangle)),
        MeshMaterial3d(materials.add(SkyMaterial::default())),
        Transform::default(),
        NoFrustumCulling,
    ));
}

/// The shadows' part of the atmosphere: the tuning, and where the height map lies relative to
/// the render origin.
pub fn shadow_atmosphere(a: &mut Atmosphere, shadows: &ShadowTuning, heights: Option<crate::heights::Maps>, origin: api::I64Vec3) {
    let on = |b: bool| if b { 1.0 } else { 0.0 };
    let distance = shadows.distance.max(1.0);
    let far = shadows.switches.far && heights.is_some();
    a.shadow = Vec4::new(distance, distance * (1.0 - shadows.fade.clamp(0.0, 1.0)), on(far), on(shadows.switches.show_cascades));
    a.far = Vec4::new(shadows.far_reads.max(1.0), shadows.far_reach, shadows.far_softness, shadows.far_bias);
    let place = |p: crate::heights::Placement| {
        Vec4::new((p.corner.0 - origin.x) as f32, (p.corner.1 - origin.y) as f32, (p.base - origin.z) as f32, p.scale as f32)
    };
    if let Some(maps) = heights {
        a.heights = place(maps.coarse);
        a.fine_heights = place(maps.fine);
    }
}

/// Puts the current atmosphere into every material that draws with it, where it differs, and
/// the latest height map into the block materials.
#[allow(clippy::too_many_arguments)]
fn sync_atmosphere(
    tuning: Res<ShadingTuning>,
    sun: Res<Sun>,
    shadows: Res<ShadowTuning>,
    heights: Res<Heights>,
    origin: Res<RenderOrigin>,
    view: Res<ViewTuning>,
    mut solids: ResMut<Assets<SolidMaterial>>,
    mut skies: ResMut<Assets<SkyMaterial>>,
) {
    let mut atmosphere = tuning.atmosphere(&sun.sky, view.render_distance);
    shadow_atmosphere(&mut atmosphere, &shadows, heights.placed, origin.cell);
    let image = heights.image.clone();
    let stale: Vec<_> = solids
        .iter()
        .filter(|(_, m)| m.atmosphere != atmosphere || image.as_ref().is_some_and(|i| *i != m.heights))
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(m) = solids.get_mut(id) {
            m.atmosphere = atmosphere;
            if let Some(image) = &image {
                m.heights = image.clone();
            }
        }
    }
    let stale: Vec<_> = skies.iter().filter(|(_, m)| m.atmosphere != atmosphere).map(|(id, _)| id).collect();
    for id in stale {
        if let Some(m) = skies.get_mut(id) {
            m.atmosphere = atmosphere;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-5
    }

    fn sky(hours: f64, tuning: &ShadingTuning) -> DaySky {
        DaySky::at(hours, &tuning.sky(), 0.0)
    }

    #[test]
    fn the_atmosphere_is_in_render_space_and_scales_fog_with_distance() {
        let tuning = ShadingTuning { fog_start: 0.5, fog_end: 1.0, ..default() };
        // At 6:00 the sun is on the eastern horizon.
        let a = tuning.atmosphere(&sky(6.0, &tuning), 400.0);
        assert!(near(a.sun_direction.truncate(), coords::to_render(Vec3::X)));
        assert_eq!((a.fog.x, a.fog.y), (200.0, 400.0));
        // White sky light with no tint at noon (with the sun at the palette's highest row), the
        // zenith's hue with full tint.
        let plain = ShadingTuning { sky_tint: 0.0, sun_arc: 70.0, ..default() };
        let a = plain.atmosphere(&sky(12.0, &plain), 400.0);
        assert!(near(a.sky_light.truncate(), Vec3::splat(plain.sky_light)));
        let tinted = ShadingTuning { sky_tint: 1.0, sun_arc: 70.0, ..default() };
        let a = tinted.atmosphere(&sky(12.0, &tinted), 400.0);
        assert!((a.sky_light.truncate().max_element() - tinted.sky_light).abs() < 1e-5);
        assert!(a.sky_light.z > a.sky_light.x, "blue, like the sky");
    }

    #[test]
    fn a_held_hour_wins_over_the_worlds() {
        let t = ShadingTuning { hold_hour: 21.5, ..default() };
        assert_eq!(t.hour(9.0), 21.5);
        assert_eq!(ShadingTuning::default().hour(9.0), 9.0);
    }

    #[test]
    fn the_discs_reach_the_shaders_as_cosines() {
        let t = ShadingTuning { sun_size: 4.0, moon_size: 2.0, ..default() };
        let a = t.atmosphere(&sky(0.0, &t), 100.0);
        assert!((a.sun_direction.w - 4f32.to_radians().cos()).abs() < 1e-6);
        assert!((a.moon_direction.w - 2f32.to_radians().cos()).abs() < 1e-6);
        // Midnight: no sun light, the moon showing.
        assert_eq!(a.sun_color.truncate(), Vec3::ZERO);
        assert_eq!(a.moon_color.w, 1.0);
    }

    #[test]
    fn switched_off_parts_are_named_for_recordings() {
        assert_eq!(Parts::default().off(), "shading all on");
        assert_eq!(Parts { fog: false, sky: false, ..default() }.off(), "shading off: fog, sky");
        let t = ShadingTuning { parts: Parts { edges: false, ..default() }, ..default() };
        let a = t.atmosphere(&sky(12.0, &t), 100.0);
        assert_eq!(a.parts, Vec4::new(0.0, 1.0, 1.0, 1.0));
    }

    #[test]
    fn every_value_round_trips_through_its_slider() {
        let mut t = ShadingTuning::default();
        for (i, slider) in ShadingTuning::default().tunables().iter().enumerate() {
            let value = slider.min + (slider.max - slider.min) * (i as f32 + 1.0) / 40.0;
            assert!(t.set_tunable(&slider.name, value), "{}", slider.name);
            let back = t.tunables().into_iter().find(|s| s.name == slider.name).unwrap().value;
            assert_eq!(back, value, "{}", slider.name);
        }
        assert!(!t.set_tunable("rays", 1.0) && !t.set_tunable("nope", 1.0));
    }
}
