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

use crate::coords;
use crate::solid::SolidMaterial;
use crate::view::ViewTuning;

/// The light and air, live-tunable. Colors are sRGB, each channel 0 to 1.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ShadingTuning {
    /// Which way the sun is, in degrees clockwise from north: 90 is east, 180 south.
    pub sun_azimuth: f32,
    /// How high the sun is, in degrees above the horizon.
    pub sun_elevation: f32,
    /// How bright the sun lights a face turned straight at it.
    pub sun_strength: f32,
    pub sun_color: [f32; 3],
    /// Light from the sky on a face looking straight up, whichever way the sun is.
    pub sky_light: f32,
    /// Light from below on a face looking straight down, as if bounced off the ground.
    pub ground_light: f32,
    /// How much the sky's light takes the sky's color: 0 is white, 1 the zenith's color.
    pub sky_tint: f32,
    /// The sky straight up, and at the horizon.
    pub zenith: [f32; 3],
    pub horizon: [f32; 3],
    /// The sun's disc, as its radius in degrees, and the glow around it.
    pub sun_size: f32,
    pub sun_glow: f32,
    /// The sun's flat rays, drawn like a child draws a sun: how many; how far they reach past
    /// the disc, in disc radii; how wide they are at the disc, as a share of the gap between
    /// them (0.45 nearly closes it); how strongly they show.
    pub rays: f32,
    pub ray_length: f32,
    pub ray_width: f32,
    pub ray_strength: f32,
    /// How much shorter some rays are than others (0 all the same), how much they fade toward
    /// their tips (0 not at all), and how fast they turn, in turns a minute.
    pub ray_variation: f32,
    pub ray_fade: f32,
    pub ray_spin: f32,
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
            sun_azimuth: 150.0,
            sun_elevation: 50.0,
            sun_strength: 0.6,
            sun_color: [1.0, 0.96, 0.88],
            sky_light: 0.55,
            ground_light: 0.35,
            sky_tint: 0.3,
            zenith: [0.33, 0.55, 0.9],
            horizon: [0.72, 0.83, 0.95],
            sun_size: 5.0,
            sun_glow: 0.15,
            rays: 12.0,
            ray_length: 1.2,
            ray_width: 0.25,
            ray_strength: 0.8,
            ray_variation: 0.4,
            ray_fade: 0.5,
            ray_spin: 0.0,
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
    /// Toward the sun, Z-up.
    pub fn sun_direction(&self) -> Vec3 {
        let (azimuth, elevation) = (self.sun_azimuth.to_radians(), self.sun_elevation.to_radians());
        Vec3::new(azimuth.sin() * elevation.cos(), azimuth.cos() * elevation.cos(), elevation.sin())
    }

    /// What the shaders get, with the render distance the fog is measured against.
    pub fn atmosphere(&self, render_distance: f32) -> Atmosphere {
        let linear = |c: [f32; 3]| Color::srgb(c[0], c[1], c[2]).to_linear().to_vec3();
        let zenith = linear(self.zenith);
        // The sky's light in the sky's color, kept as bright as white light would be.
        let sky_color = Vec3::ONE.lerp(zenith / zenith.max_element().max(1e-3), self.sky_tint.clamp(0.0, 1.0));
        let sun = coords::to_render(self.sun_direction());
        let fog_start = self.fog_start.min(self.fog_end) * render_distance;
        let on = |b: bool| if b { 1.0 } else { 0.0 };
        Atmosphere {
            sun_direction: sun.extend(self.sun_size.to_radians().cos()),
            sun_color: (linear(self.sun_color) * self.sun_strength).extend(self.sun_glow),
            sky_light: (sky_color * self.sky_light).extend(0.0),
            ground_light: (sky_color * self.ground_light).extend(0.0),
            zenith: zenith.extend(0.0),
            horizon: linear(self.horizon).extend(0.0),
            fog: Vec4::new(fog_start, (self.fog_end * render_distance).max(fog_start + 1.0), self.fog, 0.0),
            edges: Vec4::new(self.crease, self.ridge, self.edge_width, 0.0),
            parts: Vec4::new(on(self.parts.edges), on(self.parts.fog), on(self.parts.sheen), on(self.parts.light)),
            rays: Vec4::new(self.rays, self.ray_length, self.ray_width, self.ray_strength),
            ray_shape: Vec4::new(self.ray_variation, self.ray_fade, self.ray_spin, self.sun_size.to_radians()),
            stored: Vec4::new(self.block_light, self.light_falloff, self.block_falloff, self.min_light),
            stored_on: Vec4::new(on(self.parts.stored), 0.0, 0.0, 0.0),
        }
    }
}

impl Tunables for ShadingTuning {
    fn tunables(&self) -> Vec<Tunable> {
        let mut t = vec![
            Tunable::new("sun_azimuth", self.sun_azimuth, 0.0, 360.0),
            Tunable::new("sun_elevation", self.sun_elevation, -10.0, 90.0),
            Tunable::new("sun_strength", self.sun_strength, 0.0, 2.0),
        ];
        let color = |t: &mut Vec<Tunable>, name: &str, c: [f32; 3]| {
            for (channel, v) in ["r", "g", "b"].iter().zip(c) {
                t.push(Tunable::new(format!("{name}_{channel}"), v, 0.0, 1.0));
            }
        };
        color(&mut t, "sun", self.sun_color);
        t.extend([
            Tunable::new("sky_light", self.sky_light, 0.0, 2.0),
            Tunable::new("ground_light", self.ground_light, 0.0, 2.0),
            Tunable::new("sky_tint", self.sky_tint, 0.0, 1.0),
        ]);
        color(&mut t, "zenith", self.zenith);
        color(&mut t, "horizon", self.horizon);
        t.extend([
            Tunable::new("sun_size", self.sun_size, 0.1, 10.0),
            Tunable::new("sun_glow", self.sun_glow, 0.0, 2.0),
            Tunable::new("rays", self.rays, 0.0, 32.0),
            Tunable::new("ray_length", self.ray_length, 0.0, 6.0),
            Tunable::new("ray_width", self.ray_width, 0.0, 0.45),
            Tunable::new("ray_strength", self.ray_strength, 0.0, 1.0),
            Tunable::new("ray_variation", self.ray_variation, 0.0, 1.0),
            Tunable::new("ray_fade", self.ray_fade, 0.0, 1.0),
            Tunable::new("ray_spin", self.ray_spin, -10.0, 10.0),
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
        ]);
        t
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        let channel = |name: &str, prefix: &str| -> Option<usize> {
            ["r", "g", "b"].iter().position(|c| name.strip_prefix(prefix).and_then(|n| n.strip_prefix('_')) == Some(c))
        };
        if let Some(i) = channel(name, "sun") {
            self.sun_color[i] = value;
            return true;
        }
        if let Some(i) = channel(name, "zenith") {
            self.zenith[i] = value;
            return true;
        }
        if let Some(i) = channel(name, "horizon") {
            self.horizon[i] = value;
            return true;
        }
        match name {
            "sun_azimuth" => self.sun_azimuth = value,
            "sun_elevation" => self.sun_elevation = value,
            "sun_strength" => self.sun_strength = value,
            "sky_light" => self.sky_light = value,
            "ground_light" => self.ground_light = value,
            "sky_tint" => self.sky_tint = value,
            "sun_size" => self.sun_size = value,
            "sun_glow" => self.sun_glow = value,
            "rays" => self.rays = value,
            "ray_length" => self.ray_length = value,
            "ray_width" => self.ray_width = value,
            "ray_strength" => self.ray_strength = value,
            "ray_variation" => self.ray_variation = value,
            "ray_fade" => self.ray_fade = value,
            "ray_spin" => self.ray_spin = value,
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
    /// The sun's light on a face turned to it; then the glow around its disc.
    pub sun_color: Vec4,
    /// Light on faces looking straight up, and straight down, blended between.
    pub sky_light: Vec4,
    pub ground_light: Vec4,
    /// The sky's color straight up and at the horizon.
    pub zenith: Vec4,
    pub horizon: Vec4,
    /// Fog: starting distance, distance where it's thickest (meters), how thick there.
    pub fog: Vec4,
    /// Edges: crease darkness, ridge brightness, width in texels.
    pub edges: Vec4,
    /// Which parts are drawn, 1 or 0: edges, fog, sheen, light.
    pub parts: Vec4,
    /// The sun's rays: how many, how far past the disc in disc radii, how wide, how strong.
    pub rays: Vec4,
    /// How much ray lengths vary, how much rays fade, turns a minute; the disc's radius in radians.
    pub ray_shape: Vec4,
    /// Stored light: block light's strength, sky light's falloff power, block light's, and the
    /// least light.
    pub stored: Vec4,
    /// Stored light on (1) or off (0).
    pub stored_on: Vec4,
}

/// The sky: drawn over the whole screen at the far plane, so only where nothing else is.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
pub struct SkyMaterial {
    #[uniform(0)]
    pub atmosphere: Atmosphere,
}

impl Material for SkyMaterial {
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

/// Registers the sky, the shared shader code and the shading tuning.
pub struct ShadingPlugin;

impl Plugin for ShadingPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "atmosphere.wgsl");
        embedded_asset!(app, "sky.wgsl");
        app.add_plugins(MaterialPlugin::<SkyMaterial>::default())
            .init_resource::<ShadingTuning>()
            .add_systems(Startup, spawn_sky)
            .add_systems(PostUpdate, (sync_atmosphere, show_sky, tag_recordings));
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
fn tag_recordings(tuning: Res<ShadingTuning>, mut tags: ResMut<crate::perf::PerfTags>) {
    let off = tuning.parts.off();
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

/// Puts the current atmosphere into every material that draws with it, where it differs.
fn sync_atmosphere(
    tuning: Res<ShadingTuning>,
    view: Res<ViewTuning>,
    mut solids: ResMut<Assets<SolidMaterial>>,
    mut skies: ResMut<Assets<SkyMaterial>>,
) {
    let atmosphere = tuning.atmosphere(view.render_distance);
    let stale: Vec<_> = solids.iter().filter(|(_, m)| m.atmosphere != atmosphere).map(|(id, _)| id).collect();
    for id in stale {
        if let Some(m) = solids.get_mut(id) {
            m.atmosphere = atmosphere;
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

    #[test]
    fn the_sun_points_where_its_angles_say() {
        let at = |azimuth, elevation| ShadingTuning { sun_azimuth: azimuth, sun_elevation: elevation, ..default() }.sun_direction();
        assert!(near(at(0.0, 0.0), Vec3::Y), "north");
        assert!(near(at(90.0, 0.0), Vec3::X), "east");
        assert!(near(at(180.0, 0.0), -Vec3::Y), "south");
        assert!(near(at(123.0, 90.0), Vec3::Z), "overhead");
        let s = at(150.0, 50.0);
        assert!((s.length() - 1.0).abs() < 1e-5 && (s.z - 50f32.to_radians().sin()).abs() < 1e-5);
    }

    #[test]
    fn the_atmosphere_is_in_render_space_and_scales_fog_with_distance() {
        let tuning = ShadingTuning { sun_azimuth: 90.0, sun_elevation: 0.0, fog_start: 0.5, fog_end: 1.0, ..default() };
        let a = tuning.atmosphere(400.0);
        assert!(near(a.sun_direction.truncate(), coords::to_render(Vec3::X)));
        assert_eq!((a.fog.x, a.fog.y), (200.0, 400.0));
        // White sky light with no tint, the zenith's hue with full tint, as bright either way.
        let plain = ShadingTuning { sky_tint: 0.0, ..default() }.atmosphere(400.0);
        assert!(near(plain.sky_light.truncate(), Vec3::splat(tuning.sky_light)));
        let tinted = ShadingTuning { sky_tint: 1.0, ..default() }.atmosphere(400.0);
        assert!((tinted.sky_light.truncate().max_element() - tuning.sky_light).abs() < 1e-5);
        assert!(tinted.sky_light.z > tinted.sky_light.x, "blue, like the sky");
    }

    #[test]
    fn switched_off_parts_are_named_for_recordings() {
        assert_eq!(Parts::default().off(), "shading all on");
        assert_eq!(Parts { fog: false, sky: false, ..default() }.off(), "shading off: fog, sky");
        let a = ShadingTuning { parts: Parts { edges: false, ..default() }, ..default() }.atmosphere(100.0);
        assert_eq!(a.parts, Vec4::new(0.0, 1.0, 1.0, 1.0));
    }

    #[test]
    fn the_rays_reach_the_shaders_with_the_discs_radius() {
        let t = ShadingTuning { rays: 9.0, ray_length: 2.0, ray_width: 0.3, ray_spin: 1.5, sun_size: 4.0, ..Default::default() };
        let a = t.atmosphere(100.0);
        assert_eq!(a.rays, Vec4::new(9.0, 2.0, 0.3, t.ray_strength));
        assert_eq!(a.ray_shape, Vec4::new(t.ray_variation, t.ray_fade, 1.5, 4f32.to_radians()));
        assert!((a.sun_direction.w - a.ray_shape.w.cos()).abs() < 1e-6, "the disc's edge, both ways");
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
        assert!(!t.set_tunable("sun_a", 1.0) && !t.set_tunable("nope", 1.0));
    }
}
