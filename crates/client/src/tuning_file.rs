//! Saved tuning: the tuning panel's slider values, kept in a personal file outside the
//! project so they survive a restart.
//!
//! The file is `$BOXEL_TUNING` if set, otherwise `$XDG_CONFIG_HOME/boxel/tuning.txt`
//! (usually `~/.config/boxel/tuning.txt`). Sections are the panel's, by name; `#` starts a
//! comment:
//!
//! ```text
//! [movement]
//! walk_speed = 5.0
//! [brain cow]
//! flee_speed = 7.5
//! ```
//!
//! Loading applies what the file names over the code's defaults. Unknown sections and names
//! are skipped with a warning, and values are clamped to each slider's range.
//!
//! Two sections hold text instead of numbers: `[footsteps]` and `[combat sounds]`, the sounds
//! picked to try per surface and per kind, as `name = sound volume pitch` (see
//! `sound::SoundPicks`).

use std::path::PathBuf;

use api::{Tunable, Tunables};

/// Values read from a tuning file, in file order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedTuning {
    pub sections: Vec<(String, Vec<(String, f32)>)>,
    /// The lines of the sections kept as text ([`TEXT_SECTIONS`]), by section.
    pub texts: Vec<(String, Vec<(String, String)>)>,
}

/// The sections kept as text.
pub const FOOTSTEPS: &str = "footsteps";
pub const COMBAT_SOUNDS: &str = "combat sounds";
pub const TEXT_SECTIONS: [&str; 2] = [FOOTSTEPS, COMBAT_SOUNDS];

impl SavedTuning {
    /// The lines saved under the text section `section`.
    pub fn text(&self, section: &str) -> Vec<(String, String)> {
        self.texts.iter().filter(|(name, _)| name == section).flat_map(|(_, lines)| lines.clone()).collect()
    }
}

impl SavedTuning {
    /// Reads tuning text. Returns the values and a description of each line it skipped.
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        let mut saved = Self::default();
        let mut problems = Vec::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                saved.sections.push((name.trim().to_owned(), Vec::new()));
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                problems.push(format!("line {}: expected `name = value` or `[section]`", number + 1));
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            let Some((section, values)) = saved.sections.last_mut() else {
                problems.push(format!("line {}: {name} comes before any [section]", number + 1));
                continue;
            };
            if TEXT_SECTIONS.contains(&section.as_str()) {
                let line = (name.to_owned(), value.to_owned());
                match saved.texts.iter_mut().find(|(s, _)| s == section) {
                    Some((_, lines)) => lines.push(line),
                    None => saved.texts.push((section.clone(), vec![line])),
                }
                continue;
            }
            match value.parse::<f32>() {
                Ok(v) if v.is_finite() => values.push((name.to_owned(), v)),
                _ => problems.push(format!("line {}: expected a number for {name}, got {value:?}", number + 1)),
            }
        }
        (saved, problems)
    }

    /// Sets the values saved under `section` on `target`, clamped to each one's range.
    /// Names the target doesn't have are reported in `problems`.
    pub fn apply<T: Tunables + ?Sized>(&self, section: &str, target: &mut T, problems: &mut Vec<String>) {
        let current = target.tunables();
        for (_, values) in self.sections.iter().filter(|(name, _)| name == section) {
            for (name, value) in values {
                match current.iter().find(|t| &t.name == name) {
                    Some(t) => {
                        target.set_tunable(name, value.clamp(t.min, t.max));
                    }
                    None => problems.push(format!("[{section}] has no value named {name:?}")),
                }
            }
        }
    }

    /// The saved sections not among `known`, to warn about.
    pub fn unknown_sections<'a>(&'a self, known: &[String]) -> Vec<&'a str> {
        self.sections.iter().map(|(name, _)| name.as_str()).filter(|name| !known.iter().any(|k| k == name)).collect()
    }
}

/// Writes sections of values, then sections of text, as tuning text.
pub fn format(sections: &[(String, Vec<Tunable>)], texts: &[(String, Vec<(String, String)>)]) -> String {
    let mut out = String::from("# Tuning panel values, written by its Save button. Loaded at startup over the defaults.\n");
    for (section, values) in sections {
        out += &format!("\n[{section}]\n");
        for t in values {
            out += &format!("{} = {:?}\n", t.name, t.value);
        }
    }
    for (section, lines) in texts.iter().filter(|(_, lines)| !lines.is_empty()) {
        out += &format!("\n[{section}]\n");
        for (name, text) in lines {
            out += &format!("{name} = {text}\n");
        }
    }
    out
}

pub fn path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("BOXEL_TUNING") {
        return Some(path.into());
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("boxel").join("tuning.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Feel {
        speed: f32,
        bob: f32,
    }

    impl Tunables for Feel {
        fn tunables(&self) -> Vec<Tunable> {
            vec![Tunable::new("speed", self.speed, 0.0, 10.0), Tunable::new("bob", self.bob, 0.0, 1.0)]
        }

        fn set_tunable(&mut self, name: &str, value: f32) -> bool {
            match name {
                "speed" => self.speed = value,
                "bob" => self.bob = value,
                _ => return false,
            }
            true
        }
    }

    #[test]
    fn footsteps_are_kept_as_text() {
        let picks = vec![("base:grass".to_owned(), "base:step_leather 0.4 0.98".to_owned())];
        let combat = vec![("swing".to_owned(), "base:swing_rpg 1.00 1.10".to_owned())];
        let texts = [(FOOTSTEPS.to_owned(), picks.clone()), (COMBAT_SOUNDS.to_owned(), combat.clone())];
        let text = format(&[("sound".to_owned(), vec![Tunable::new("master", 0.8, 0.0, 2.0)])], &texts);
        let (saved, problems) = SavedTuning::parse(&text);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!((saved.text(FOOTSTEPS), saved.text(COMBAT_SOUNDS)), (picks, combat));
        assert_eq!(saved.sections[0].1, [("master".to_owned(), 0.8)], "numbers still read as numbers");
    }

    #[test]
    fn written_values_read_back() {
        let sections = vec![
            ("movement".to_owned(), vec![Tunable::new("walk_speed", 5.0, 0.0, 10.0), Tunable::new("gravity", 25.5, 0.0, 80.0)]),
            ("brain cow".to_owned(), vec![Tunable::new("flee_speed", 0.1, 0.0, 9.0)]),
        ];
        let (saved, problems) = SavedTuning::parse(&format(&sections, &[]));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            saved.sections,
            vec![
                ("movement".to_owned(), vec![("walk_speed".to_owned(), 5.0), ("gravity".to_owned(), 25.5)]),
                ("brain cow".to_owned(), vec![("flee_speed".to_owned(), 0.1)]),
            ]
        );
    }

    #[test]
    fn skips_comments_and_reports_bad_lines() {
        let text = "orphan = 1\n# note\n[camera]  # trailing\n  bob = 0.5 \nnonsense\nspeed = fast\nspeed = inf\n";
        let (saved, problems) = SavedTuning::parse(text);
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert_eq!(saved.sections, vec![("camera".to_owned(), vec![("bob".to_owned(), 0.5)])]);
    }

    #[test]
    fn applying_changes_only_what_is_named_and_clamps() {
        let (saved, _) = SavedTuning::parse("[camera]\nspeed = 50\n[other]\nbob = 0.9\n");
        let mut feel = Feel { speed: 2.0, bob: 0.3 };
        let mut problems = Vec::new();
        saved.apply("camera", &mut feel, &mut problems);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(feel, Feel { speed: 10.0, bob: 0.3 }, "clamped to the slider's max, bob untouched");
    }

    #[test]
    fn unknown_names_and_sections_are_reported() {
        let (saved, _) = SavedTuning::parse("[camera]\nsped = 3\nbob = 0.1\n[gone]\nx = 1\n");
        let mut feel = Feel { speed: 2.0, bob: 0.3 };
        let mut problems = Vec::new();
        saved.apply("camera", &mut feel, &mut problems);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(feel, Feel { speed: 2.0, bob: 0.1 }, "a misspelled name doesn't stop the rest");
        assert_eq!(saved.unknown_sections(&["camera".to_owned()]), vec!["gone"]);
    }
}
