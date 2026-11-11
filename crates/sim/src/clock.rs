//! The time of day: hours from 0 to 24, advanced every tick at a tunable day length, and
//! saved with the world. The simulation owns it so every client sees the same sky, and so
//! creatures can act on it later; it changes nothing in the simulation yet.

use api::{Tunable, Tunables};

/// Where a world's day stands, and how fast it goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    /// Hours since midnight, 0 up to (not including) 24.
    hours: f64,
    /// How many real seconds a whole day takes.
    pub day_seconds: f32,
    /// Stopped, the time holds where it is.
    pub paused: bool,
}

impl Clock {
    /// The hour a world starts at when nothing set it: mid-morning.
    pub const START_HOURS: f64 = 9.0;
    /// The shortest and longest days the tuning allows, in seconds.
    pub const SHORTEST_DAY: f32 = 10.0;
    pub const LONGEST_DAY: f32 = 4.0 * 3600.0;

    pub fn hours(&self) -> f64 {
        self.hours
    }

    /// Sets the time, wrapped into a day.
    pub fn set_hours(&mut self, hours: f64) {
        self.hours = if hours.is_finite() { hours.rem_euclid(24.0) } else { Self::START_HOURS };
        // `rem_euclid` can round up to 24 itself for a tiny negative input.
        if self.hours >= 24.0 {
            self.hours = 0.0;
        }
    }

    /// Advances `seconds` of real time, unless paused.
    pub fn step(&mut self, seconds: f32) {
        if !self.paused {
            let day = self.day_seconds.max(Self::SHORTEST_DAY) as f64;
            self.set_hours(self.hours + seconds as f64 * 24.0 / day);
        }
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self { hours: Self::START_HOURS, day_seconds: 120.0, paused: false }
    }
}

impl Tunables for Clock {
    fn tunables(&self) -> Vec<Tunable> {
        vec![Tunable::new("day_seconds", self.day_seconds, Self::SHORTEST_DAY, Self::LONGEST_DAY)]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "day_seconds" => self.day_seconds = value.clamp(Self::SHORTEST_DAY, Self::LONGEST_DAY),
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_takes_day_seconds() {
        let mut clock = Clock { day_seconds: 120.0, ..Clock::default() };
        clock.set_hours(0.0);
        clock.step(30.0);
        assert!((clock.hours() - 6.0).abs() < 1e-9);
        clock.step(60.0);
        assert!((clock.hours() - 18.0).abs() < 1e-9);
    }

    #[test]
    fn the_time_wraps_past_midnight() {
        let mut clock = Clock { day_seconds: 120.0, ..Clock::default() };
        clock.set_hours(23.0);
        clock.step(10.0);
        assert!((clock.hours() - 1.0).abs() < 1e-9);
        clock.set_hours(-1.0);
        assert!((clock.hours() - 23.0).abs() < 1e-9);
        clock.set_hours(48.5);
        assert!((clock.hours() - 0.5).abs() < 1e-9);
        clock.set_hours(-1e-18);
        assert!(clock.hours() < 24.0);
        clock.set_hours(f64::NAN);
        assert_eq!(clock.hours(), Clock::START_HOURS);
    }

    #[test]
    fn paused_holds_the_time() {
        let mut clock = Clock { paused: true, ..Clock::default() };
        clock.step(100.0);
        assert_eq!(clock.hours(), Clock::START_HOURS);
    }

    #[test]
    fn many_ticks_add_up_to_a_day() {
        let mut clock = Clock { day_seconds: 120.0, ..Clock::default() };
        clock.set_hours(0.0);
        for _ in 0..120 * 60 {
            clock.step(1.0 / 60.0);
        }
        // Back to midnight, give or take the tick's f32 rounding (a few milliseconds a day).
        let off = clock.hours().min(24.0 - clock.hours());
        assert!(off < 1e-5, "{}", clock.hours());
    }

    #[test]
    fn day_length_is_clamped() {
        let mut clock = Clock::default();
        assert!(clock.set_tunable("day_seconds", 1.0));
        assert_eq!(clock.day_seconds, Clock::SHORTEST_DAY);
        assert!(clock.set_tunable("day_seconds", 1e9));
        assert_eq!(clock.day_seconds, Clock::LONGEST_DAY);
        assert!(!clock.set_tunable("nope", 1.0));
    }
}
