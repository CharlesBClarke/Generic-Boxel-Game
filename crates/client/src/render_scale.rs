//! The world's resolution: the 3D view can be drawn smaller than the window, to buy frame
//! rate, and stretched up to fill it. The menus and on-screen text are drawn by a camera of
//! their own at the window's full size, so they stay sharp whatever the world is drawn at.
//!
//! At native resolution the player's camera draws straight to the window, as it always did;
//! picked lower, it draws into an image of that size, shown behind the UI across the window.

use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::window::{PrimaryWindow, WindowRef};
use bevy_egui::{EguiContext, PrimaryEguiContext};

use crate::PlayerCamera;
use crate::view::ViewTuning;

/// The resolutions to pick from, by height; the width follows the window's shape. Saved by
/// place in [`Resolution::ALL`], so new ones go at the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// The window's own size.
    Native,
    P1440,
    P1080,
    P900,
    P720,
    P540,
    P360,
}

impl Resolution {
    pub const ALL: [Resolution; 7] = [
        Resolution::Native,
        Resolution::P1440,
        Resolution::P1080,
        Resolution::P900,
        Resolution::P720,
        Resolution::P540,
        Resolution::P360,
    ];

    /// The height drawn at, or `None` for the window's own.
    pub fn height(self) -> Option<u32> {
        match self {
            Resolution::Native => None,
            Resolution::P1440 => Some(1440),
            Resolution::P1080 => Some(1080),
            Resolution::P900 => Some(900),
            Resolution::P720 => Some(720),
            Resolution::P540 => Some(540),
            Resolution::P360 => Some(360),
        }
    }

    /// What the picker shows, for a window `window` physical pixels in size.
    pub fn label(self, window: UVec2) -> String {
        match (self.height(), draw_size(window, self)) {
            (None, _) => format!("native ({}×{})", window.x, window.y),
            (Some(h), Some(size)) => format!("{h}p ({}×{})", size.x, size.y),
            (Some(h), None) => format!("{h}p (as big as the window: native)"),
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&r| r == self).unwrap_or(0)
    }

    pub fn from_index(value: f32) -> Self {
        Self::ALL[(value.round().max(0.0) as usize).min(Self::ALL.len() - 1)]
    }
}

/// How the smaller picture is stretched to the window. Saved by place in [`Upscale::ALL`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Upscale {
    /// Blended between neighboring pixels: soft.
    Smooth,
    /// Each pixel a sharp-edged square.
    Pixelated,
}

impl Upscale {
    pub const ALL: [Upscale; 2] = [Upscale::Smooth, Upscale::Pixelated];

    pub fn label(self) -> &'static str {
        match self {
            Upscale::Smooth => "smooth",
            Upscale::Pixelated => "pixelated",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&u| u == self).unwrap_or(0)
    }

    pub fn from_index(value: f32) -> Self {
        Self::ALL[(value.round().max(0.0) as usize).min(Self::ALL.len() - 1)]
    }

    fn sampler(self) -> ImageSampler {
        match self {
            Upscale::Smooth => ImageSampler::linear(),
            Upscale::Pixelated => ImageSampler::nearest(),
        }
    }
}

/// The size to draw the world at in a window `window` physical pixels in size, or `None` to
/// draw it straight to the window: at native, when the pick isn't smaller than the window, or
/// when the window has no size (minimized).
pub fn draw_size(window: UVec2, pick: Resolution) -> Option<UVec2> {
    let height = pick.height()?;
    if window.x == 0 || window.y == 0 || height >= window.y {
        return None;
    }
    let width = (window.x as f64 * height as f64 / window.y as f64).round() as u32;
    Some(UVec2::new(width.max(1), height))
}

/// The camera that draws the UI and egui to the window, after the world.
#[derive(Component)]
pub(crate) struct ScreenCamera;

/// The world's picture, stretched across the window behind the rest of the UI, when it's drawn
/// smaller than the window.
#[derive(Component)]
pub(crate) struct WorldPicture;

/// Spawns the screen camera, which owns the UI and egui, and the picture node.
pub(crate) fn setup_screen(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Camera { order: 1, clear_color: ClearColorConfig::None, ..default() },
        // Matched to the player camera's at native (see `apply_resolution`), so the two share
        // the window's textures.
        Msaa::Off,
        IsDefaultUiCamera,
        EguiContext::default(),
        PrimaryEguiContext,
        ScreenCamera,
    ));
    commands.spawn((
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
        ImageNode::default(),
        GlobalZIndex(i32::MIN),
        Visibility::Hidden,
        WorldPicture,
    ));
}

/// The player's camera, kept apart from the screen camera's for the borrow checker.
type PlayerOnly = (With<PlayerCamera>, Without<ScreenCamera>);

/// Points the player's camera at the window or at an image of the picked size, when the pick,
/// the stretch or the window's size changes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_resolution(
    view: Res<ViewTuning>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    player: Single<(Entity, &Msaa), PlayerOnly>,
    mut screen: Single<&mut Msaa, With<ScreenCamera>>,
    mut picture: Single<(&mut ImageNode, &mut Visibility), With<WorldPicture>>,
    mut current: Local<Option<(Option<UVec2>, Upscale)>>,
) {
    let (camera, player_msaa) = *player;
    let size = draw_size(window.physical_size(), view.resolution);
    // Two cameras drawing to the window share its textures only with the same multisampling;
    // drawn into an image, the screen camera has nothing to multisample.
    let msaa = if size.is_none() { *player_msaa } else { Msaa::Off };
    if **screen != msaa {
        **screen = msaa;
    }
    if *current == Some((size, view.upscale)) {
        return;
    }
    *current = Some((size, view.upscale));
    let (node, visibility) = &mut *picture;
    match size {
        None => {
            commands.entity(camera).insert(RenderTarget::Window(WindowRef::Primary));
            **visibility = Visibility::Hidden;
            node.image = Handle::default();
        }
        Some(size) => {
            let mut image = Image::new_target_texture(size.x, size.y, TextureFormat::bevy_default(), None);
            image.sampler = view.upscale.sampler();
            let handle = images.add(image);
            commands.entity(camera).insert(RenderTarget::Image(ImageRenderTarget::from(handle.clone())));
            **visibility = Visibility::Inherited;
            node.image = handle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_draws_to_the_window() {
        assert_eq!(draw_size(UVec2::new(2560, 1440), Resolution::Native), None);
    }

    #[test]
    fn the_width_follows_the_window_shape() {
        assert_eq!(draw_size(UVec2::new(2560, 1440), Resolution::P1080), Some(UVec2::new(1920, 1080)));
        assert_eq!(draw_size(UVec2::new(2560, 1440), Resolution::P720), Some(UVec2::new(1280, 720)));
        // A square window stays square; an odd shape rounds.
        assert_eq!(draw_size(UVec2::new(1080, 1080), Resolution::P720), Some(UVec2::new(720, 720)));
        assert_eq!(draw_size(UVec2::new(1000, 999), Resolution::P540), Some(UVec2::new(541, 540)));
    }

    #[test]
    fn a_pick_as_big_as_the_window_is_native() {
        assert_eq!(draw_size(UVec2::new(2560, 1440), Resolution::P1440), None);
        assert_eq!(draw_size(UVec2::new(1280, 720), Resolution::P1080), None);
    }

    #[test]
    fn a_minimized_window_draws_to_the_window() {
        assert_eq!(draw_size(UVec2::new(0, 0), Resolution::P720), None);
        assert_eq!(draw_size(UVec2::new(800, 0), Resolution::P720), None);
    }

    #[test]
    fn a_very_thin_window_still_draws_a_pixel() {
        assert_eq!(draw_size(UVec2::new(1, 2000), Resolution::P360), Some(UVec2::new(1, 360)));
    }

    #[test]
    fn picks_round_trip_through_their_index() {
        for r in Resolution::ALL {
            assert_eq!(Resolution::from_index(r.index() as f32), r);
        }
        for u in Upscale::ALL {
            assert_eq!(Upscale::from_index(u.index() as f32), u);
        }
        assert_eq!(Resolution::from_index(99.0), Resolution::P360);
        assert_eq!(Resolution::from_index(-3.0), Resolution::Native);
    }
}
