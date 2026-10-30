//! Loose items: what a broken block drops, and how it moves until a player picks it up. The
//! engine owns the bodies and the rules; the mod supplies the numbers.

use crate::{Tunable, Tunables};

/// How loose items behave. Without one registered, breaking a block drops nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemDropDef {
    /// Speed a dropped item pops out of the broken block at, meters per second, mostly upward.
    pub pop_speed: f32,
    /// Downward acceleration, meters per second squared.
    pub gravity: f32,
    /// Share of its speed an item keeps when it hits something: 0 stops dead, 1 bounces forever.
    pub bounce: f32,
    /// How fast an item on the ground slows down sideways, per second.
    pub drag: f32,
    /// Edge length of the item for a full block's worth of material, in meters. More or less
    /// material makes it bigger or smaller.
    pub size: f32,
    /// How close to a player's body an item has to be to get pulled toward it, in meters.
    pub pull_radius: f32,
    /// Speed of the pull, meters per second.
    pub pull_speed: f32,
    /// How close to a player's body an item has to be to get picked up, in meters.
    pub pickup_radius: f32,
    /// Items of the same material closer than this merge into one, in meters.
    pub merge_radius: f32,
}

impl ItemDropDef {
    /// Checks the values make sense.
    pub fn validate(&self) -> Result<(), String> {
        let fields = [
            ("pop_speed", self.pop_speed),
            ("gravity", self.gravity),
            ("drag", self.drag),
            ("pull_radius", self.pull_radius),
            ("pull_speed", self.pull_speed),
            ("pickup_radius", self.pickup_radius),
            ("merge_radius", self.merge_radius),
        ];
        if let Some((name, value)) = fields.iter().find(|(_, v)| !(v.is_finite() && *v >= 0.0)) {
            return Err(format!("{name} must be at least 0, got {value}"));
        }
        if !(0.0..=1.0).contains(&self.bounce) {
            return Err(format!("bounce must be from 0 to 1, got {}", self.bounce));
        }
        if !(self.size.is_finite() && self.size > 0.0 && self.size <= 1.0) {
            return Err(format!("size must be above 0 and at most 1, got {}", self.size));
        }
        Ok(())
    }
}

impl Tunables for ItemDropDef {
    fn tunables(&self) -> Vec<Tunable> {
        vec![
            Tunable::new("pop_speed", self.pop_speed, 0.0, 15.0),
            Tunable::new("gravity", self.gravity, 0.0, 60.0),
            Tunable::new("bounce", self.bounce, 0.0, 1.0),
            Tunable::new("drag", self.drag, 0.0, 30.0),
            Tunable::new("size", self.size, 0.05, 1.0),
            Tunable::new("pull_radius", self.pull_radius, 0.0, 10.0),
            Tunable::new("pull_speed", self.pull_speed, 0.0, 30.0),
            Tunable::new("pickup_radius", self.pickup_radius, 0.0, 3.0),
            Tunable::new("merge_radius", self.merge_radius, 0.0, 3.0),
        ]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        let field = match name {
            "pop_speed" => &mut self.pop_speed,
            "gravity" => &mut self.gravity,
            "bounce" => &mut self.bounce,
            "drag" => &mut self.drag,
            "size" => &mut self.size,
            "pull_radius" => &mut self.pull_radius,
            "pull_speed" => &mut self.pull_speed,
            "pickup_radius" => &mut self.pickup_radius,
            "merge_radius" => &mut self.merge_radius,
            _ => return false,
        };
        *field = value;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def() -> ItemDropDef {
        ItemDropDef {
            pop_speed: 3.0,
            gravity: 20.0,
            bounce: 0.3,
            drag: 8.0,
            size: 0.25,
            pull_radius: 2.0,
            pull_speed: 8.0,
            pickup_radius: 0.4,
            merge_radius: 1.0,
        }
    }

    #[test]
    fn checks_its_values() {
        assert_eq!(def().validate(), Ok(()));
        assert!(ItemDropDef { gravity: -1.0, ..def() }.validate().is_err());
        assert!(ItemDropDef { bounce: 1.5, ..def() }.validate().is_err());
        assert!(ItemDropDef { size: 0.0, ..def() }.validate().is_err());
        assert!(ItemDropDef { pull_speed: f32::NAN, ..def() }.validate().is_err());
    }

    #[test]
    fn every_value_is_tunable() {
        let mut d = def();
        for t in d.tunables() {
            assert!(d.set_tunable(&t.name, t.min), "{} sets", t.name);
        }
        assert!(!d.set_tunable("missing", 1.0));
    }
}
