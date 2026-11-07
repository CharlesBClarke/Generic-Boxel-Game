//! What the player carries: an amount of each material, in [`FULL_VOLUME`] units, with every
//! material sharing one capacity by volume. The capacity is passed in rather than kept, so it
//! follows the inventory definition while that's tuned; `None` means no limit.

use crate::{FULL_VOLUME, MaterialId};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inventory {
    /// By material index.
    amounts: Vec<u32>,
}

impl Inventory {
    /// How much of `material` is carried.
    pub fn amount(&self, material: MaterialId) -> u32 {
        self.amounts.get(material.index()).copied().unwrap_or(0)
    }

    /// How much is carried, every material together.
    pub fn total(&self) -> u32 {
        self.amounts.iter().fold(0u32, |sum, &a| sum.saturating_add(a))
    }

    /// How much more fits under `capacity`.
    pub fn room(&self, capacity: Option<u32>) -> u32 {
        capacity.map_or(u32::MAX, |c| c.saturating_sub(self.total()))
    }

    /// Adds as much of `amount` of `material` as fits under `capacity`. Returns how much went in.
    pub fn add(&mut self, material: MaterialId, amount: u32, capacity: Option<u32>) -> u32 {
        let taken = amount.min(self.room(capacity));
        if taken > 0 {
            if self.amounts.len() <= material.index() {
                self.amounts.resize(material.index() + 1, 0);
            }
            let held = &mut self.amounts[material.index()];
            *held = held.saturating_add(taken);
        }
        taken
    }

    /// Raises `material` to at least `amount`, whatever the capacity.
    pub fn top_up(&mut self, material: MaterialId, amount: u32) {
        let short = amount.saturating_sub(self.amount(material));
        self.add(material, short, None);
    }

    /// Takes `amount` of `material` out, if that much is carried. Returns whether it was.
    pub fn take(&mut self, material: MaterialId, amount: u32) -> bool {
        match self.amounts.get_mut(material.index()) {
            Some(held) if *held >= amount => {
                *held -= amount;
                true
            }
            _ => amount == 0,
        }
    }

    /// Every material carried, with how much, in material order.
    pub fn iter(&self) -> impl Iterator<Item = (MaterialId, u32)> + '_ {
        self.amounts.iter().enumerate().filter(|&(_, &a)| a > 0).map(|(i, &a)| (MaterialId::from_index(i), a))
    }
}

/// A capacity in boxels' worth, as [`FULL_VOLUME`] units, rounded to the nearest unit.
pub fn capacity_units(boxels: f32) -> u32 {
    (boxels.max(0.0) * FULL_VOLUME as f32).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn materials() -> (MaterialId, MaterialId) {
        (MaterialId::from_index(0), MaterialId::from_index(1))
    }

    #[test]
    fn topping_up_raises_but_never_lowers() {
        let (stone, wood) = materials();
        let mut inv = Inventory::default();
        inv.add(wood, 9, None);
        inv.top_up(stone, 4);
        inv.top_up(wood, 4);
        assert_eq!((inv.amount(stone), inv.amount(wood)), (4, 9));
    }

    #[test]
    fn materials_share_one_capacity() {
        let (stone, wood) = materials();
        let mut inv = Inventory::default();
        let cap = Some(4 * FULL_VOLUME);
        assert_eq!(inv.add(stone, 3 * FULL_VOLUME, cap), 3 * FULL_VOLUME);
        assert_eq!(inv.room(cap), FULL_VOLUME);
        assert_eq!(inv.add(wood, 2 * FULL_VOLUME, cap), FULL_VOLUME, "only what fits goes in");
        assert_eq!((inv.amount(stone), inv.amount(wood)), (3 * FULL_VOLUME, FULL_VOLUME));
        assert_eq!(inv.room(cap), 0);
        assert_eq!(inv.add(stone, 1, cap), 0);
        assert_eq!(inv.add(stone, FULL_VOLUME, None), FULL_VOLUME, "no capacity, no limit");
    }

    #[test]
    fn takes_only_what_is_there() {
        let (stone, wood) = materials();
        let mut inv = Inventory::default();
        inv.add(stone, FULL_VOLUME, None);
        assert!(!inv.take(stone, FULL_VOLUME + 1));
        assert!(!inv.take(wood, 1));
        assert!(inv.take(stone, FULL_VOLUME / 2));
        assert_eq!(inv.amount(stone), FULL_VOLUME / 2);
        assert_eq!(inv.iter().collect::<Vec<_>>(), [(stone, FULL_VOLUME / 2)]);
        assert!(inv.take(stone, FULL_VOLUME / 2));
        assert_eq!(inv.iter().count(), 0, "empty materials aren't listed");
    }

    #[test]
    fn a_lowered_capacity_keeps_what_is_carried() {
        let (stone, _) = materials();
        let mut inv = Inventory::default();
        inv.add(stone, 10 * FULL_VOLUME, None);
        assert_eq!(inv.room(Some(capacity_units(4.0))), 0);
        assert_eq!(inv.amount(stone), 10 * FULL_VOLUME);
        assert_eq!(capacity_units(0.5), FULL_VOLUME / 2);
    }
}
