//! The sky through a day: where the sun and moon are at an hour, and the colors of the sky and
//! its light, from the time-of-day palette of Eanpa Sky by SkyeShark (MIT, by way of the
//! stylised-sky block; see `licenses/`). Plain math, no Bevy: [`crate::shading`] turns a
//! [`DaySky`] into the shaders' atmosphere.
//!
//! Directions are Z-up. The sun rises in the east at 6:00, is highest in the south at noon
//! (at `sun_arc` degrees) and sets in the west at 18:00; the moon follows the same arc,
//! `moon_lag` hours behind and lower.

use bevy::math::Vec3;

/// One row of the palette: the sky's colors and the sun's light with the sun at `elevation`
/// degrees. Colors are linear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub elevation: f32,
    /// The sky straight up, and at the horizon.
    pub zenith: [f32; 3],
    pub horizon: [f32; 3],
    /// The sun's color.
    pub sun: [f32; 3],
    /// How strong the sun is, 0 to 3 (Eanpa's scale: 3 at its highest).
    pub strength: f32,
    /// How much the stars show, 0 to 1.
    pub stars: f32,
}

/// Eanpa Sky's palette, by the sun's elevation: deep night, the end of twilight, sunset,
/// golden hour, day and high noon.
pub const PALETTE: [Palette; 6] = [
    Palette { elevation: -18.0, zenith: [0.01, 0.016, 0.055], horizon: [0.03, 0.042, 0.1], sun: [0.55, 0.65, 0.95], strength: 0.0, stars: 1.0 },
    Palette { elevation: -6.0, zenith: [0.022, 0.045, 0.115], horizon: [0.19, 0.12, 0.19], sun: [1.0, 0.42, 0.22], strength: 0.1, stars: 0.85 },
    Palette { elevation: 0.0, zenith: [0.07, 0.13, 0.3], horizon: [0.92, 0.42, 0.2], sun: [1.0, 0.48, 0.23], strength: 0.55, stars: 0.25 },
    Palette { elevation: 8.0, zenith: [0.14, 0.3, 0.66], horizon: [1.0, 0.62, 0.36], sun: [1.0, 0.69, 0.41], strength: 1.6, stars: 0.0 },
    Palette { elevation: 30.0, zenith: [0.15, 0.38, 0.82], horizon: [0.62, 0.78, 0.92], sun: [1.0, 0.94, 0.86], strength: 2.6, stars: 0.0 },
    Palette { elevation: 70.0, zenith: [0.11, 0.35, 0.8], horizon: [0.6, 0.76, 0.9], sun: [1.0, 1.0, 1.0], strength: 3.0, stars: 0.0 },
];

/// The palette at `elevation` degrees: the rows either side blended, held at the ends.
pub fn palette_at(elevation: f32) -> Palette {
    let first = PALETTE[0];
    let last = PALETTE[PALETTE.len() - 1];
    if elevation <= first.elevation || elevation.is_nan() {
        return first;
    }
    if elevation >= last.elevation {
        return last;
    }
    let i = PALETTE.windows(2).position(|w| elevation <= w[1].elevation).unwrap_or(PALETTE.len() - 2);
    let (a, b) = (PALETTE[i], PALETTE[i + 1]);
    let k = (elevation - a.elevation) / (b.elevation - a.elevation);
    let mix3 = |x: [f32; 3], y: [f32; 3]| [0, 1, 2].map(|c| x[c] + (y[c] - x[c]) * k);
    Palette {
        elevation,
        zenith: mix3(a.zenith, b.zenith),
        horizon: mix3(a.horizon, b.horizon),
        sun: mix3(a.sun, b.sun),
        strength: a.strength + (b.strength - a.strength) * k,
        stars: a.stars + (b.stars - a.stars) * k,
    }
}

/// Where something on the day's arc is at `hours`: (azimuth, elevation) in degrees, azimuth
/// clockwise from north. Up from 6:00 to 18:00, highest (`arc` degrees) at noon in the south,
/// as far below at midnight in the north.
pub fn on_arc(hours: f64, arc: f32) -> (f32, f32) {
    let day = ((hours - 6.0) / 12.0).rem_euclid(2.0) as f32;
    let azimuth = (90.0 + 180.0 * day).rem_euclid(360.0);
    let elevation = arc * (std::f32::consts::PI * day).sin();
    (azimuth, elevation)
}

/// Toward a point at (azimuth, elevation) degrees, Z-up.
pub fn direction(azimuth: f32, elevation: f32) -> Vec3 {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    Vec3::new(a.sin() * e.cos(), a.cos() * e.cos(), e.sin())
}

/// What a day's sky is tuned by (see [`crate::shading::ShadingTuning`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyTuning {
    /// The sun's elevation at noon, degrees.
    pub sun_arc: f32,
    /// How bright the sun lights a face turned to it at its highest.
    pub sun_strength: f32,
    /// Light from the sky on an upward face at midday, and from below on a downward one.
    pub sky_light: f32,
    pub ground_light: f32,
    /// How much the sky's light takes the zenith's color (0 white), and the ground's the
    /// horizon's.
    pub sky_tint: f32,
    /// How many hours the moon runs behind the sun, and its highest elevation.
    pub moon_lag: f32,
    pub moon_arc: f32,
    /// How bright the moon lights a face turned to it when high.
    pub moon_strength: f32,
    /// Sky light at deepest night, so the open world stays faintly readable.
    pub night_light: f32,
    /// How much warmer than the palette a low sun is (the stylised-sky block's tweak: tone
    /// mapping washes a low sun's colors pale otherwise); 0 is the palette as it is.
    pub warmth: f32,
    /// The sky's colors, brighter or darker, by day and at full night.
    pub day_sky: f32,
    pub night_sky: f32,
}

/// Everything about the sky at one moment. Directions Z-up, colors linear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DaySky {
    pub hours: f64,
    /// Where the sun is, degrees.
    pub sun_azimuth: f32,
    pub sun_elevation: f32,
    /// The sun's light on a face turned to it; zero once it's down.
    pub sun_light: Vec3,
    /// The sun's hue at full brightness, for its disc and glow.
    pub sun_hue: Vec3,
    pub moon: Vec3,
    /// The moon's light on a face turned to it; zero while it's down or the sun is up.
    pub moon_light: Vec3,
    /// 0 by day, 1 by night: how much the moon and stars show.
    pub night: f32,
    pub stars: f32,
    pub zenith: Vec3,
    pub horizon: Vec3,
    /// The sky's light on an upward face, and from below on a downward one.
    pub sky_light: Vec3,
    pub ground_light: Vec3,
}

/// The sun's light fades in over its first few degrees above the horizon and is gone at the
/// horizon itself, since below it the shadow maps would be drawn from underneath.
pub const SUN_FADE_DEGREES: f32 = 3.0;

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The colors of `c` turned toward its own hue at full brightness by `k` from white.
fn tinted(c: [f32; 3], k: f32) -> Vec3 {
    let c = Vec3::from_array(c);
    Vec3::ONE.lerp(c / c.max_element().max(1e-4), k.clamp(0.0, 1.0))
}

impl DaySky {
    /// The sky at `hours`, with the sun moved in steps of `step` degrees (0 for smooth), so
    /// its shadows don't crawl every frame.
    pub fn at(hours: f64, t: &SkyTuning, step: f32) -> Self {
        let snap = |a: f32| if step > 0.0 { (a / step).round() * step } else { a };
        let (az, el) = on_arc(hours, t.sun_arc);
        let (sun_azimuth, sun_elevation) = (snap(az), snap(el));
        let (moon_az, moon_el) = on_arc(hours - t.moon_lag as f64, t.moon_arc);
        let moon = direction(moon_az, moon_el);
        let pal = palette_at(sun_elevation);
        // The stylised-sky block's warmth for a low sun: at its most just above the horizon.
        let warm = t.warmth * smoothstep(16.0, 2.0, sun_elevation) * ((sun_elevation + 8.0) / 8.0).clamp(0.0, 1.0);
        let horizon = [pal.horizon[0], pal.horizon[1] * (1.0 - warm * 0.2), pal.horizon[2] * (1.0 - warm * 0.42)];
        let night = ((-sun_elevation - 2.0) / 8.0).clamp(0.0, 1.0);
        let sky_brightness = t.day_sky + (t.night_sky - t.day_sky) * night;
        let noon = PALETTE[PALETTE.len() - 1].strength;
        let day = pal.strength / noon;

        // The sun, warmer still when low, as the block keys its clouds' light.
        let key = 1.0 + warm * 0.9;
        let sun_hue = Vec3::from_array(pal.sun.map(|c| c.powf(key)));
        let up = smoothstep(0.0, SUN_FADE_DEGREES, sun_elevation);
        let sun_light = sun_hue * t.sun_strength * day * up;

        // The moon: blue and dim, only once the sun has gone, fading in as it rises.
        let moon_up = smoothstep(0.0, 7.0, moon_el);
        let moon_light = Vec3::new(0.5, 0.6, 0.95) * t.moon_strength * night * moon_up;

        // The sky's light: the block's hemisphere light (0.25 + strength × 0.25 at Eanpa's
        // scale), in the sky's own color, which darkens through dusk to near black at night;
        // relative to noon, with the night floor under it.
        let top = PALETTE[PALETTE.len() - 1];
        let darkness = Vec3::from_array(pal.zenith).max_element() / Vec3::from_array(top.zenith).max_element();
        let daylight = (0.25 + pal.strength * 0.25) / (0.25 + noon * 0.25) * darkness;
        let sky_hue = tinted(pal.zenith, t.sky_tint);
        let ground_hue = tinted(horizon, t.sky_tint);
        let night_hue = Vec3::new(0.55, 0.65, 1.0);
        // Through dusk the palette's light hands over to the night's, which is all there is
        // once the sun is 10° down: `night_light` alone, so at 0 the open world is as dark as
        // a cave.
        let sky_light = (sky_hue * t.sky_light * daylight).lerp(night_hue * t.night_light, night);
        let ground_light = (ground_hue * t.ground_light * daylight).lerp(night_hue * t.night_light * 0.5, night);

        Self {
            hours,
            sun_azimuth,
            sun_elevation,
            sun_light,
            sun_hue,
            moon,
            moon_light,
            night,
            stars: pal.stars,
            zenith: Vec3::from_array(pal.zenith) * sky_brightness,
            horizon: Vec3::from_array(horizon) * sky_brightness,
            sky_light,
            ground_light,
        }
    }

    /// Toward the sun, Z-up.
    pub fn sun(&self) -> Vec3 {
        direction(self.sun_azimuth, self.sun_elevation)
    }

    /// Whether the sun lights anything: false once it's at or below the horizon, so the
    /// shadows can stop drawing.
    pub fn sun_lights(&self) -> bool {
        self.sun_light.max_element() > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuning() -> SkyTuning {
        SkyTuning {
            sun_arc: 58.0,
            sun_strength: 0.6,
            sky_light: 0.55,
            ground_light: 0.35,
            sky_tint: 0.3,
            moon_lag: 12.0,
            moon_arc: 48.0,
            moon_strength: 0.15,
            night_light: 0.05,
            warmth: 1.0,
            day_sky: 1.0,
            night_sky: 1.0,
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn the_palette_holds_at_its_ends_and_meets_its_rows() {
        assert_eq!(palette_at(-90.0), PALETTE[0]);
        assert_eq!(palette_at(90.0), PALETTE[5]);
        assert_eq!(palette_at(f32::NAN), PALETTE[0]);
        for row in PALETTE {
            let p = palette_at(row.elevation);
            assert!(close(p.strength, row.strength) && close(p.zenith[2], row.zenith[2]), "{}", row.elevation);
        }
    }

    #[test]
    fn the_palette_blends_between_rows() {
        // Halfway from sunset (0°) to golden hour (8°).
        let p = palette_at(4.0);
        assert!(close(p.strength, (0.55 + 1.6) / 2.0));
        assert!(close(p.horizon[1], (0.42 + 0.62) / 2.0));
        assert!(close(p.stars, 0.125));
    }

    #[test]
    fn the_sun_crosses_the_sky_east_to_west() {
        let (az, el) = on_arc(6.0, 58.0);
        assert!(close(az, 90.0) && close(el, 0.0));
        let (az, el) = on_arc(12.0, 58.0);
        assert!(close(az, 180.0) && close(el, 58.0));
        let (az, el) = on_arc(18.0, 58.0);
        assert!(close(az, 270.0) && close(el, 0.0));
        let (az, el) = on_arc(0.0, 58.0);
        assert!(close(az, 0.0) && close(el, -58.0));
        // 10:00 is where the sun sat before it moved (azimuth 150, about 50 up).
        let (az, el) = on_arc(10.0, 58.0);
        assert!(close(az, 150.0) && (el - 50.2).abs() < 0.1);
        // Hours wrap.
        assert_eq!(on_arc(30.0, 58.0), on_arc(6.0, 58.0));
        assert_eq!(on_arc(-6.0, 58.0), on_arc(18.0, 58.0));
    }

    #[test]
    fn directions_point_the_compass_way() {
        assert!(direction(90.0, 0.0).distance(Vec3::X) < 1e-5, "east is +X");
        assert!(direction(0.0, 0.0).distance(Vec3::Y) < 1e-5, "north is +Y");
        assert!(direction(123.0, 90.0).distance(Vec3::Z) < 1e-5);
    }

    #[test]
    fn noon_is_bright_and_starless_midnight_dark_and_moonlit() {
        let t = tuning();
        let noon = DaySky::at(12.0, &t, 0.0);
        assert!(noon.sun_lights() && noon.night == 0.0 && noon.stars == 0.0);
        assert_eq!(noon.moon_light, Vec3::ZERO);
        let midnight = DaySky::at(0.0, &t, 0.0);
        assert!(!midnight.sun_lights());
        assert_eq!(midnight.night, 1.0);
        assert!(midnight.stars > 0.99);
        assert!(midnight.moon.z > 0.7 && midnight.moon_light.max_element() > 0.0, "the moon is high at midnight");
        assert!(midnight.zenith.max_element() < noon.zenith.max_element() / 10.0);
        // The night floor keeps a little sky light, and with it at 0 the night is near black.
        assert!(midnight.sky_light.min_element() >= t.night_light * 0.5);
        let dark = DaySky::at(0.0, &SkyTuning { night_light: 0.0, moon_strength: 0.0, ..t }, 0.0);
        assert_eq!((dark.sky_light, dark.ground_light), (Vec3::ZERO, Vec3::ZERO), "as dark as a cave");
        assert_eq!(dark.moon_light, Vec3::ZERO);
    }

    #[test]
    fn the_sun_goes_out_at_the_horizon_not_below_it() {
        let t = tuning();
        // Just before 18:00 the sun is above the horizon, but within its fade.
        let low = DaySky::at(17.97, &t, 0.0);
        assert!(low.sun_elevation > 0.0 && low.sun_elevation < SUN_FADE_DEGREES);
        let set = DaySky::at(18.0, &t, 0.0);
        assert!(!set.sun_lights());
        assert!(low.sun_light.max_element() < DaySky::at(16.0, &t, 0.0).sun_light.max_element());
        assert!(!DaySky::at(19.0, &t, 0.0).sun_lights());
    }

    #[test]
    fn sunset_light_is_warm_and_noon_light_white() {
        let t = tuning();
        let sunset = DaySky::at(17.5, &t, 0.0).sun_light;
        assert!(sunset.x > sunset.y * 1.4 && sunset.y > sunset.z, "{sunset:?}");
        // White at the palette's highest row; nearly so at the default 58°.
        let high = SkyTuning { sun_arc: 70.0, ..t };
        let noon = DaySky::at(12.0, &high, 0.0).sun_light;
        assert!(close(noon.x, noon.z), "{noon:?}");
        let noon = DaySky::at(12.0, &t, 0.0).sun_light;
        assert!(noon.z > noon.x * 0.9, "{noon:?}");
        // The light from below is warm at dusk: it takes the horizon's color.
        let dusk = DaySky::at(17.8, &t, 0.0).ground_light;
        assert!(dusk.x > dusk.z, "{dusk:?}");
    }

    #[test]
    fn the_sun_and_moon_can_both_be_up() {
        // With the moon only six hours behind, it's up all afternoon.
        let t = SkyTuning { moon_lag: 6.0, ..tuning() };
        let afternoon = DaySky::at(15.0, &t, 0.0);
        assert!(afternoon.sun_elevation > 0.0 && afternoon.moon.z > 0.0);
        // But only the sun lights faces while it's up: moonlight waits for the night.
        assert_eq!(afternoon.moon_light, Vec3::ZERO);
    }

    #[test]
    fn day_and_night_skies_are_brightened_apart() {
        let t = SkyTuning { day_sky: 2.0, night_sky: 0.5, ..tuning() };
        let plain = tuning();
        let noon = DaySky::at(12.0, &t, 0.0).zenith;
        assert!(noon.distance(DaySky::at(12.0, &plain, 0.0).zenith * 2.0) < 1e-5);
        let midnight = DaySky::at(0.0, &t, 0.0).horizon;
        assert!(midnight.distance(DaySky::at(0.0, &plain, 0.0).horizon * 0.5) < 1e-5);
        // Halfway through dusk (the sun 6° down), halfway between.
        let dusk = DaySky::at(18.0 + 6.0 / 58.0 * 12.0 / std::f64::consts::PI, &t, 0.0);
        assert!((dusk.night - 0.5).abs() < 0.02, "{}", dusk.night);
    }

    #[test]
    fn the_sun_moves_in_steps() {
        let t = tuning();
        let sky = DaySky::at(10.123, &t, 0.5);
        assert!(close((sky.sun_azimuth / 0.5).fract(), 0.0) && close((sky.sun_elevation / 0.5).fract().abs(), 0.0));
    }
}
