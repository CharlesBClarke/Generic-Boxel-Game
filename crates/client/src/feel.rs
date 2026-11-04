//! Camera feel: FOV that widens with speed, head bob, a robotic shock-absorber on landing,
//! and smoothing over step-ups and crouching, so the eyes move like a hydraulic. Render-only, so it never affects the simulation.
//!
//! Landings are a damped suspension. A soft landing compresses it smoothly in proportion to
//! the impact. A landing at or above `hard_landing_speed` bottoms it out instantly with a
//! nod and a slow recovery, so a fall that would hurt reads differently from one that wouldn't.

use api::{Tunable, Tunables};
use bevy::prelude::*;

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct CameraFeel {
    /// Field of view at rest, degrees.
    pub fov: f32,
    /// Extra degrees per m/s of horizontal speed above `fov_speed_start`.
    pub fov_per_speed: f32,
    pub fov_speed_start: f32,
    pub fov_max: f32,
    /// How quickly the FOV follows its target, per second.
    pub fov_response: f32,
    /// Head bob height, meters.
    pub bob_amount: f32,
    /// Bob cycles per meter walked.
    pub bob_per_meter: f32,
    /// How much of the landing speed the suspension takes on, as a fraction.
    pub landing_absorb: f32,
    /// Suspension spring stiffness, per second squared.
    pub landing_stiffness: f32,
    /// Suspension damping ratio: below 1 bounces, 1 settles without bouncing.
    pub landing_damping: f32,
    /// How far the suspension can compress, meters.
    pub landing_max_travel: f32,
    /// Landing speed, m/s, at which the suspension bottoms out: "this fall hurts". With fall
    /// damage, the health definition's `fall_speed` is used instead.
    pub hard_landing_speed: f32,
    /// Spring stiffness while recovering from a hard landing; lower is slower.
    pub hard_recover_stiffness: f32,
    /// Downward nod per m/s of landing speed on soft landings, degrees.
    pub landing_nod: f32,
    /// Downward nod on hard landings, degrees.
    pub hard_nod: f32,
    /// How quickly a nod recovers, per second.
    pub nod_recover: f32,
    /// How quickly the camera catches up after a step-up, per second.
    pub step_recover: f32,
    /// How quickly the camera follows crouching and standing, per second.
    pub crouch_recover: f32,
}

impl Default for CameraFeel {
    fn default() -> Self {
        Self {
            fov: 80.0,
            fov_per_speed: 1.5,
            fov_speed_start: 5.5,
            fov_max: 105.0,
            fov_response: 8.0,
            bob_amount: 0.035,
            bob_per_meter: 0.35,
            landing_absorb: 0.34,
            landing_stiffness: 150.0,
            landing_damping: 0.7,
            landing_max_travel: 0.45,
            // About a 5.7-boxel fall with base's gravity; jumping off a 3-boxel ledge stays soft.
            hard_landing_speed: 20.0,
            hard_recover_stiffness: 35.0,
            landing_nod: 0.12,
            hard_nod: 5.0,
            nod_recover: 6.0,
            step_recover: 10.0,
            crouch_recover: 12.0,
        }
    }
}

macro_rules! tunable_fields {
    ($($field:ident: $min:expr, $max:expr;)*) => {
        impl Tunables for CameraFeel {
            fn tunables(&self) -> Vec<Tunable> {
                vec![$(Tunable::new(stringify!($field), self.$field, $min, $max)),*]
            }

            fn set_tunable(&mut self, name: &str, value: f32) -> bool {
                match name {
                    $(stringify!($field) => self.$field = value,)*
                    _ => return false,
                }
                true
            }
        }
    };
}

tunable_fields! {
    fov: 50.0, 120.0;
    fov_per_speed: 0.0, 6.0;
    fov_speed_start: 0.0, 20.0;
    fov_max: 50.0, 140.0;
    fov_response: 0.5, 30.0;
    bob_amount: 0.0, 0.2;
    bob_per_meter: 0.05, 1.5;
    landing_absorb: 0.0, 1.0;
    landing_stiffness: 10.0, 500.0;
    landing_damping: 0.1, 2.0;
    landing_max_travel: 0.0, 1.2;
    hard_landing_speed: 3.0, 60.0;
    hard_recover_stiffness: 5.0, 300.0;
    landing_nod: 0.0, 1.0;
    hard_nod: 0.0, 20.0;
    nod_recover: 0.5, 30.0;
    step_recover: 1.0, 40.0;
    crouch_recover: 1.0, 40.0;
}

/// The camera effects' running state.
#[derive(Resource, Debug)]
pub struct FeelState {
    pub fov: f32,
    bob_phase: f32,
    bob_weight: f32,
    /// Suspension compression, meters, 0 at rest and negative when compressed.
    dip: f32,
    dip_velocity: f32,
    /// Recovering from a bottomed-out landing.
    hard: bool,
    /// Downward nod, degrees.
    nod: f32,
    /// Leftover height from step-ups, meters, easing to 0.
    step: f32,
    /// Leftover eye movement from crouching or standing, meters, easing to 0.
    crouch: f32,
    seen_landings: u32,
    seen_stepped: f64,
    seen_crouch_shift: f64,
}

impl Default for FeelState {
    fn default() -> Self {
        Self {
            fov: CameraFeel::default().fov,
            bob_phase: 0.0,
            bob_weight: 0.0,
            dip: 0.0,
            dip_velocity: 0.0,
            hard: false,
            nod: 0.0,
            step: 0.0,
            crouch: 0.0,
            seen_landings: 0,
            seen_stepped: 0.0,
            seen_crouch_shift: 0.0,
        }
    }
}

/// What the camera effects need to know about the player this frame.
pub struct Motion {
    pub horizontal_speed: f32,
    pub grounded: bool,
    pub landings: u32,
    pub last_landing_speed: f32,
    pub stepped_total: f64,
    pub crouch_shift_total: f64,
}

/// Spring steps are capped at this length so the suspension stays stable at low frame rates.
const MAX_SPRING_STEP: f32 = 1.0 / 240.0;

impl FeelState {
    /// Advances the effects by `dt` seconds of render time.
    pub fn update(&mut self, feel: &CameraFeel, motion: &Motion, dt: f32) {
        let extra = (motion.horizontal_speed - feel.fov_speed_start).max(0.0) * feel.fov_per_speed;
        let target = (feel.fov + extra).min(feel.fov_max.max(feel.fov));
        self.fov += (target - self.fov) * (1.0 - (-feel.fov_response * dt).exp());

        // Bob only while walking on the ground, fading in and out rather than snapping.
        self.bob_phase += motion.horizontal_speed * feel.bob_per_meter * std::f32::consts::TAU * dt;
        let bob_target = if motion.grounded { (motion.horizontal_speed / 4.0).min(1.0) } else { 0.0 };
        self.bob_weight += (bob_target - self.bob_weight) * (1.0 - (-10.0 * dt).exp());

        if motion.landings != self.seen_landings {
            self.seen_landings = motion.landings;
            self.land(feel, motion.last_landing_speed);
        }
        self.spring(feel, dt);
        self.nod *= (-feel.nod_recover * dt).exp();

        // A step up moves the body instantly; start the camera where it was and ease it up.
        let stepped = (motion.stepped_total - self.seen_stepped) as f32;
        self.seen_stepped = motion.stepped_total;
        self.step = (self.step - stepped) * (-feel.step_recover * dt).exp();

        // Crouching moves the eyes instantly too; ease them the same way.
        let shifted = (motion.crouch_shift_total - self.seen_crouch_shift) as f32;
        self.seen_crouch_shift = motion.crouch_shift_total;
        self.crouch = (self.crouch - shifted) * (-feel.crouch_recover * dt).exp();
    }

    fn land(&mut self, feel: &CameraFeel, speed: f32) {
        if speed >= feel.hard_landing_speed {
            // Bottom out: full travel at once, no easing in.
            self.dip = -feel.landing_max_travel;
            self.dip_velocity = 0.0;
            self.hard = true;
            self.nod = self.nod.max(feel.hard_nod);
        } else {
            self.dip_velocity -= speed * feel.landing_absorb;
            self.hard = false;
            self.nod = self.nod.max(speed * feel.landing_nod);
        }
    }

    fn spring(&mut self, feel: &CameraFeel, dt: f32) {
        let steps = (dt / MAX_SPRING_STEP).ceil().max(1.0);
        let h = dt / steps;
        for _ in 0..steps as u32 {
            let (stiffness, damping) = if self.hard {
                (feel.hard_recover_stiffness, 1.0)
            } else {
                (feel.landing_stiffness, feel.landing_damping)
            };
            let c = 2.0 * damping * stiffness.sqrt();
            self.dip_velocity += (-stiffness * self.dip - c * self.dip_velocity) * h;
            self.dip += self.dip_velocity * h;
            if self.dip < -feel.landing_max_travel {
                self.dip = -feel.landing_max_travel;
                self.dip_velocity = self.dip_velocity.max(0.0);
            }
        }
        if self.hard && self.dip.abs() < 0.005 {
            self.hard = false;
        }
    }

    /// Vertical camera offset this frame, meters.
    pub fn vertical_offset(&self, feel: &CameraFeel) -> f32 {
        self.dip + self.step + self.crouch - self.bob_phase.sin().abs() * feel.bob_amount * self.bob_weight
    }

    /// Downward pitch offset this frame, radians.
    pub fn nod(&self) -> f32 {
        self.nod.to_radians()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn motion(speed: f32, grounded: bool) -> Motion {
        Motion {
            horizontal_speed: speed,
            grounded,
            landings: 0,
            last_landing_speed: 0.0,
            stepped_total: 0.0,
            crouch_shift_total: 0.0,
        }
    }

    fn landed(speed: f32) -> Motion {
        Motion { landings: 1, last_landing_speed: speed, ..motion(0.0, true) }
    }

    /// The deepest the camera goes after a landing at `speed`, and the offset on the first frame.
    fn dip_after(speed: f32) -> (f32, f32) {
        let feel = CameraFeel::default();
        let mut state = FeelState::default();
        state.update(&feel, &landed(speed), DT);
        let first = state.vertical_offset(&feel);
        let mut deepest = first;
        for _ in 0..60 {
            state.update(&feel, &landed(speed), DT);
            deepest = deepest.min(state.vertical_offset(&feel));
        }
        (deepest, first)
    }

    #[test]
    fn fov_widens_with_speed_up_to_the_cap() {
        let feel = CameraFeel::default();
        let mut state = FeelState::default();
        for _ in 0..600 {
            state.update(&feel, &motion(3.0, true), DT);
        }
        assert!((state.fov - feel.fov).abs() < 0.01);
        for _ in 0..600 {
            state.update(&feel, &motion(100.0, false), DT);
        }
        assert!((state.fov - feel.fov_max).abs() < 0.01);
    }

    #[test]
    fn soft_landings_compress_more_the_harder_they_are() {
        let max = CameraFeel::default().landing_max_travel;
        let (jump, _) = dip_after(9.0);
        let (drop, _) = dip_after(14.0);
        assert!(jump < -0.05, "a jump landing should be felt: {jump}");
        assert!(drop < jump, "a bigger drop should dip deeper: {drop} vs {jump}");
        assert!(drop > -max, "a soft landing should not bottom out: {drop}");
    }

    #[test]
    fn soft_landings_ease_in() {
        let (deepest, first) = dip_after(12.0);
        assert!(first > deepest * 0.5, "first frame {first} should be well short of the deepest {deepest}");
    }

    #[test]
    fn hard_landings_bottom_out_at_once() {
        let feel = CameraFeel::default();
        let (deepest, first) = dip_after(feel.hard_landing_speed + 1.0);
        assert!((first + feel.landing_max_travel).abs() < 0.02, "first frame {first}");
        assert_eq!(deepest, first, "a hard landing starts at the bottom and only recovers from there");
    }

    #[test]
    fn landings_recover() {
        let feel = CameraFeel::default();
        for speed in [9.0, 30.0] {
            let mut state = FeelState::default();
            for _ in 0..240 {
                state.update(&feel, &landed(speed), DT);
            }
            assert!(state.vertical_offset(&feel).abs() < 5e-3, "speed {speed}");
            assert!(state.nod() < 1e-3);
        }
    }

    #[test]
    fn the_suspension_is_stable_at_low_frame_rates() {
        let feel = CameraFeel::default();
        let mut state = FeelState::default();
        for _ in 0..30 {
            state.update(&feel, &landed(12.0), 1.0 / 15.0);
            assert!(state.vertical_offset(&feel).abs() <= feel.landing_max_travel + 1e-3);
        }
    }

    #[test]
    fn step_ups_are_smoothed() {
        let feel = CameraFeel::default();
        let mut state = FeelState::default();
        let stepped = Motion { stepped_total: 1.0, ..motion(0.0, true) };
        state.update(&feel, &stepped, DT);
        assert!(state.vertical_offset(&feel) < -0.8, "the camera should start near where it was");
    }

    #[test]
    fn crouching_eases_the_eyes_down() {
        let feel = CameraFeel::default();
        let mut state = FeelState::default();
        let crouched = Motion { crouch_shift_total: -0.8, ..motion(0.0, true) };
        state.update(&feel, &crouched, DT);
        let first = state.vertical_offset(&feel);
        assert!(first > 0.6, "the camera should start near standing height: {first}");
        for _ in 0..60 {
            state.update(&feel, &crouched, DT);
        }
        assert!(state.vertical_offset(&feel).abs() < 0.01, "and settle at the crouched eyes");
    }

    #[test]
    fn tunables_round_trip() {
        let mut feel = CameraFeel::default();
        for t in feel.tunables() {
            assert!(t.min <= t.value && t.value <= t.max, "{} default outside its range", t.name);
            assert!(feel.set_tunable(&t.name, t.max));
        }
        assert!(feel.tunables().iter().all(|t| t.value == t.max));
        assert!(!feel.set_tunable("nonsense", 1.0));
    }
}
