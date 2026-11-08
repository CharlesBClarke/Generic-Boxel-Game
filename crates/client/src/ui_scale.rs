//! How big the UI is drawn: menus and on-screen text grow and shrink with the window's height,
//! times a size the player picks. Applies to both egui (the menus and the hotbar) and Bevy UI
//! (the health bar and the readouts). The 3D scene, including the held tool and outlines,
//! isn't scaled.

use api::{Tunable, Tunables};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::{EguiContextSettings, PrimaryEguiContext};

/// The UI scale's tuning.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct UiScaleTuning {
    /// The window height, in logical pixels, at which the UI is drawn at its natural size.
    pub reference_height: f32,
    /// The player's multiplier on top of the window's.
    pub size: f32,
}

impl Default for UiScaleTuning {
    fn default() -> Self {
        Self { reference_height: 1080.0, size: 1.0 }
    }
}

/// The smallest and largest scale the UI is ever drawn at, so a tiny or huge window can't
/// make it unusable.
pub const MIN_SCALE: f32 = 0.5;
pub const MAX_SCALE: f32 = 4.0;

impl UiScaleTuning {
    /// The UI scale for a window `height` logical pixels tall.
    pub fn scale(&self, height: f32) -> f32 {
        let scale = self.size * height / self.reference_height.max(1.0);
        if scale.is_finite() { scale.clamp(MIN_SCALE, MAX_SCALE) } else { 1.0 }
    }
}

impl Tunables for UiScaleTuning {
    fn tunables(&self) -> Vec<Tunable> {
        vec![
            Tunable::new("size", self.size, 0.5, 3.0),
            Tunable::new("reference_height", self.reference_height, 480.0, 2160.0),
        ]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "size" => self.size = value.max(0.1),
            "reference_height" => self.reference_height = value.max(1.0),
            _ => return false,
        }
        true
    }
}

/// Sets egui's and Bevy UI's scale from the window's height, only when it changes, so egui
/// doesn't lay out again every frame. Waits while the left button is held, so dragging the size
/// slider doesn't move the slider under the pointer.
pub(crate) fn apply_ui_scale(
    tuning: Res<UiScaleTuning>,
    window: Single<&Window, With<PrimaryWindow>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut ui_scale: ResMut<UiScale>,
    mut egui: Query<&mut EguiContextSettings, With<PrimaryEguiContext>>,
) {
    if buttons.pressed(MouseButton::Left) {
        return;
    }
    let scale = tuning.scale(window.height());
    if ui_scale.0 != scale {
        ui_scale.0 = scale;
    }
    for mut settings in &mut egui {
        if settings.scale_factor != scale {
            settings.scale_factor = scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_size_at_the_reference_height() {
        assert_eq!(UiScaleTuning::default().scale(1080.0), 1.0);
    }

    #[test]
    fn grows_with_the_window() {
        let scale = UiScaleTuning::default().scale(1440.0);
        assert!((scale - 4.0 / 3.0).abs() < 1e-5, "{scale}");
    }

    #[test]
    fn size_multiplies() {
        let tuning = UiScaleTuning { size: 1.5, ..default() };
        assert!((tuning.scale(1080.0) - 1.5).abs() < 1e-5);
        assert!((tuning.scale(720.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn kept_within_limits() {
        let tuning = UiScaleTuning::default();
        assert_eq!(tuning.scale(10.0), MIN_SCALE);
        assert_eq!(tuning.scale(100_000.0), MAX_SCALE);
        let broken = UiScaleTuning { reference_height: 0.0, size: 1.0 };
        assert_eq!(broken.scale(1080.0), MAX_SCALE);
    }

    #[test]
    fn sliders_set_values() {
        let mut tuning = UiScaleTuning::default();
        assert!(tuning.set_tunable("size", 2.0));
        assert!(tuning.set_tunable("reference_height", 900.0));
        assert!(!tuning.set_tunable("nope", 1.0));
        assert_eq!(tuning, UiScaleTuning { reference_height: 900.0, size: 2.0 });
        let names: Vec<_> = tuning.tunables().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["size", "reference_height"]);
    }

    #[test]
    fn saves_and_loads_through_the_tuning_file() {
        let tuned = UiScaleTuning { reference_height: 900.0, size: 1.25 };
        let text = crate::tuning_file::format(&[("ui".to_owned(), tuned.tunables())], &[]);
        let (saved, mut problems) = crate::tuning_file::SavedTuning::parse(&text);
        let mut loaded = UiScaleTuning::default();
        saved.apply("ui", &mut loaded, &mut problems);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(loaded, tuned);
    }
}
