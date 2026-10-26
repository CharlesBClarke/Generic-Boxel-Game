//! Key bindings. Defaults live here; a player can override any of them in a personal file
//! outside the project, so nobody's own layout ends up in the repo.
//!
//! The file is `$BOXEL_CONTROLS` if set, otherwise `$XDG_CONFIG_HOME/boxel/controls.txt`
//! (usually `~/.config/boxel/controls.txt`). One binding per line, `#` starts a comment:
//!
//! ```text
//! forward = KeyW
//! fly = KeyQ
//! ```
//!
//! Keys use Bevy's physical key names, which are QWERTY positions: `KeyW` is whichever key
//! sends the W scancode.

use std::path::PathBuf;

use bevy::prelude::*;

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct Controls {
    pub forward: KeyCode,
    pub back: KeyCode,
    pub left: KeyCode,
    pub right: KeyCode,
    pub jump: KeyCode,
    pub sprint: KeyCode,
    /// Fly mode: down. Fly up shares the jump key.
    pub down: KeyCode,
    pub fly: KeyCode,
    pub teleport: KeyCode,
    pub frame_cap: KeyCode,
    /// Opens the live tuning panel.
    pub tuning: KeyCode,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            forward: KeyCode::KeyW,
            back: KeyCode::KeyS,
            left: KeyCode::KeyA,
            right: KeyCode::KeyD,
            jump: KeyCode::Space,
            sprint: KeyCode::ControlLeft,
            down: KeyCode::ShiftLeft,
            fly: KeyCode::KeyF,
            teleport: KeyCode::KeyT,
            frame_cap: KeyCode::F3,
            tuning: KeyCode::Tab,
        }
    }
}

impl Controls {
    /// The defaults with the personal file's overrides applied. A missing file is fine; bad
    /// lines are reported and skipped.
    pub fn load() -> Self {
        let mut controls = Self::default();
        let Some(path) = config_path() else {
            return controls;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return controls;
        };
        for problem in controls.apply(&text) {
            warn!("{}: {problem}", path.display());
        }
        info!("loaded controls from {}", path.display());
        controls
    }

    /// Applies overrides from config text. Returns a description of each line it skipped.
    pub fn apply(&mut self, text: &str) -> Vec<String> {
        let mut problems = Vec::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((action, key)) = line.split_once('=') else {
                problems.push(format!("line {}: expected `action = Key`", number + 1));
                continue;
            };
            let (action, key) = (action.trim(), key.trim());
            let Some(code) = parse_key(key) else {
                problems.push(format!("line {}: unknown key {key:?}", number + 1));
                continue;
            };
            let slot = match action {
                "forward" => &mut self.forward,
                "back" => &mut self.back,
                "left" => &mut self.left,
                "right" => &mut self.right,
                "jump" => &mut self.jump,
                "sprint" => &mut self.sprint,
                "down" => &mut self.down,
                "fly" => &mut self.fly,
                "teleport" => &mut self.teleport,
                "frame_cap" => &mut self.frame_cap,
                "tuning" => &mut self.tuning,
                _ => {
                    problems.push(format!("line {}: unknown action {action:?}", number + 1));
                    continue;
                }
            };
            *slot = code;
        }
        problems
    }
}

fn config_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("BOXEL_CONTROLS") {
        return Some(path.into());
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("boxel").join("controls.txt"))
}

/// A short label for the HUD: `KeyW` becomes `W`, `Digit1` becomes `1`.
pub fn label(key: KeyCode) -> String {
    let name = format!("{key:?}");
    name.strip_prefix("Key").or_else(|| name.strip_prefix("Digit")).unwrap_or(&name).to_string()
}

/// Parses a Bevy physical key name, such as `KeyW`, `Digit1`, `F3` or `ShiftLeft`.
/// Case doesn't matter.
pub fn parse_key(name: &str) -> Option<KeyCode> {
    KEYS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, code)| *code)
}

const KEYS: &[(&str, KeyCode)] = &[
    ("KeyA", KeyCode::KeyA), ("KeyB", KeyCode::KeyB), ("KeyC", KeyCode::KeyC),
    ("KeyD", KeyCode::KeyD), ("KeyE", KeyCode::KeyE), ("KeyF", KeyCode::KeyF),
    ("KeyG", KeyCode::KeyG), ("KeyH", KeyCode::KeyH), ("KeyI", KeyCode::KeyI),
    ("KeyJ", KeyCode::KeyJ), ("KeyK", KeyCode::KeyK), ("KeyL", KeyCode::KeyL),
    ("KeyM", KeyCode::KeyM), ("KeyN", KeyCode::KeyN), ("KeyO", KeyCode::KeyO),
    ("KeyP", KeyCode::KeyP), ("KeyQ", KeyCode::KeyQ), ("KeyR", KeyCode::KeyR),
    ("KeyS", KeyCode::KeyS), ("KeyT", KeyCode::KeyT), ("KeyU", KeyCode::KeyU),
    ("KeyV", KeyCode::KeyV), ("KeyW", KeyCode::KeyW), ("KeyX", KeyCode::KeyX),
    ("KeyY", KeyCode::KeyY), ("KeyZ", KeyCode::KeyZ),
    ("Digit0", KeyCode::Digit0), ("Digit1", KeyCode::Digit1), ("Digit2", KeyCode::Digit2),
    ("Digit3", KeyCode::Digit3), ("Digit4", KeyCode::Digit4), ("Digit5", KeyCode::Digit5),
    ("Digit6", KeyCode::Digit6), ("Digit7", KeyCode::Digit7), ("Digit8", KeyCode::Digit8),
    ("Digit9", KeyCode::Digit9),
    ("F1", KeyCode::F1), ("F2", KeyCode::F2), ("F3", KeyCode::F3), ("F4", KeyCode::F4),
    ("F5", KeyCode::F5), ("F6", KeyCode::F6), ("F7", KeyCode::F7), ("F8", KeyCode::F8),
    ("F9", KeyCode::F9), ("F10", KeyCode::F10), ("F11", KeyCode::F11), ("F12", KeyCode::F12),
    ("Space", KeyCode::Space), ("Tab", KeyCode::Tab), ("Enter", KeyCode::Enter),
    ("Backspace", KeyCode::Backspace), ("CapsLock", KeyCode::CapsLock),
    ("ShiftLeft", KeyCode::ShiftLeft), ("ShiftRight", KeyCode::ShiftRight),
    ("ControlLeft", KeyCode::ControlLeft), ("ControlRight", KeyCode::ControlRight),
    ("AltLeft", KeyCode::AltLeft), ("AltRight", KeyCode::AltRight),
    ("ArrowUp", KeyCode::ArrowUp), ("ArrowDown", KeyCode::ArrowDown),
    ("ArrowLeft", KeyCode::ArrowLeft), ("ArrowRight", KeyCode::ArrowRight),
    ("Semicolon", KeyCode::Semicolon), ("Quote", KeyCode::Quote), ("Comma", KeyCode::Comma),
    ("Period", KeyCode::Period), ("Slash", KeyCode::Slash), ("Backslash", KeyCode::Backslash),
    ("BracketLeft", KeyCode::BracketLeft), ("BracketRight", KeyCode::BracketRight),
    ("Backquote", KeyCode::Backquote), ("Minus", KeyCode::Minus), ("Equal", KeyCode::Equal),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_only_what_the_file_names() {
        let mut controls = Controls::default();
        let problems = controls.apply("# my layout\nforward = KeyF\n\n  right=KeyT  # trailing comment\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(controls.forward, KeyCode::KeyF);
        assert_eq!(controls.right, KeyCode::KeyT);
        assert_eq!(controls.back, Controls::default().back);
    }

    #[test]
    fn reports_bad_lines_and_keeps_going() {
        let mut controls = Controls::default();
        let problems = controls.apply("forward = KeyQQ\ndash = KeyQ\nnonsense\nback = KeyR\n");
        assert_eq!(problems.len(), 3);
        assert_eq!(controls.forward, KeyCode::KeyW);
        assert_eq!(controls.back, KeyCode::KeyR);
    }

    #[test]
    fn every_key_name_matches_its_code() {
        for (name, code) in KEYS {
            assert_eq!(format!("{code:?}"), *name);
        }
    }

    #[test]
    fn key_names_ignore_case() {
        assert_eq!(parse_key("keyA"), Some(KeyCode::KeyA));
        assert_eq!(parse_key("SHIFTLEFT"), Some(KeyCode::ShiftLeft));
    }

    #[test]
    fn labels_are_short() {
        assert_eq!(label(KeyCode::KeyW), "W");
        assert_eq!(label(KeyCode::Digit1), "1");
        assert_eq!(label(KeyCode::F3), "F3");
    }
}
