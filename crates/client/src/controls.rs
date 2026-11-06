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

/// Declares the plain key actions: one row each, giving the name (the field and the config
/// file name), the default key, and the HUD hint text (`None` for keys shown elsewhere or not
/// at all). Everything else about a key action comes from its row.
macro_rules! key_actions {
    ($( $(#[$doc:meta])* $name:ident = $key:ident, $hud:expr; )*) => {
        /// The actions bound to a single key each. `Controls` derefs to these, so they read as
        /// `controls.fly`.
        #[derive(Clone, Debug, PartialEq)]
        pub struct Keys {
            $( $(#[$doc])* pub $name: KeyCode, )*
        }

        impl Default for Keys {
            fn default() -> Self {
                Self { $( $name: KeyCode::$key, )* }
            }
        }

        impl Keys {
            /// The key bound to the action with this config name.
            pub fn get_mut(&mut self, name: &str) -> Option<&mut KeyCode> {
                match name {
                    $( stringify!($name) => Some(&mut self.$name), )*
                    _ => None,
                }
            }

            /// Every key action with its key and HUD hint text, in table order.
            pub fn all(&self) -> Vec<(&'static str, KeyCode, Option<&'static str>)> {
                vec![ $( (stringify!($name), self.$name, $hud), )* ]
            }
        }
    };
}

key_actions! {
    forward = KeyW, None;
    back = KeyS, None;
    left = KeyA, None;
    right = KeyD, None;
    jump = Space, Some("jump");
    /// Sprint. A press turns sprinting on until it's pressed again or forward is let go.
    sprint = ShiftLeft, Some("toggle sprint (or double-tap forward)");
    /// Crouch. Shares left Ctrl with fly-down by default; flying ignores crouch.
    crouch = ControlLeft, Some("crouch");
    /// Fly mode: down. Fly up shares the jump key.
    down = ControlLeft, Some("down");
    fly = KeyF, Some("fly");
    teleport = KeyT, Some("teleport");
    /// Cycles the frame cap; its key shows beside the cap readout.
    frame_cap = F3, None;
    /// Opens the live tuning panel.
    tuning = Tab, Some("tuning");
    /// Starts and stops the chase drill; the drill shows its own key.
    drill = F4, None;
    /// Puts every target back at its spot, whole.
    respawn_targets = F6, Some("respawn targets");
    /// Reads the sounds again (`sounds.toml` and its files), to try sound files without a
    /// restart.
    reload_sounds = F5, Some("reload sounds");
    /// Shows and hides the dev text (readouts, coordinates, controls). Hidden by default in
    /// release builds.
    dev_text = F1, Some("hide dev text");
    /// Shows and hides every creature's path (see `crate::combat::draw_routes`).
    show_routes = F7, Some("show cow paths");
    /// Throws a block's worth of the held material. Held, it keeps throwing. Shown in the held
    /// item line.
    drop = KeyQ, None;
    /// Opens the crafting menu at a crafting table aimed at, and closes it.
    interact = KeyE, Some("craft at a table");
    /// Measures the frame rate for a few seconds and writes it down (see `crate::perf`).
    record_perf = F9, None;
    /// Goes back to where the last frame rate recording was made, looking the same way.
    perf_spot = F10, None;
    /// In the crafting menu: turns the part being painted with a quarter turn.
    turn_part = KeyR, None;
}

/// All bindings. Plain key actions live in [`Keys`] (one table above); the rest are the ones
/// that take a mouse button, a number, or come as a set.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct Controls {
    pub keys: Keys,
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

impl std::ops::Deref for Controls {
    type Target = Keys;
    fn deref(&self) -> &Keys {
        &self.keys
    }
}

impl std::ops::DerefMut for Controls {
    fn deref_mut(&mut self) -> &mut Keys {
        &mut self.keys
    }
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
            keys: Keys::default(),
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

/// Splits config text into `(line number, action, value)`, skipping blanks and comments.
/// A line without `=` comes back with `None` for the value.
fn entries(text: &str) -> impl Iterator<Item = (usize, &str, Option<&str>)> {
    text.lines().enumerate().filter_map(|(number, line)| {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            return None;
        }
        Some(match line.split_once('=') {
            Some((action, value)) => (number + 1, action.trim(), Some(value.trim())),
            None => (number + 1, line, None),
        })
    })
}

impl Controls {
    /// The defaults with the personal file's overrides applied. A missing file is fine; bad
    /// lines are reported and skipped, and actions the file doesn't list are named.
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
        let unlisted = Self::unlisted(&text);
        if !unlisted.is_empty() {
            warn!("{} doesn't list: {}", path.display(), unlisted.join(", "));
        }
        info!("loaded controls from {}", path.display());
        controls
    }

    /// Every action name the config file takes, under its current name.
    pub fn action_names() -> Vec<String> {
        let mut names: Vec<String> = Keys::default().all().iter().map(|(name, _, _)| name.to_string()).collect();
        names.extend(["use", "alt_use", "placement_mode", "double_tap_window"].map(String::from));
        names.extend((1..=9).map(|n| format!("slot_{n}")));
        names
    }

    /// The actions config text doesn't name, each with its default, such as `show_routes (F7)`.
    /// Old names count for the new ones.
    pub fn unlisted(text: &str) -> Vec<String> {
        let named: Vec<&str> = entries(text)
            .map(|(_, action, _)| match action {
                "break" => "use",
                "place" => "alt_use",
                other => other,
            })
            .collect();
        let defaults = Self::default();
        Self::action_names()
            .into_iter()
            .filter(|name| !named.contains(&name.as_str()))
            .map(|name| {
                let default = match name.as_str() {
                    "use" => defaults.use_main.label(),
                    "alt_use" => defaults.use_alt.label(),
                    "placement_mode" => defaults.placement_mode.label(),
                    "double_tap_window" => defaults.double_tap_window.to_string(),
                    _ => match name.strip_prefix("slot_").and_then(|n| n.parse::<usize>().ok()) {
                        Some(n) => format!("{:?}", defaults.slots[n - 1]),
                        None => defaults.keys.all().iter().find(|(n, _, _)| *n == name).map_or(String::new(), |(_, key, _)| format!("{key:?}")),
                    },
                };
                format!("{name} ({default})")
            })
            .collect()
    }

    /// Applies overrides from config text. Returns a description of each line it skipped.
    pub fn apply(&mut self, text: &str) -> Vec<String> {
        let mut problems = Vec::new();
        for (number, action, key) in entries(text) {
            let Some(key) = key else {
                problems.push(format!("line {number}: expected `action = Key`"));
                continue;
            };
            if action == "double_tap_window" {
                match key.parse::<f32>() {
                    Ok(seconds) if seconds.is_finite() && seconds >= 0.0 => self.double_tap_window = seconds,
                    _ => problems.push(format!("line {number}: expected seconds, at least 0, got {key:?}")),
                }
                continue;
            }
            let Some(binding) = parse_binding(key) else {
                problems.push(format!("line {number}: unknown key {key:?}"));
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
                problems.push(format!("line {number}: {action} needs a key, not a mouse button"));
                continue;
            };
            let slot_index = action.strip_prefix("slot_").and_then(|n| n.parse::<usize>().ok());
            if let Some(n) = slot_index.filter(|n| (1..=9).contains(n)) {
                self.slots[n - 1] = code;
                continue;
            }
            match self.keys.get_mut(action) {
                Some(slot) => *slot = code,
                None => problems.push(format!("line {number}: unknown action {action:?}")),
            }
        }
        problems
    }

    /// The controls hint for the HUD: movement and the uses, then every key action that has
    /// hint text, wrapped into lines.
    pub fn hint(&self) -> String {
        let mut parts = vec![
            format!("{}{}{}{} move", label(self.forward), label(self.left), label(self.back), label(self.right)),
            "Esc release mouse".into(),
            format!("{} use (break, swing)", self.use_main.label()),
            format!("{} second use (place, guard)", self.use_alt.label()),
            "1-9 or scroll pick".into(),
        ];
        parts.extend(self.keys.all().into_iter().filter_map(|(_, key, hud)| Some(format!("{} {}", label(key), hud?))));
        let mut lines = vec![String::new()];
        for part in parts {
            let line = lines.last_mut().expect("starts with one line");
            if !line.is_empty() && line.len() + part.len() > 110 {
                lines.push(part);
            } else {
                if !line.is_empty() {
                    *line += ", ";
                }
                *line += &part;
            }
        }
        lines.join("\n")
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
    fn every_key_action_parses_into_its_own_field() {
        for (name, _, _) in Keys::default().all() {
            let mut controls = Controls::default();
            assert!(controls.apply(&format!("{name} = Backquote\n")).is_empty(), "{name}");
            let changed: Vec<_> = controls.keys.all().into_iter().filter(|(_, key, _)| *key == KeyCode::Backquote).map(|(n, _, _)| n).collect();
            assert_eq!(changed, [name]);
        }
    }

    #[test]
    fn action_names_are_unique() {
        let names = Controls::action_names();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
    }

    #[test]
    fn the_hint_shows_every_key_action_with_hint_text() {
        let hint = Controls::default().hint();
        for (name, key, hud) in Keys::default().all() {
            if let Some(hud) = hud {
                assert!(hint.contains(&format!("{} {hud}", label(key))), "{name} missing from {hint}");
            }
        }
        assert!(hint.contains("WASD move") && hint.contains("left click use"));
        assert!(hint.lines().all(|line| line.len() <= 120), "{hint}");
    }

    #[test]
    fn names_what_the_file_leaves_out() {
        let unlisted = Controls::unlisted("forward = KeyF\nbreak = MouseLeft\nslot_1 = Digit1\n");
        assert!(unlisted.contains(&"show_routes (F7)".to_string()), "{unlisted:?}");
        assert!(unlisted.contains(&"alt_use (right click)".to_string()), "{unlisted:?}");
        assert!(!unlisted.iter().any(|u| u.starts_with("forward ") || u.starts_with("use ") || u.starts_with("slot_1 ")), "{unlisted:?}");
        assert!(unlisted.contains(&"slot_2 (Digit2)".to_string()));
        let everything: String = Controls::action_names().iter().map(|n| format!("{n} = x\n")).collect();
        assert!(Controls::unlisted(&everything).is_empty());
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
