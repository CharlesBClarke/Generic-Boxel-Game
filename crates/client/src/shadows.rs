//! Sun shadows. Near the camera, Bevy's cascaded shadow maps: a `DirectionalLight` that follows
//! the sun, whose brightness nothing uses, only its shadow maps, read in `solid.wgsl`. They are
//! drawn from the real triangles, so every shape casts a shadow sharp to the texel. Past the
//! maps, a coarse shadow from a height map of the terrain (see [`crate::heights`]) takes over,
//! so distant hills still shade valleys. Live-tunable, with a sun that can move for testing.

use api::{Tunable, Tunables};
use bevy::light::{CascadeShadowConfig, CascadeShadowConfigBuilder, DirectionalLightShadowMap, ShadowFilteringMethod};
use bevy::prelude::*;

use crate::coords;
use crate::shading::ShadingTuning;

/// How shadow edges are smoothed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    /// Hardware 2×2: crisp and cheap.
    Crisp,
    /// A fixed blur over a few texels.
    Gaussian,
    /// A noisy blur that changes every frame; made for temporal anti-aliasing.
    Temporal,
}

impl Filter {
    pub const ALL: [Filter; 3] = [Filter::Crisp, Filter::Gaussian, Filter::Temporal];

    pub fn label(self) -> &'static str {
        match self {
            Filter::Crisp => "crisp (2×2)",
            Filter::Gaussian => "soft (Gaussian)",
            Filter::Temporal => "temporal (noisy)",
        }
    }

    fn index(self) -> f32 {
        Filter::ALL.iter().position(|&f| f == self).unwrap_or(0) as f32
    }

    fn from_index(value: f32) -> Filter {
        Filter::ALL[(value.round().max(0.0) as usize).min(Filter::ALL.len() - 1)]
    }

    fn method(self) -> ShadowFilteringMethod {
        match self {
            Filter::Crisp => ShadowFilteringMethod::Hardware2x2,
            Filter::Gaussian => ShadowFilteringMethod::Gaussian,
            Filter::Temporal => ShadowFilteringMethod::Temporal,
        }
    }
}

/// The shadows, live-tunable. Distances are in meters.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ShadowTuning {
    /// How far from the camera the shadow maps reach.
    pub distance: f32,
    /// How many shadow maps share that distance, each farther one coarser.
    pub cascades: f32,
    /// Where the first, sharpest map ends.
    pub first_cascade: f32,
    /// Each map's width and height in texels, a power of two.
    pub map_size: f32,
    /// How far a shadow is pushed back toward the sun, and out along the surface's normal (in
    /// map texels), so a face doesn't shadow itself in stripes.
    pub depth_bias: f32,
    pub normal_bias: f32,
    pub filter: Filter,
    /// The last share of `distance` over which the maps fade into the far shadow.
    pub fade: f32,
    /// The far shadow: how many steps it takes toward the sun, how far it looks (meters), how
    /// soft its edges are (higher is softer), and how far a column must rise above a ray
    /// before it shades it, so flat ground under a low sun doesn't shade itself.
    pub far_steps: f32,
    pub far_reach: f32,
    pub far_softness: f32,
    pub far_bias: f32,
    /// The moving sun: seconds for a whole day, east to west, and how low it starts and ends.
    /// It moves in steps of `sun_step` degrees, so shadows don't crawl every frame.
    pub day_seconds: f32,
    pub sun_low: f32,
    pub sun_step: f32,
    /// Switches, not saved.
    pub switches: Switches,
}

/// Parts of the shadows that can be switched off, to see what they cost, and the moving sun.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Switches {
    /// The shadow maps near the camera.
    pub maps: bool,
    /// The height-map shadow past them.
    pub far: bool,
    /// The sun moves across the sky.
    pub sun_moves: bool,
    /// Each map tinted, to see where it ends.
    pub show_cascades: bool,
}

impl Default for Switches {
    fn default() -> Self {
        Self { maps: true, far: true, sun_moves: false, show_cascades: false }
    }
}

impl Switches {
    /// What's off, for a perf recording; empty with everything on.
    pub fn off(&self) -> Vec<&'static str> {
        let mut off = Vec::new();
        if !self.maps {
            off.push("shadow maps");
        }
        if !self.far {
            off.push("far shadow");
        }
        off
    }
}

impl Default for ShadowTuning {
    fn default() -> Self {
        Self {
            distance: 128.0,
            cascades: 3.0,
            first_cascade: 16.0,
            map_size: 2048.0,
            depth_bias: 0.02,
            normal_bias: 1.8,
            filter: Filter::Crisp,
            fade: 0.2,
            far_steps: 48.0,
            far_reach: 512.0,
            far_softness: 4.0,
            far_bias: 0.5,
            day_seconds: 120.0,
            sun_low: 5.0,
            sun_step: 0.05,
            switches: Switches::default(),
        }
    }
}

impl ShadowTuning {
    /// The cascades as Bevy wants them, kept within what it accepts.
    pub fn cascade_config(&self) -> CascadeShadowConfig {
        let maximum_distance = self.distance.max(1.0);
        let num_cascades = (self.cascades.round() as usize).clamp(1, 4);
        let first_cascade_far_bound = self.first_cascade.clamp(0.5, maximum_distance);
        CascadeShadowConfigBuilder {
            num_cascades,
            minimum_distance: 0.1,
            maximum_distance,
            first_cascade_far_bound,
            overlap_proportion: 0.2,
        }
        .build()
    }

    /// The map size as a power of two, from 512 to 8192.
    pub fn map_texels(&self) -> usize {
        (self.map_size.clamp(512.0, 8192.0) as usize).next_power_of_two().min(8192)
    }
}

impl Tunables for ShadowTuning {
    fn tunables(&self) -> Vec<Tunable> {
        vec![
            Tunable::new("distance", self.distance, 16.0, 512.0),
            Tunable::new("cascades", self.cascades, 1.0, 4.0),
            Tunable::new("first_cascade", self.first_cascade, 2.0, 64.0),
            Tunable::new("map_size", self.map_size, 512.0, 4096.0),
            Tunable::new("depth_bias", self.depth_bias, 0.0, 0.5),
            Tunable::new("normal_bias", self.normal_bias, 0.0, 5.0),
            Tunable::new("filter", self.filter.index(), 0.0, (Filter::ALL.len() - 1) as f32),
            Tunable::new("fade", self.fade, 0.0, 0.5),
            Tunable::new("far_steps", self.far_steps, 4.0, 128.0),
            Tunable::new("far_reach", self.far_reach, 32.0, 2048.0),
            Tunable::new("far_softness", self.far_softness, 0.0, 16.0),
            Tunable::new("far_bias", self.far_bias, 0.0, 4.0),
            Tunable::new("day_seconds", self.day_seconds, 10.0, 1200.0),
            Tunable::new("sun_low", self.sun_low, -10.0, 60.0),
            Tunable::new("sun_step", self.sun_step, 0.0, 2.0),
        ]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "distance" => self.distance = value,
            "cascades" => self.cascades = value.round(),
            "first_cascade" => self.first_cascade = value,
            "map_size" => self.map_size = (value.max(1.0) as usize).next_power_of_two() as f32,
            "depth_bias" => self.depth_bias = value,
            "normal_bias" => self.normal_bias = value,
            "filter" => self.filter = Filter::from_index(value),
            "fade" => self.fade = value,
            "far_steps" => self.far_steps = value.round(),
            "far_reach" => self.far_reach = value,
            "far_softness" => self.far_softness = value,
            "far_bias" => self.far_bias = value,
            "day_seconds" => self.day_seconds = value,
            "sun_low" => self.sun_low = value,
            "sun_step" => self.sun_step = value,
            _ => return false,
        }
        true
    }
}

/// Where the sun is now, in degrees: from the Shading sliders, or moving across the sky.
/// Everything that draws the sun or its light reads it from here.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    pub azimuth: f32,
    pub elevation: f32,
    /// How far through the moving sun's day, 0 at sunrise to 1 at sunset.
    pub day: f32,
}

impl Default for Sun {
    fn default() -> Self {
        let t = ShadingTuning::default();
        Self { azimuth: t.sun_azimuth, elevation: t.sun_elevation, day: 0.5 }
    }
}

impl Sun {
    /// The sun at `day` through a day: up in the east, highest (at the Shading elevation) in the
    /// south, down in the west, in steps of `step` degrees.
    pub fn on_day(day: f32, highest: f32, lowest: f32, step: f32) -> (f32, f32) {
        let azimuth = 90.0 + 180.0 * day;
        let elevation = lowest + (highest - lowest) * (std::f32::consts::PI * day).sin();
        let snap = |a: f32| if step > 0.0 { (a / step).round() * step } else { a };
        (snap(azimuth), snap(elevation))
    }

    /// Shading with this sun in place of its sliders'.
    pub fn applied(&self, shading: &ShadingTuning) -> ShadingTuning {
        ShadingTuning { sun_azimuth: self.azimuth, sun_elevation: self.elevation, ..*shading }
    }
}

/// The light whose shadow maps the blocks read.
#[derive(Component)]
pub struct SunLight;

/// Registers the shadows.
pub struct ShadowsPlugin;

impl Plugin for ShadowsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShadowTuning>()
            .init_resource::<Sun>()
            .insert_resource(DirectionalLightShadowMap { size: ShadowTuning::default().map_texels() })
            .add_systems(Startup, spawn_sun_light)
            .add_systems(PostUpdate, (move_sun, sync_sun_light).chain().before(crate::shading::ShadingSet));
    }
}

fn spawn_sun_light(mut commands: Commands) {
    let tuning = ShadowTuning::default();
    commands.spawn((
        SunLight,
        DirectionalLight { shadows_enabled: true, ..default() },
        sun_transform(ShadingTuning::default().sun_direction()),
        tuning.cascade_config(),
    ));
}

/// A light shining from `toward_sun` (Z-up), in render space.
pub fn sun_transform(toward_sun: Vec3) -> Transform {
    let forward = -coords::to_render(toward_sun);
    let up = if forward.y.abs() > 0.99 { Vec3::NEG_Z } else { Vec3::Y };
    Transform::default().looking_to(forward, up)
}

/// Moves the sun along its day when it's moving, or holds it at the sliders.
fn move_sun(time: Res<Time>, shading: Res<ShadingTuning>, tuning: Res<ShadowTuning>, mut sun: ResMut<Sun>) {
    let mut next = *sun;
    if tuning.switches.sun_moves {
        next.day = (sun.day + time.delta_secs() / tuning.day_seconds.max(1.0)).fract();
        (next.azimuth, next.elevation) = Sun::on_day(next.day, shading.sun_elevation, tuning.sun_low, tuning.sun_step);
    } else {
        (next.azimuth, next.elevation) = (shading.sun_azimuth, shading.sun_elevation);
    }
    sun.set_if_neq(next);
}

/// Points the light at the sun and gives it the tuning.
#[allow(clippy::type_complexity)]
fn sync_sun_light(
    sun: Res<Sun>,
    shading: Res<ShadingTuning>,
    tuning: Res<ShadowTuning>,
    mut map: ResMut<DirectionalLightShadowMap>,
    mut light: Single<(&mut DirectionalLight, &mut Transform, &mut CascadeShadowConfig), With<SunLight>>,
    camera: Single<(Entity, Option<&ShadowFilteringMethod>), With<crate::PlayerCamera>>,
    mut commands: Commands,
) {
    if !(sun.is_changed() || shading.is_changed() || tuning.is_changed()) {
        return;
    }
    let (light, transform, cascades) = &mut *light;
    // No shadow below the horizon: the maps would be drawn from underneath.
    let up = sun.applied(&shading).sun_direction().z > 0.0;
    let on = tuning.switches.maps && shading.parts.light && up;
    if light.shadows_enabled != on {
        light.shadows_enabled = on;
    }
    if light.shadow_depth_bias != tuning.depth_bias || light.shadow_normal_bias != tuning.normal_bias {
        light.shadow_depth_bias = tuning.depth_bias;
        light.shadow_normal_bias = tuning.normal_bias;
    }
    let placed = sun_transform(sun.applied(&shading).sun_direction());
    if **transform != placed {
        **transform = placed;
    }
    let config = tuning.cascade_config();
    if cascades.bounds != config.bounds {
        **cascades = config;
    }
    if map.size != tuning.map_texels() {
        map.size = tuning.map_texels();
    }
    let (entity, filter) = *camera;
    if filter != Some(&tuning.filter.method()) {
        commands.entity(entity).insert(tuning.filter.method());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-5
    }

    #[test]
    fn the_light_shines_away_from_the_sun() {
        for toward in [Vec3::new(1.0, 0.0, 1.0).normalize(), Vec3::new(0.0, -1.0, 0.3).normalize(), Vec3::Z, Vec3::new(0.01, 0.0, 1.0).normalize()] {
            let t = sun_transform(toward);
            assert!(near(*t.forward(), -coords::to_render(toward)), "{toward:?}");
        }
    }

    #[test]
    fn cascades_follow_the_tuning_and_stay_valid() {
        let config = ShadowTuning { distance: 128.0, cascades: 3.0, first_cascade: 16.0, ..default() }.cascade_config();
        assert_eq!(config.bounds.len(), 3);
        assert_eq!(config.bounds[0], 16.0);
        assert!((config.bounds.last().unwrap() - 128.0).abs() < 1e-3);
        // Out of range values are kept within what Bevy accepts rather than panicking.
        for (distance, cascades, first) in [(1.0, 9.0, 64.0), (500.0, 0.0, 0.0), (16.0, 4.0, 100.0)] {
            let c = ShadowTuning { distance, cascades, first_cascade: first, ..default() }.cascade_config();
            assert!((1..=4).contains(&c.bounds.len()));
        }
    }

    #[test]
    fn map_sizes_are_powers_of_two() {
        let mut t = ShadowTuning::default();
        assert!(t.set_tunable("map_size", 3000.0));
        assert_eq!(t.map_size, 4096.0);
        assert_eq!(ShadowTuning { map_size: 700.0, ..default() }.map_texels(), 1024);
        assert_eq!(ShadowTuning { map_size: 100.0, ..default() }.map_texels(), 512);
    }

    #[test]
    fn the_moving_sun_rises_east_peaks_south_and_sets_west() {
        let (az, el) = Sun::on_day(0.0, 60.0, 5.0, 0.0);
        assert_eq!((az, el), (90.0, 5.0));
        let (az, el) = Sun::on_day(0.5, 60.0, 5.0, 0.0);
        assert_eq!(az, 180.0);
        assert!((el - 60.0).abs() < 1e-4);
        let (az, el) = Sun::on_day(1.0, 60.0, 5.0, 0.0);
        assert!((az - 270.0).abs() < 1e-4 && (el - 5.0).abs() < 1e-4);
    }

    #[test]
    fn the_moving_sun_moves_in_steps() {
        let step = 0.5;
        for i in 0..50 {
            let (az, el) = Sun::on_day(i as f32 / 50.0, 60.0, 5.0, step);
            for a in [az, el] {
                assert!(((a / step) - (a / step).round()).abs() < 1e-3, "{a}");
            }
        }
        // Small moves within a step leave it where it was.
        assert_eq!(Sun::on_day(0.3, 60.0, 5.0, step), Sun::on_day(0.3001, 60.0, 5.0, step));
    }

    #[test]
    fn the_filter_is_picked_by_number() {
        let mut t = ShadowTuning::default();
        for f in Filter::ALL {
            assert!(t.set_tunable("filter", f.index()));
            assert_eq!(t.filter, f);
        }
        assert!(t.set_tunable("filter", 7.0));
        assert_eq!(t.filter, Filter::Temporal);
    }

    #[test]
    fn every_value_round_trips_through_its_slider() {
        let mut t = ShadowTuning::default();
        for slider in ShadowTuning::default().tunables() {
            // A whole number that's a power of two survives every rounding rule.
            let value = 1024.0f32.clamp(slider.min, slider.max).round();
            let value = if slider.name == "filter" { 1.0 } else { value };
            assert!(t.set_tunable(&slider.name, value), "{}", slider.name);
            let back = t.tunables().into_iter().find(|s| s.name == slider.name).unwrap().value;
            assert_eq!(back, value, "{}", slider.name);
        }
        assert!(!t.set_tunable("nope", 1.0));
    }
}
