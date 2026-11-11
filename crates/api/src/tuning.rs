//! Live tuning. Anything with values worth adjusting while the game runs lists them here,
//! and the engine's tuning panel draws a slider for each. Plain data both ways, so it works
//! the same across a WebAssembly boundary.

/// One adjustable value.
#[derive(Clone, Debug, PartialEq)]
pub struct Tunable {
    /// Short identifier, such as `"ground_accel"`. Stable, so values can be saved and restored.
    pub name: String,
    pub value: f32,
    pub min: f32,
    pub max: f32,
    /// The small group it's shown in within its section, such as `"Jumping"`; empty for
    /// none. Values that touch the same thing share a group.
    pub group: String,
    /// What it does, in a sentence or two, for the panel's tooltip and search. Optional,
    /// but encouraged.
    pub help: String,
}

impl Tunable {
    pub fn new(name: impl Into<String>, value: f32, min: f32, max: f32) -> Self {
        Self { name: name.into(), value, min, max, group: String::new(), help: String::new() }
    }

    /// This value with help text.
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = help.into();
        self
    }

    /// This value in a group.
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.group = group.into();
        self
    }
}

/// Puts each value named in `groups` into that group, by name. Values no group names keep
/// theirs.
pub fn group(values: &mut [Tunable], groups: &[(&str, &[&str])]) {
    for (title, names) in groups {
        for t in values.iter_mut().filter(|t| names.contains(&t.name.as_str())) {
            t.group = (*title).to_owned();
        }
    }
}

/// Gives the values named in `helps` their help text, by name.
pub fn help(values: &mut [Tunable], helps: &[(&str, &str)]) {
    for (name, text) in helps {
        if let Some(t) = values.iter_mut().find(|t| t.name == *name) {
            t.help = (*text).to_owned();
        }
    }
}

/// `values` with [`group`] and [`help`] applied: a section's `tunables` in one call, its
/// groups and help kept in tables beside it.
pub fn described(mut values: Vec<Tunable>, groups: &[(&str, &[&str])], helps: &[(&str, &str)]) -> Vec<Tunable> {
    group(&mut values, groups);
    help(&mut values, helps);
    values
}

pub trait Tunables {
    /// Every adjustable value with its current setting.
    fn tunables(&self) -> Vec<Tunable>;

    /// Changes one value. Returns false if there is no value by that name.
    fn set_tunable(&mut self, name: &str, value: f32) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_and_help_land_by_name() {
        let mut values = vec![Tunable::new("a", 1.0, 0.0, 2.0), Tunable::new("b", 1.0, 0.0, 2.0).group("kept")];
        group(&mut values, &[("First", &["a"]), ("Missing", &["z"])]);
        help(&mut values, &[("b", "does b"), ("z", "nothing")]);
        assert_eq!(values[0].group, "First");
        assert_eq!(values[1].group, "kept");
        assert_eq!(values[1].help, "does b");
        assert_eq!(values[0].help, "");
    }
}
