//! Loose items: what a broken block drops, and how it moves until a player picks it up, and
//! what a player carries. The engine owns the bodies, the amounts and the rules; the mod
//! supplies the numbers.

use crate::block::is_valid_name;
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

/// The most a player can be made to carry, in boxels' worth, so amounts in 1/4096ths of a boxel
/// stay well inside 32 bits.
pub const MAX_CAPACITY: f32 = 100_000.0;

/// What a player carries: an amount of each material, all sharing one capacity by volume, if
/// there is one, so carrying many kinds costs no more than carrying one. Placing a block uses up
/// its volume of its material. Without one registered, there's no inventory and placing is free.
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryDef {
    /// How much a player can carry, in boxels' worth of material, every material together.
    /// `None` is no limit.
    pub capacity: Option<f32>,
    /// Speed a thrown item leaves the player at, meters per second, along the look.
    pub throw_speed: f32,
    /// Seconds before the player who threw an item can pick it up again.
    pub pickup_delay: f32,
    /// What a player starts with: material names and boxels' worth of each. Names are checked
    /// once every mod has registered its blocks.
    pub start: Vec<(String, f32)>,
    /// Materials that never run out: kept topped up to [`ENDLESS_AMOUNT`] boxels' worth, past any
    /// capacity, however much is used. Names are checked like `start`'s.
    pub endless: Vec<String>,
}

/// How much of an endless material a player always has, at least, in boxels' worth.
pub const ENDLESS_AMOUNT: f32 = 64.0;

impl InventoryDef {
    /// Checks the values make sense.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(capacity) = self.capacity
            && !(capacity.is_finite() && (0.0..=MAX_CAPACITY).contains(&capacity))
        {
            return Err(format!("capacity must be from 0 to {MAX_CAPACITY}, got {capacity}"));
        }
        let fields = [("throw_speed", self.throw_speed), ("pickup_delay", self.pickup_delay)];
        if let Some((name, value)) = fields.iter().find(|(_, v)| !(v.is_finite() && *v >= 0.0)) {
            return Err(format!("{name} must be at least 0, got {value}"));
        }
        for (name, amount) in &self.start {
            if !is_valid_name(name) {
                return Err(format!("invalid material name {name:?} in start, expected namespace:path"));
            }
            if !(amount.is_finite() && *amount >= 0.0) {
                return Err(format!("start amount of {name} must be at least 0, got {amount}"));
            }
        }
        if let Some(name) = self.endless.iter().find(|n| !is_valid_name(n)) {
            return Err(format!("invalid material name {name:?} in endless, expected namespace:path"));
        }
        let total: f32 = self.start.iter().map(|(_, amount)| amount).sum();
        if let Some(capacity) = self.capacity
            && total > capacity
        {
            return Err(format!("start holds {total} boxels, more than the capacity of {capacity}"));
        }
        Ok(())
    }
}

impl Tunables for InventoryDef {
    fn tunables(&self) -> Vec<Tunable> {
        // No limit has nothing to tune.
        let capacity = self.capacity.map(|c| Tunable::new("capacity", c, 0.0, 256.0));
        capacity
            .into_iter()
            .chain([
                Tunable::new("throw_speed", self.throw_speed, 0.0, 20.0),
                Tunable::new("pickup_delay", self.pickup_delay, 0.0, 5.0),
            ])
            .collect()
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        let field = match name {
            "capacity" => match &mut self.capacity {
                Some(capacity) => capacity,
                None => return false,
            },
            "throw_speed" => &mut self.throw_speed,
            "pickup_delay" => &mut self.pickup_delay,
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

    fn inventory() -> InventoryDef {
        InventoryDef {
            capacity: Some(64.0),
            throw_speed: 6.0,
            pickup_delay: 1.0,
            start: vec![("test:stone".into(), 16.0), ("test:wood".into(), 16.0)],
            endless: vec!["test:copper".into()],
        }
    }

    #[test]
    fn an_inventory_checks_its_values() {
        assert_eq!(inventory().validate(), Ok(()));
        assert!(InventoryDef { capacity: Some(-1.0), ..inventory() }.validate().is_err());
        assert!(InventoryDef { capacity: Some(f32::INFINITY), ..inventory() }.validate().is_err());
        assert_eq!(InventoryDef { capacity: None, ..inventory() }.validate(), Ok(()), "no limit");
        assert!(InventoryDef { pickup_delay: f32::NAN, ..inventory() }.validate().is_err());
        assert!(InventoryDef { capacity: Some(20.0), ..inventory() }.validate().is_err(), "starts with more than fits");
        assert!(InventoryDef { start: vec![("stone".into(), 1.0)], ..inventory() }.validate().is_err(), "a bad name");
        assert!(InventoryDef { start: vec![("test:stone".into(), -1.0)], ..inventory() }.validate().is_err());
        assert!(InventoryDef { endless: vec!["copper".into()], ..inventory() }.validate().is_err(), "a bad endless name");
    }

    #[test]
    fn every_inventory_value_is_tunable() {
        let mut d = inventory();
        for t in d.tunables() {
            assert!(d.set_tunable(&t.name, t.min), "{} sets", t.name);
        }
        assert!(!d.set_tunable("start", 1.0));
        let mut unlimited = InventoryDef { capacity: None, ..inventory() };
        assert!(unlimited.tunables().iter().all(|t| t.name != "capacity"));
        assert!(!unlimited.set_tunable("capacity", 10.0), "no limit stays no limit");
    }
}
