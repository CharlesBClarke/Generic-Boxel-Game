//! Key bindings. Defaults live here; a player can override any of them in a personal file
//! outside the project, so nobody's own layout ends up in the repo.
//!
//! The file is `$BOXEL_CONTROLS` if set, otherwise `$XDG_CONFIG_HOME/boxel/controls.txt`
//! (usually `~/.config/boxel/controls.txt`). One binding per line, `#` starts a comment:
//!
//! ```text
//! forward = KeyW
//! fly = KeyQ
//! double_tap_window = 0.3
//! ```
//!
//! `double_tap_window` is the one setting that takes a number: seconds between two taps of
//! forward that start sprinting, or 0 to turn double-tap sprint off.
//!
//! Keys use Bevy's physical key names, which are QWERTY positions: `KeyW` is whichever key
//! sends the W scancode. `use`, `alt_use` and `placement_mode` also take mouse buttons:
//! `MouseLeft`, `MouseRight`, `MouseMiddle`, `MouseBack`, `MouseForward`.
//!
//! Whatever the player holds has a main use and a second use: a block breaks and places, a
//! tool swings and guards. `break` and `place` are older names for `use` and `alt_use`.

use std::path::PathBuf;

use bevy::prelude::*;

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct Controls {
    pub forward: KeyCode,
    pub back: KeyCode,
    pub left: KeyCode,
    pub right: KeyCode,
    pub jump: KeyCode,
    /// Sprint. A press turns sprinting on until it's pressed again or forward is let go.
    pub sprint: KeyCode,
    /// Crouch. Shares left Ctrl with fly-down by default; flying ignores crouch.
    pub crouch: KeyCode,
    /// Fly mode: down. Fly up shares the jump key.
    pub down: KeyCode,
    pub fly: KeyCode,
    pub teleport: KeyCode,
    pub frame_cap: KeyCode,
    /// Opens the live tuning panel.
    pub tuning: KeyCode,
    /// Starts and stops the chase drill.
    pub drill: KeyCode,
    /// Puts every target back at its spot, whole.
    pub respawn_targets: KeyCode,
    /// Shows and hides the dev text (readouts, coordinates, controls). Hidden by default in
    /// release builds.
    pub dev_text: KeyCode,
    /// Throws a block's worth of the held material. Held, it keeps throwing.
    pub drop: KeyCode,
    /// Opens the crafting menu at a crafting table aimed at, and closes it.
    pub interact: KeyCode,
    /// In the crafting menu: turns the part being painted with a quarter turn.
    pub turn_part: KeyCode,
    /// Tapping forward twice within this many seconds sprints until forward is released.
    /// 0 turns it off. The sprint key works either way.
    pub double_tap_window: f32,
    /// The main use of what's held: breaks a block, swings a tool.
    pub use_main: Binding,
    /// The second use of what's held: places a block, raises a tool's guard.
    pub use_alt: Binding,
    /// Switches between basic placement and precise placement.
    pub placement_mode: Binding,
    /// Picks a tool or material from the palette, first to ninth. The scroll wheel also cycles it.
    pub slots: [KeyCode; 9],
}

/// A key or a mouse button.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Binding {
    Key(KeyCode),
    Mouse(MouseButton),
}

impl Binding {
    pub fn pressed(self, keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> bool {
        match self {
            Binding::Key(key) => keys.pressed(key),
            Binding::Mouse(button) => mouse.pressed(button),
        }
    }

    pub fn just_pressed(self, keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> bool {
        match self {
            Binding::Key(key) => keys.just_pressed(key),
            Binding::Mouse(button) => mouse.just_pressed(button),
        }
    }

    /// A short label for the HUD.
    pub fn label(self) -> String {
        match self {
            Binding::Key(key) => label(key),
            Binding::Mouse(MouseButton::Left) => "left click".into(),
            Binding::Mouse(MouseButton::Right) => "right click".into(),
            Binding::Mouse(MouseButton::Middle) => "middle click".into(),
            Binding::Mouse(button) => format!("mouse {button:?}").to_lowercase(),
        }
    }
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            forward: KeyCode::KeyW,
            back: KeyCode::KeyS,
            left: KeyCode::KeyA,
            right: KeyCode::KeyD,
            jump: KeyCode::Space,
            sprint: KeyCode::ShiftLeft,
            crouch: KeyCode::ControlLeft,
            down: KeyCode::ControlLeft,
            fly: KeyCode::KeyF,
            teleport: KeyCode::KeyT,
            frame_cap: KeyCode::F3,
            tuning: KeyCode::Tab,
            drill: KeyCode::F4,
            respawn_targets: KeyCode::F6,
            dev_text: KeyCode::F1,
            drop: KeyCode::KeyQ,
            interact: KeyCode::KeyE,
            turn_part: KeyCode::KeyR,
            double_tap_window: 0.3,
            use_main: Binding::Mouse(MouseButton::Left),
            use_alt: Binding::Mouse(MouseButton::Right),
            placement_mode: Binding::Mouse(MouseButton::Middle),
            slots: [
                KeyCode::Digit1,
                KeyCode::Digit2,
                KeyCode::Digit3,
                KeyCode::Digit4,
                KeyCode::Digit5,
                KeyCode::Digit6,
                KeyCode::Digit7,
                KeyCode::Digit8,
                KeyCode::Digit9,
            ],
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
            if action == "double_tap_window" {
                match key.parse::<f32>() {
                    Ok(seconds) if seconds.is_finite() && seconds >= 0.0 => self.double_tap_window = seconds,
                    _ => problems.push(format!("line {}: expected seconds, at least 0, got {key:?}", number + 1)),
                }
                continue;
            }
            let Some(binding) = parse_binding(key) else {
                problems.push(format!("line {}: unknown key {key:?}", number + 1));
                continue;
            };
            match action {
                "use" | "break" => {
                    self.use_main = binding;
                    continue;
                }
                "alt_use" | "place" => {
                    self.use_alt = binding;
                    continue;
                }
                "placement_mode" => {
                    self.placement_mode = binding;
                    continue;
                }
                _ => {}
            }
            let Binding::Key(code) = binding else {
                problems.push(format!("line {}: {action} needs a key, not a mouse button", number + 1));
                continue;
            };
            let slot_index = action.strip_prefix("slot_").and_then(|n| n.parse::<usize>().ok());
            if let Some(n) = slot_index.filter(|n| (1..=9).contains(n)) {
                self.slots[n - 1] = code;
                continue;
            }
            let slot = match action {
                "forward" => &mut self.forward,
                "back" => &mut self.back,
                "left" => &mut self.left,
                "right" => &mut self.right,
                "jump" => &mut self.jump,
                "sprint" => &mut self.sprint,
                "crouch" => &mut self.crouch,
                "down" => &mut self.down,
                "fly" => &mut self.fly,
                "teleport" => &mut self.teleport,
                "frame_cap" => &mut self.frame_cap,
                "tuning" => &mut self.tuning,
                "drill" => &mut self.drill,
                "respawn_targets" => &mut self.respawn_targets,
                "dev_text" => &mut self.dev_text,
                "drop" => &mut self.drop,
                "interact" => &mut self.interact,
                "turn_part" => &mut self.turn_part,
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

/// Parses a key name or one of the mouse button names. Case doesn't matter.
pub fn parse_binding(name: &str) -> Option<Binding> {
    let mouse = [
        ("MouseLeft", MouseButton::Left),
        ("MouseRight", MouseButton::Right),
        ("MouseMiddle", MouseButton::Middle),
        ("MouseBack", MouseButton::Back),
        ("MouseForward", MouseButton::Forward),
    ];
    match mouse.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
        Some((_, button)) => Some(Binding::Mouse(*button)),
        None => parse_key(name).map(Binding::Key),
    }
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

/// Double-tap forward to sprint: a second tap soon after the first sprints until forward is
/// released.
#[derive(Resource, Debug, Default)]
pub struct DoubleTapSprint {
    last_tap: Option<f64>,
    pub active: bool,
}

impl DoubleTapSprint {
    /// Call every frame with whether forward was just pressed, whether it's held, the time in
    /// seconds, and the window from the controls.
    pub fn update(&mut self, tapped: bool, held: bool, now: f64, window: f32) {
        if !held {
            self.active = false;
        }
        if tapped {
            if window > 0.0 && self.last_tap.is_some_and(|t| now - t <= window as f64) {
                self.active = true;
            }
            self.last_tap = Some(now);
        }
    }
}

/// Sprint is a toggle: a press turns it on, and it stays on until sprint is pressed again or
/// forward is let go. Pressed while standing, it waits for the walk that follows.
#[derive(Resource, Debug, Default)]
pub struct SprintToggle {
    forward_was_held: bool,
    pub active: bool,
}

impl SprintToggle {
    /// Call every frame with whether sprint was just pressed and whether forward is held.
    pub fn update(&mut self, pressed: bool, forward_held: bool) {
        if pressed {
            self.active = !self.active;
        }
        if self.forward_was_held && !forward_held {
            self.active = false;
        }
        self.forward_was_held = forward_held;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_tapping_forward_sprints_until_released() {
        let mut d = DoubleTapSprint::default();
        d.update(true, true, 0.0, 0.3);
        assert!(!d.active, "one tap is a walk");
        d.update(false, false, 0.1, 0.3);
        d.update(true, true, 0.2, 0.3);
        assert!(d.active);
        d.update(false, true, 5.0, 0.3);
        assert!(d.active, "holding keeps sprinting");
        d.update(false, false, 5.1, 0.3);
        assert!(!d.active, "letting go stops");
    }

    #[test]
    fn sprint_toggles_on_and_off_with_one_press_each() {
        let mut t = SprintToggle::default();
        t.update(true, true);
        assert!(t.active, "one press sprints");
        t.update(false, true);
        assert!(t.active, "no need to hold it");
        t.update(true, true);
        assert!(!t.active, "a second press walks");
    }

    #[test]
    fn letting_go_of_forward_stops_the_sprint() {
        let mut t = SprintToggle::default();
        t.update(false, true);
        t.update(true, true);
        t.update(false, false);
        assert!(!t.active);
        t.update(false, true);
        assert!(!t.active, "walking again doesn't bring it back");
    }

    #[test]
    fn a_press_while_standing_waits_for_the_walk() {
        let mut t = SprintToggle::default();
        t.update(true, false);
        t.update(false, false);
        assert!(t.active);
        t.update(false, true);
        assert!(t.active, "sprints from the first step");
    }

    #[test]
    fn shift_sprints_and_ctrl_crouches_by_default() {
        let controls = Controls::default();
        assert_eq!(controls.sprint, KeyCode::ShiftLeft);
        assert_eq!((controls.crouch, controls.down), (KeyCode::ControlLeft, KeyCode::ControlLeft));
    }

    #[test]
    fn slow_taps_and_a_zero_window_do_not_sprint() {
        let mut d = DoubleTapSprint::default();
        d.update(true, true, 0.0, 0.3);
        d.update(false, false, 0.2, 0.3);
        d.update(true, true, 0.5, 0.3);
        assert!(!d.active, "too slow");

        let mut d = DoubleTapSprint::default();
        d.update(true, true, 0.0, 0.0);
        d.update(false, false, 0.05, 0.0);
        d.update(true, true, 0.1, 0.0);
        assert!(!d.active, "turned off");
    }

    #[test]
    fn reads_the_double_tap_window() {
        let mut controls = Controls::default();
        assert!(controls.apply("double_tap_window = 0.25\n").is_empty());
        assert_eq!(controls.double_tap_window, 0.25);
        assert_eq!(controls.apply("double_tap_window = soon\ndouble_tap_window = -1\n").len(), 2);
        assert_eq!(controls.double_tap_window, 0.25);
    }

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
    fn uses_take_keys_or_mouse_buttons() {
        let mut controls = Controls::default();
        let problems = controls.apply(
            "break = KeyQ\nplace = mousemiddle\nplacement_mode = KeyG\nslot_3 = KeyZ\njump = MouseLeft\nslot_10 = KeyX\n",
        );
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert_eq!(controls.use_main, Binding::Key(KeyCode::KeyQ));
        assert_eq!(controls.use_alt, Binding::Mouse(MouseButton::Middle));
        assert_eq!(controls.placement_mode, Binding::Key(KeyCode::KeyG));
        assert_eq!(Controls::default().placement_mode, Binding::Mouse(MouseButton::Middle));
        assert_eq!(controls.slots[2], KeyCode::KeyZ);
        assert_eq!(controls.slots[0], KeyCode::Digit1);
        assert_eq!(controls.jump, Controls::default().jump);
    }

    #[test]
    fn use_and_alt_use_have_their_old_names_too() {
        let mut controls = Controls::default();
        assert!(controls.apply("use = KeyQ\nalt_use = KeyE\n").is_empty());
        assert_eq!((controls.use_main, controls.use_alt), (Binding::Key(KeyCode::KeyQ), Binding::Key(KeyCode::KeyE)));
        assert!(controls.apply("break = MouseRight\nplace = MouseLeft\nrespawn_targets = F7\n").is_empty());
        assert_eq!(controls.use_main, Binding::Mouse(MouseButton::Right));
        assert_eq!(controls.use_alt, Binding::Mouse(MouseButton::Left));
        assert_eq!(controls.respawn_targets, KeyCode::F7);
    }

    #[test]
    fn interact_is_bound_and_the_shape_key_is_gone() {
        let mut controls = Controls::default();
        assert_eq!(controls.interact, KeyCode::KeyE);
        assert!(controls.apply("interact = KeyR
").is_empty());
        assert_eq!(controls.interact, KeyCode::KeyR);
        assert_eq!(controls.apply("shape = KeyG
").len(), 1, "shapes are crafted now, not picked");
    }

    #[test]
    fn turn_part_is_bound() {
        let mut controls = Controls::default();
        assert_eq!(controls.turn_part, KeyCode::KeyR);
        assert!(controls.apply("turn_part = KeyG\n").is_empty());
        assert_eq!(controls.turn_part, KeyCode::KeyG);
    }

    #[test]
    fn dev_text_is_bound() {
        let mut controls = Controls::default();
        assert_eq!(controls.dev_text, KeyCode::F1);
        assert!(controls.apply("dev_text = F2\n").is_empty());
        assert_eq!(controls.dev_text, KeyCode::F2);
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
