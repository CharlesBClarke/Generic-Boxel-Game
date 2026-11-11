//! How the tuning panel is laid out: which tab each section goes in, its heading, what a
//! search matches, and the favourite sliders, kept in a file beside the tuning file.

use std::collections::BTreeSet;
use std::path::PathBuf;

use api::Tunable;

use crate::tuning_file;

/// A page of the tuning panel's sidebar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    Favourites,
    #[default]
    Player,
    Combat,
    Building,
    Creatures,
    World,
    Light,
    Display,
    Sound,
    Dev,
    /// Sections no other tab claims, such as a mod's own.
    Other,
}

impl Tab {
    pub const ALL: [Tab; 11] = [
        Tab::Favourites,
        Tab::Player,
        Tab::Combat,
        Tab::Building,
        Tab::Creatures,
        Tab::World,
        Tab::Light,
        Tab::Display,
        Tab::Sound,
        Tab::Dev,
        Tab::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Favourites => "★ Favourites",
            Tab::Player => "Player",
            Tab::Combat => "Combat",
            Tab::Building => "Building and items",
            Tab::Creatures => "Creatures",
            Tab::World => "World",
            Tab::Light => "Light and sky",
            Tab::Display => "Display",
            Tab::Sound => "Sound",
            Tab::Dev => "Dev",
            Tab::Other => "Other",
        }
    }

    /// The tab a section (by its saved name) goes in. Never [`Tab::Favourites`].
    pub fn of(section: &str) -> Tab {
        match section {
            "movement" | "push_out" | "health" | "camera" => Tab::Player,
            "combat" => Tab::Combat,
            "interaction" | "inventory" | "item_drops" => Tab::Building,
            "crowd" | "animation" => Tab::Creatures,
            "world" | "time" | "saving" | "signals" => Tab::World,
            "shading" | "shadows" | "ao" => Tab::Light,
            "view" | "ui" => Tab::Display,
            "sound" => Tab::Sound,
            "drill" => Tab::Dev,
            _ => match section.split_once(' ') {
                Some(("tool", _)) => Tab::Combat,
                Some(("target" | "brain", _)) => Tab::Creatures,
                Some(("terrain", _)) => Tab::World,
                _ => Tab::Other,
            },
        }
    }
}

/// A section's heading in the panel, by its saved name.
pub fn heading(section: &str) -> String {
    let fixed = match section {
        "movement" => "Movement",
        "interaction" => "Breaking and placing",
        "push_out" => "Push-out",
        "crowd" => "Crowd",
        "health" => "Health",
        "item_drops" => "Item drops",
        "inventory" => "Inventory",
        "signals" => "Signals and crystals",
        "world" => "World",
        "time" => "Time of day",
        "saving" => "Saving",
        "combat" => "Combat feel",
        "animation" => "Animation",
        "camera" => "Camera",
        "ao" => "Ambient occlusion",
        "view" => "View",
        "ui" => "UI scale",
        "shading" => "Shading",
        "shadows" => "Shadows",
        "drill" => "Chase drill",
        "sound" => "Sound",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_owned();
    }
    match section.split_once(' ') {
        Some(("tool", name)) => format!("Tool: {name}"),
        Some(("target", name)) => format!("Body: {name}"),
        Some(("brain", name)) => format!("Brain: {name}"),
        Some(("terrain", name)) => format!("Terrain: {name}"),
        _ => section.to_owned(),
    }
}

/// Whether a value matches a search: every word of `query` appears somewhere in its name
/// (underscores read as spaces too), its section, group, help or tab, ignoring case. An
/// empty query matches nothing.
pub fn matches(query: &str, section: &str, t: &Tunable) -> bool {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return false;
    }
    let text = format!(
        "{} {} {} {} {} {} {}",
        t.name,
        t.name.replace('_', " "),
        section,
        heading(section),
        t.group,
        t.help,
        Tab::of(section).label()
    )
    .to_lowercase();
    words.iter().all(|w| text.contains(w.as_str()))
}

/// What a value's tooltip says: its help, range, default and where it's saved.
pub fn tooltip(section: &str, t: &Tunable, default: Option<f32>) -> String {
    let mut out = String::new();
    if !t.help.is_empty() {
        out += &t.help;
        out += "\n\n";
    }
    out += &format!("{} to {}", number(t.min), number(t.max));
    if let Some(d) = default {
        out += &format!(", default {}", number(d));
    }
    out += &format!("\nSaved as [{section}] {}", t.name);
    out += "\nRight-click to reset or favourite";
    out
}

/// Whether `value` is off its default, allowing for float noise at the slider's scale.
pub fn changed(t: &Tunable, default: f32) -> bool {
    (t.value - default).abs() > (t.max - t.min).abs() * 1e-5
}

/// A value as short as it reads, such as `5` or `0.035`.
pub fn number(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".to_owned() } else { s.to_owned() }
}

/// The sliders marked as favourites, by section and name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Favourites(pub BTreeSet<(String, String)>);

impl Favourites {
    pub fn has(&self, section: &str, name: &str) -> bool {
        self.0.contains(&(section.to_owned(), name.to_owned()))
    }

    pub fn toggle(&mut self, section: &str, name: &str) {
        let key = (section.to_owned(), name.to_owned());
        if !self.0.remove(&key) {
            self.0.insert(key);
        }
    }

    /// Reads favourites text: `[section]` lines, then one name a line; `#` starts a comment.
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();
        let mut section = None;
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = Some(name.trim().to_owned());
            } else if let Some(section) = &section {
                out.0.insert((section.clone(), line.to_owned()));
            }
        }
        out
    }

    pub fn text(&self) -> String {
        let mut out = String::from("# Favourite tuning sliders, starred in the tuning panel.\n");
        let mut last: Option<&str> = None;
        for (section, name) in &self.0 {
            if last != Some(section.as_str()) {
                out += &format!("[{section}]\n");
                last = Some(section);
            }
            out += &format!("{name}\n");
        }
        out
    }

    /// The favourites file: `tuning_favourites.txt` beside the tuning file.
    pub fn path() -> Option<PathBuf> {
        Some(tuning_file::path()?.with_file_name("tuning_favourites.txt"))
    }

    /// The saved favourites, or none if there's no file yet.
    pub fn load() -> Self {
        Self::path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<PathBuf> {
        let path = Self::path().ok_or_else(|| std::io::Error::other("no config folder"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, self.text())?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(name: &str) -> Tunable {
        Tunable::new(name, 1.0, 0.0, 2.0)
    }

    #[test]
    fn every_built_in_section_has_a_tab() {
        let sections = [
            "movement", "interaction", "push_out", "crowd", "health", "item_drops", "inventory", "signals",
            "tool base:club", "target base:cow", "brain base:cow", "world", "time", "terrain base:hills", "saving",
            "combat", "animation", "camera", "ao", "view", "ui", "shading", "shadows", "drill", "sound",
        ];
        for s in sections {
            let tab = Tab::of(s);
            assert!(tab != Tab::Other && tab != Tab::Favourites, "{s}");
            assert!(heading(s) != s || s.contains(' '), "{s} has a heading of its own");
        }
        assert_eq!(Tab::of("mymod stuff"), Tab::Other);
        assert_eq!(Tab::of("brain mymod:fox"), Tab::Creatures);
    }

    #[test]
    fn search_matches_every_word_anywhere() {
        let sun = t("sun_size").group("Sky look").help("The sun's disc, as its radius in degrees.");
        assert!(matches("sun", "shading", &sun));
        assert!(matches("SUN size", "shading", &sun), "underscores read as spaces, case ignored");
        assert!(matches("disc radius", "shading", &sun), "help counts");
        assert!(matches("sky look", "shading", &sun), "group counts");
        assert!(matches("light sky", "shading", &sun), "tab counts");
        assert!(matches("shading", "shading", &sun), "section counts");
        assert!(!matches("sun moon", "shading", &sun), "every word must match");
        assert!(!matches("   ", "shading", &sun));
        assert!(matches("cow", "brain base:cow", &t("flee_time")), "a mod's section name counts");
    }

    #[test]
    fn the_tooltip_tells_range_default_and_name() {
        let tip = tooltip("movement", &t("walk_speed").help("Top speed."), Some(5.0));
        assert!(tip.starts_with("Top speed.\n\n0 to 2, default 5\nSaved as [movement] walk_speed"), "{tip}");
        assert!(!tooltip("movement", &t("x"), None).contains("default"));
    }

    #[test]
    fn changed_allows_float_noise() {
        let v = t("a");
        assert!(!changed(&v, 1.000001));
        assert!(changed(&v, 1.01));
    }

    #[test]
    fn favourites_round_trip_through_text() {
        let mut f = Favourites::default();
        f.toggle("brain base:cow", "flee_time");
        f.toggle("movement", "walk_speed");
        f.toggle("movement", "gravity");
        f.toggle("movement", "gravity");
        assert!(f.has("movement", "walk_speed") && !f.has("movement", "gravity"));
        let text = f.text();
        assert_eq!(text.matches("[movement]").count(), 1);
        assert_eq!(Favourites::parse(&text), f);
        assert_eq!(Favourites::parse("stray\n[a]\nb # note\n\n"), Favourites([("a".into(), "b".into())].into()));
    }
}
