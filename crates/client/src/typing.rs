//! Keys typed into a text box don't also play: while egui has a text field focused, the
//! game's key state is cleared each frame before anything reads it. Escape still gets
//! through, so it closes whatever panel the text box is in.

use bevy::prelude::*;
use bevy_egui::{EguiPreUpdateSet, input::EguiWantsInput};

/// Runs the gate in `PreUpdate`, once egui has read this frame's input.
pub struct TypingPlugin;

impl Plugin for TypingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, gate_keys_while_typing.after(EguiPreUpdateSet::ProcessInput));
    }
}

fn gate_keys_while_typing(wants: Res<EguiWantsInput>, mut keys: ResMut<ButtonInput<KeyCode>>) {
    gate_keys(wants.wants_keyboard_input(), &mut keys);
}

/// Clears every key while `typing`, except a fresh press of Escape.
pub fn gate_keys(typing: bool, keys: &mut ButtonInput<KeyCode>) {
    if !typing {
        return;
    }
    let escape = keys.just_pressed(KeyCode::Escape);
    keys.reset_all();
    if escape {
        keys.press(KeyCode::Escape);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(keys: &[KeyCode]) -> ButtonInput<KeyCode> {
        let mut input = ButtonInput::default();
        for &key in keys {
            input.press(key);
        }
        input
    }

    #[test]
    fn typing_hides_keys_from_the_game() {
        let mut keys = held(&[KeyCode::KeyW, KeyCode::Space, KeyCode::Digit2]);
        gate_keys(true, &mut keys);
        assert!(!keys.pressed(KeyCode::KeyW) && !keys.just_pressed(KeyCode::Space) && !keys.just_pressed(KeyCode::Digit2));
    }

    #[test]
    fn keys_play_when_not_typing() {
        let mut keys = held(&[KeyCode::KeyW]);
        gate_keys(false, &mut keys);
        assert!(keys.pressed(KeyCode::KeyW) && keys.just_pressed(KeyCode::KeyW));
    }

    #[test]
    fn escape_still_closes_the_panel_while_typing() {
        let mut keys = held(&[KeyCode::KeyW, KeyCode::Escape]);
        gate_keys(true, &mut keys);
        assert!(keys.just_pressed(KeyCode::Escape));
        assert!(!keys.pressed(KeyCode::KeyW));
    }
}
