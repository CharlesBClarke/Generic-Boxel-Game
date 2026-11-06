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
            sun_size: 1.5,
            sun_glow: 0.4,
            fog_start: 0.5,
            fog_end: 1.0,
            fog: 1.0,
            crease: 0.3,
            ridge: 0.12,
            edge_width: 2.0,
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
        Atmosphere {
            sun_direction: sun.extend(self.sun_size.to_radians().cos()),
            sun_color: (linear(self.sun_color) * self.sun_strength).extend(self.sun_glow),
            sky_light: (sky_color * self.sky_light).extend(0.0),
            ground_light: (sky_color * self.ground_light).extend(0.0),
            zenith: zenith.extend(0.0),
            horizon: linear(self.horizon).extend(0.0),
            fog: Vec4::new(fog_start, (self.fog_end * render_distance).max(fog_start + 1.0), self.fog, 0.0),
            edges: Vec4::new(self.crease, self.ridge, self.edge_width, 0.0),
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
            Tunable::new("fog_start", self.fog_start, 0.0, 1.0),
            Tunable::new("fog_end", self.fog_end, 0.05, 1.5),
            Tunable::new("fog", self.fog, 0.0, 1.0),
            Tunable::new("crease", self.crease, 0.0, 1.0),
            Tunable::new("ridge", self.ridge, 0.0, 1.0),
            Tunable::new("edge_width", self.edge_width, 0.0, 8.0),
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
            "fog_start" => self.fog_start = value,
            "fog_end" => self.fog_end = value,
            "fog" => self.fog = value,
            "crease" => self.crease = value,
            "ridge" => self.ridge = value,
            "edge_width" => self.edge_width = value,
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
            .add_systems(PostUpdate, sync_atmosphere);
    }
}

/// One triangle that covers the whole screen, in clip space.
fn spawn_sky(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<SkyMaterial>>) {
    let triangle = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[-1.0, -1.0, 0.0], [3.0, -1.0, 0.0], [-1.0, 3.0, 0.0]]);
    commands.spawn((
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
