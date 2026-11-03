//! Playing a model's clips on its skeleton.
//!
//! Which clip plays is decided here for now: a creature loops `stand` and now and then plays
//! `eat`. That's game logic in the client (API debt, "Clip choice in the client"), until a
//! creature's behavior can say which clip it wants.

use api::{BonePose, Skin, Tunable, Tunables};
use bevy::prelude::*;

/// The clip a creature idles in, and the one it grazes with now and then.
const IDLE: &str = "stand";
const GRAZE: &str = "eat";

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct AnimationTuning {
    /// How fast clips play, as a share of their own speed.
    pub speed: f32,
    /// Seconds a change of clip blends over.
    pub blend: f32,
    /// Seconds between grazes: at least `graze_min`, at most `graze_min + graze_spread`.
    pub graze_min: f32,
    pub graze_spread: f32,
}

impl Default for AnimationTuning {
    fn default() -> Self {
        Self { speed: 1.0, blend: 0.25, graze_min: 6.0, graze_spread: 10.0 }
    }
}

impl Tunables for AnimationTuning {
    fn tunables(&self) -> Vec<Tunable> {
        vec![
            Tunable::new("speed", self.speed, 0.0, 3.0),
            Tunable::new("blend", self.blend, 0.0, 1.0),
            Tunable::new("graze_min", self.graze_min, 0.5, 30.0),
            Tunable::new("graze_spread", self.graze_spread, 0.0, 30.0),
        ]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "speed" => self.speed = value,
            "blend" => self.blend = value,
            "graze_min" => self.graze_min = value,
            "graze_spread" => self.graze_spread = value,
            _ => return false,
        }
        true
    }
}

/// A clip every creature plays instead of choosing, from the tuning panel.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct AnimationPreview {
    pub clip: Option<String>,
}

/// Seconds a previewed clip that doesn't loop holds its end before playing again.
const PREVIEW_HOLD: f32 = 0.6;

/// One creature's place in its clips.
#[derive(Clone, Debug, PartialEq)]
pub struct Animator {
    clip: usize,
    /// Seconds into `clip`, at its own speed.
    time: f32,
    /// The clip and time being blended away from, and seconds of blending done.
    from: Option<(usize, f32)>,
    faded: f32,
    /// Seconds until the next graze.
    graze_in: f32,
    /// For this creature's random timings.
    seed: u64,
}

impl Animator {
    /// Starts idling at a point set by `seed`, so creatures aren't in step.
    pub fn new(skin: &Skin, seed: u64, tuning: &AnimationTuning) -> Self {
        let clip = skin.clip(IDLE).unwrap_or(0);
        let length = skin.clips.get(clip).map_or(0.0, |c| c.length);
        let mut animator = Animator { clip, time: random(seed) * length, from: None, faded: 0.0, graze_in: 0.0, seed };
        animator.graze_in = animator.next_graze(tuning);
        animator
    }

    pub fn clip(&self) -> usize {
        self.clip
    }

    fn next_graze(&mut self, tuning: &AnimationTuning) -> f32 {
        self.seed = self.seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        tuning.graze_min + random(self.seed) * tuning.graze_spread
    }

    fn play(&mut self, clip: usize) {
        self.from = Some((self.clip, self.time));
        self.faded = 0.0;
        self.clip = clip;
        self.time = 0.0;
    }

    /// Moves on `dt` seconds and picks what plays next: `preview`'s clip if set, otherwise
    /// idling with a graze now and then.
    pub fn update(&mut self, skin: &Skin, tuning: &AnimationTuning, preview: Option<&str>, dt: f32) {
        let dt = dt * tuning.speed;
        self.time += dt;
        self.faded += dt;
        if let Some((clip, time)) = &mut self.from {
            *time += dt;
            if self.faded >= tuning.blend || skin.clips.get(*clip).is_none() {
                self.from = None;
            }
        }
        let Some(current) = skin.clips.get(self.clip) else {
            return;
        };
        if let Some(wanted) = preview.and_then(|name| skin.clip(name)) {
            if wanted != self.clip {
                self.play(wanted);
            } else if current.finished(self.time - PREVIEW_HOLD) {
                self.from = None;
                self.time = 0.0;
            }
            return;
        }
        let idle = skin.clip(IDLE).unwrap_or(0);
        if self.clip != idle {
            if current.looping || current.finished(self.time) {
                self.play(idle);
            }
            return;
        }
        self.graze_in -= dt;
        if self.graze_in <= 0.0 {
            self.graze_in = self.next_graze(tuning);
            if let Some(graze) = skin.clip(GRAZE) {
                self.play(graze);
            }
        }
    }

    /// Every bone's pose now, relative to its parent.
    pub fn pose(&self, skin: &Skin, tuning: &AnimationTuning) -> Vec<BonePose> {
        let now = skin.pose(self.clip, self.time);
        match self.from {
            Some((clip, time)) if tuning.blend > 0.0 => {
                let f = (self.faded / tuning.blend).clamp(0.0, 1.0);
                skin.pose(clip, time).iter().zip(&now).map(|(a, b)| a.blend(b, f)).collect()
            }
            _ => now,
        }
    }
}

/// A number from 0 to 1 picked by `seed`.
fn random(seed: u64) -> f32 {
    let mut x = seed.wrapping_mul(0x2545_f491_4f6c_dd1d) ^ 0x5851_f42d_4c95_7f2d;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// A bone's pose as a Bevy transform, in the model's own Z-up space.
pub fn transform(pose: &BonePose) -> Transform {
    Transform {
        translation: Vec3::from_array(pose.translation),
        rotation: Quat::from_array(pose.rotation).normalize(),
        scale: Vec3::from_array(pose.scale),
    }
}

#[cfg(test)]
mod tests {
    use api::{Bone, Clip, Track};

    use super::*;

    const REST: BonePose = BonePose { translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] };

    /// One bone; `stand` slides it up 1 m over 2 s, `eat` slides it sideways 1 m over 1 s and
    /// doesn't loop.
    fn skin() -> Skin {
        let clip = |name: &str, length: f32, looping: bool, to: [f32; 3]| Clip {
            name: name.into(),
            length,
            looping,
            tracks: vec![Track { bone: 0, translations: vec![(0.0, [0.0; 3]), (length, to)], rotations: vec![] }],
        };
        Skin {
            joints: vec![],
            weights: vec![],
            bones: vec![Bone { name: "b".into(), parent: None, rest: REST, inverse_bind: [0.0; 16] }],
            clips: vec![clip("eat", 1.0, false, [1.0, 0.0, 0.0]), clip("stand", 2.0, true, [0.0, 0.0, 1.0])],
        }
    }

    fn still() -> AnimationTuning {
        AnimationTuning { blend: 0.0, graze_min: 3.0, graze_spread: 0.0, ..default() }
    }

    #[test]
    fn creatures_start_idling_out_of_step() {
        let s = skin();
        let a = Animator::new(&s, 1, &still());
        let b = Animator::new(&s, 2, &still());
        assert_eq!(a.clip(), 1, "stand");
        assert_ne!(a.time, b.time);
        assert!((0.0..2.0).contains(&a.time));
    }

    #[test]
    fn an_idling_creature_grazes_then_goes_back_to_idling() {
        let s = skin();
        let tuning = still();
        let mut a = Animator::new(&s, 7, &tuning);
        a.update(&s, &tuning, None, 2.9);
        assert_eq!(a.clip(), 1, "not yet");
        a.update(&s, &tuning, None, 0.2);
        assert_eq!(a.clip(), 0, "grazing after graze_min");
        a.update(&s, &tuning, None, 0.5);
        assert!((a.pose(&s, &tuning)[0].translation[0] - 0.5).abs() < 1e-4, "halfway through eat");
        a.update(&s, &tuning, None, 0.6);
        assert_eq!(a.clip(), 1, "back to stand once eat ends");
    }

    #[test]
    fn speed_scales_time() {
        let s = skin();
        let tuning = AnimationTuning { speed: 2.0, ..still() };
        let mut a = Animator::new(&s, 7, &tuning);
        a.update(&s, &tuning, Some("eat"), 0.0);
        a.update(&s, &tuning, Some("eat"), 0.25);
        assert!((a.pose(&s, &tuning)[0].translation[0] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn a_preview_plays_its_clip_and_repeats_one_that_ends() {
        let s = skin();
        let tuning = still();
        let mut a = Animator::new(&s, 7, &tuning);
        a.update(&s, &tuning, Some("eat"), 0.1);
        assert_eq!(a.clip(), 0);
        a.update(&s, &tuning, Some("eat"), 1.2);
        assert_eq!(a.clip(), 0, "a preview doesn't go back to idling");
        assert!((a.pose(&s, &tuning)[0].translation[0] - 1.0).abs() < 1e-4, "holding the end");
        a.update(&s, &tuning, Some("eat"), PREVIEW_HOLD);
        assert!(a.time < 0.1, "and starting over");
        a.update(&s, &tuning, Some("nothing"), 0.1);
        a.update(&s, &tuning, None, 5.0);
        assert_eq!(a.clip(), 1, "an unknown clip or no preview goes back to choosing");
    }

    #[test]
    fn a_change_of_clip_blends_from_the_last_pose() {
        let s = skin();
        let tuning = AnimationTuning { blend: 0.2, ..still() };
        let mut a = Animator::new(&s, 7, &tuning);
        let before = a.pose(&s, &tuning)[0].translation;
        a.update(&s, &tuning, Some("eat"), 0.0);
        assert!((a.pose(&s, &tuning)[0].translation[2] - before[2]).abs() < 1e-4, "starts where stand was");
        a.update(&s, &tuning, Some("eat"), 0.3);
        assert!((a.pose(&s, &tuning)[0].translation[2]).abs() < 1e-4, "all eat once the blend is done");
    }

    #[test]
    fn every_value_is_tunable() {
        let mut t = AnimationTuning::default();
        for tunable in t.tunables() {
            assert!(t.set_tunable(&tunable.name, tunable.min), "{}", tunable.name);
        }
        assert!(!t.set_tunable("nope", 1.0));
    }
}
