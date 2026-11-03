//! The player being hurt, on the client side: a health bar, and the juice of a creature's blow
//! landing. The simulation decides what hits and how hard; this only shows it.
//!
//! A blow freezes the game for a moment, jolts and shakes the view, flashes the screen red
//! (weaker when the guard caught it), plays the attacker's sound low and says what happened
//! under the crosshair.

use bevy::prelude::*;

use crate::combat::{CombatFeel, HitReadout, HitStop, TargetViews, ViewKick, play_sound};
use crate::Sim;

/// How strong the red flash is for a guarded blow, as a share of an unguarded one.
const GUARDED_FLASH: f32 = 0.4;
/// The health bar's size, pixels.
const BAR_WIDTH: f32 = 240.0;
const BAR_HEIGHT: f32 = 10.0;

/// The red flash over the screen: seconds left, and how strong it started.
#[derive(Resource, Debug, Default)]
pub(crate) struct HurtFlash {
    left: f32,
    length: f32,
    strength: f32,
}

impl HurtFlash {
    fn flash(&mut self, seconds: f32, strength: f32) {
        self.left = seconds;
        self.length = seconds.max(1e-3);
        self.strength = strength;
    }

    /// How opaque it is now.
    fn alpha(&self) -> f32 {
        if self.left <= 0.0 {
            return 0.0;
        }
        let fade = (self.left / self.length).clamp(0.0, 1.0);
        self.strength * fade * fade
    }
}

/// The readout's words for a blow.
fn hurt_words(damage: f32, guarded: bool, died: bool) -> String {
    match (died, guarded) {
        (true, _) => "KNOCKED OUT".into(),
        (false, true) => format!("GUARDED  -{damage:.0}"),
        (false, false) => format!("HURT  -{damage:.0}"),
    }
}

#[derive(Component)]
pub(crate) struct HealthFill;

#[derive(Component)]
pub(crate) struct HealthBar;

#[derive(Component)]
pub(crate) struct FlashOverlay;

/// The health bar at the top of the screen, and the red flash covering it all.
pub(crate) fn setup_health(mut commands: Commands) {
    commands.spawn((
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
        BackgroundColor(Color::NONE),
        Pickable::IGNORE,
        FlashOverlay,
    ));
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            top: px(12),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Node {
                    width: px(BAR_WIDTH),
                    height: px(BAR_HEIGHT),
                    border: UiRect::all(px(1)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
                BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.6)),
                Visibility::Hidden,
                HealthBar,
            ))
            .with_child((
                Node { width: percent(100), height: percent(100), ..default() },
                BackgroundColor(Color::srgb(0.85, 0.15, 0.1)),
                HealthFill,
            ));
        });
}

/// Plays the juice of every blow on the player since the last frame.
#[allow(clippy::too_many_arguments)]
pub(crate) fn play_player_hurts(
    mut commands: Commands,
    mut sim: ResMut<Sim>,
    feel: Res<CombatFeel>,
    views: Res<TargetViews>,
    mut stop: ResMut<HitStop>,
    mut kick: ResMut<ViewKick>,
    mut flash: ResMut<HurtFlash>,
    mut readout: ResMut<HitReadout>,
) {
    for hurt in sim.0.take_player_hurts() {
        let share = if hurt.guarded { GUARDED_FLASH } else { 1.0 };
        stop.freeze(feel.hurt_hitstop * share);
        kick.kick(feel.hurt_kick * share);
        kick.shake(feel.hurt_shake * share, feel.shake_time * 2.0);
        flash.flash(feel.hurt_flash, feel.hurt_flash_alpha * share);
        readout.show(hurt_words(hurt.damage, hurt.guarded, hurt.died));
        let sound = sim.0.target_def(hurt.kind).hit_sound.clone();
        play_sound(&mut commands, &views, sound.as_deref(), 0.75, &feel, hurt.tick ^ hurt.target);
    }
}

/// Keeps the health bar and the red flash up to date. The bar shows only with a health
/// definition.
pub(crate) fn update_health(
    time: Res<Time>,
    sim: Res<Sim>,
    mut flash: ResMut<HurtFlash>,
    mut bar: Single<&mut Visibility, With<HealthBar>>,
    mut fill: Single<&mut Node, With<HealthFill>>,
    mut overlay: Single<&mut BackgroundColor, With<FlashOverlay>>,
) {
    flash.left = (flash.left - time.delta_secs()).max(0.0);
    let color = Color::srgba(0.8, 0.0, 0.0, flash.alpha());
    if overlay.0 != color {
        overlay.0 = color;
    }
    let Some(def) = sim.0.health_def() else {
        **bar = Visibility::Hidden;
        return;
    };
    **bar = Visibility::Inherited;
    let width = percent(100.0 * (sim.0.health() / def.health).clamp(0.0, 1.0));
    if fill.width != width {
        fill.width = width;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flash_fades_out() {
        let mut f = HurtFlash::default();
        assert_eq!(f.alpha(), 0.0);
        f.flash(0.4, 0.5);
        assert!((f.alpha() - 0.5).abs() < 1e-6);
        f.left = 0.2;
        assert!(f.alpha() > 0.0 && f.alpha() < 0.25);
        f.left = 0.0;
        assert_eq!(f.alpha(), 0.0);
    }

    #[test]
    fn the_readout_says_what_a_blow_did() {
        assert_eq!(hurt_words(7.0, false, false), "HURT  -7");
        assert_eq!(hurt_words(1.5, true, false), "GUARDED  -2");
        assert_eq!(hurt_words(7.0, false, true), "KNOCKED OUT");
    }
}
