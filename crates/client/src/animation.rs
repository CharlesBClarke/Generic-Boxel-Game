//! Playing a model's clips on its skeleton.
//!
//! Which clip plays is the creature's brain's choice, read from the simulation; a creature
//! without one plays its model's first looping clip. The tuning panel can make every creature
//! play one clip instead, for previewing.

use api::{BonePose, Skin, Tunable, Tunables};
use bevy::prelude::*;

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct AnimationTuning {
    /// How fast clips play, as a share of their own speed.
    pub speed: f32,
    /// Seconds a change of clip blends over.
    pub blend: f32,
}

impl Default for AnimationTuning {
    fn default() -> Self {
        Self { speed: 1.0, blend: 0.25 }
    }
}

impl Tunables for AnimationTuning {
    fn tunables(&self) -> Vec<Tunable> {
        vec![Tunable::new("speed", self.speed, 0.0, 3.0), Tunable::new("blend", self.blend, 0.0, 1.0)]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        match name {
            "speed" => self.speed = value,
            "blend" => self.blend = value,
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
}

/// The clip a creature plays when nothing chooses one: its model's first looping clip.
fn fallback(skin: &Skin) -> usize {
    skin.clips.iter().position(|c| c.looping).unwrap_or(0)
}

impl Animator {
    /// Starts the fallback clip at a point set by `seed`, so creatures aren't in step.
    pub fn new(skin: &Skin, seed: u64) -> Self {
        let clip = fallback(skin);
        let length = skin.clips.get(clip).map_or(0.0, |c| c.length);
        Animator { clip, time: random(seed) * length, from: None, faded: 0.0 }
    }

    pub fn clip(&self) -> usize {
        self.clip
    }

    fn play(&mut self, clip: usize) {
        self.from = Some((self.clip, self.time));
        self.faded = 0.0;
        self.clip = clip;
        self.time = 0.0;
    }

    /// Moves on `dt` seconds and plays what's wanted: `preview`'s clip if set, otherwise the
    /// clip named `wanted` at `wanted_speed`, or the fallback when that's empty or unknown. A
    /// clip that doesn't loop plays once when it's first wanted, then holds its end.
    pub fn update(&mut self, skin: &Skin, tuning: &AnimationTuning, preview: Option<&str>, wanted: (&str, f32), dt: f32) {
        let dt = dt * tuning.speed;
        self.faded += dt;
        if let Some((clip, time)) = &mut self.from {
            *time += dt;
            if self.faded >= tuning.blend || skin.clips.get(*clip).is_none() {
                self.from = None;
            }
        }
        if let Some(previewed) = preview.and_then(|name| skin.clip(name)) {
            self.time += dt;
            if previewed != self.clip {
                self.play(previewed);
            } else if skin.clips.get(self.clip).is_some_and(|c| c.finished(self.time - PREVIEW_HOLD)) {
                self.from = None;
                self.time = 0.0;
            }
            return;
        }
        let (name, speed) = wanted;
        let chosen = skin.clip(name).unwrap_or_else(|| fallback(skin));
        if chosen != self.clip {
            self.play(chosen);
        }
        let speed = if speed.is_finite() { speed.max(0.0) } else { 1.0 };
        self.time += dt * speed;
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
        AnimationTuning { blend: 0.0, ..default() }
    }

    const STAND: (&str, f32) = ("stand", 1.0);
    const EAT: (&str, f32) = ("eat", 1.0);

    #[test]
    fn creatures_start_on_the_first_looping_clip_out_of_step() {
        let s = skin();
        let (a, b) = (Animator::new(&s, 1), Animator::new(&s, 2));
        assert_eq!(a.clip(), 1, "stand, the first that loops");
        assert_ne!(a.time, b.time);
        assert!((0.0..2.0).contains(&a.time));
    }

    #[test]
    fn it_plays_the_wanted_clip_and_holds_the_end_of_one_that_does_not_loop() {
        let s = skin();
        let tuning = still();
        let mut a = Animator::new(&s, 7);
        a.update(&s, &tuning, None, STAND, 0.1);
        assert_eq!(a.clip(), 1);
        a.update(&s, &tuning, None, EAT, 0.0);
        assert_eq!(a.clip(), 0, "eat, as soon as it's wanted");
        a.update(&s, &tuning, None, EAT, 0.5);
        assert!((a.pose(&s, &tuning)[0].translation[0] - 0.5).abs() < 1e-4, "halfway through eat");
        a.update(&s, &tuning, None, EAT, 2.0);
        assert_eq!(a.clip(), 0, "still eat while wanted");
        assert!((a.pose(&s, &tuning)[0].translation[0] - 1.0).abs() < 1e-4, "holding its end");
        a.update(&s, &tuning, None, STAND, 0.1);
        a.update(&s, &tuning, None, EAT, 0.25);
        assert!((a.pose(&s, &tuning)[0].translation[0] - 0.25).abs() < 1e-4, "wanted again, it starts over");
    }

    #[test]
    fn an_empty_or_unknown_clip_plays_the_fallback() {
        let s = skin();
        let mut a = Animator::new(&s, 7);
        a.update(&s, &still(), None, EAT, 0.1);
        a.update(&s, &still(), None, ("", 1.0), 0.1);
        assert_eq!(a.clip(), 1);
        a.update(&s, &still(), None, EAT, 0.1);
        a.update(&s, &still(), None, ("moo", 1.0), 0.1);
        assert_eq!(a.clip(), 1);
    }

    #[test]
    fn speed_and_the_wanted_speed_scale_time() {
        let s = skin();
        let tuning = AnimationTuning { speed: 2.0, ..still() };
        let mut a = Animator::new(&s, 7);
        a.update(&s, &tuning, None, EAT, 0.0);
        a.update(&s, &tuning, None, EAT, 0.25);
        assert!((a.pose(&s, &tuning)[0].translation[0] - 0.5).abs() < 1e-4);
        let mut a = Animator::new(&s, 7);
        a.update(&s, &still(), None, ("eat", 0.5), 0.0);
        a.update(&s, &still(), None, ("eat", 0.5), 0.5);
        assert!((a.pose(&s, &still())[0].translation[0] - 0.25).abs() < 1e-4, "half speed");
        a.update(&s, &still(), None, ("eat", 0.0), 0.5);
        assert!((a.pose(&s, &still())[0].translation[0] - 0.25).abs() < 1e-4, "stopped");
    }

    #[test]
    fn a_preview_overrides_the_wanted_clip_and_repeats_one_that_ends() {
        let s = skin();
        let tuning = still();
        let mut a = Animator::new(&s, 7);
        a.update(&s, &tuning, Some("eat"), STAND, 0.1);
        assert_eq!(a.clip(), 0);
        a.update(&s, &tuning, Some("eat"), STAND, 1.2);
        assert_eq!(a.clip(), 0, "a preview beats what's wanted");
        assert!((a.pose(&s, &tuning)[0].translation[0] - 1.0).abs() < 1e-4, "holding the end");
        a.update(&s, &tuning, Some("eat"), STAND, PREVIEW_HOLD);
        assert!(a.time < 0.1, "and starting over");
        a.update(&s, &tuning, Some("nothing"), STAND, 0.1);
        assert_eq!(a.clip(), 1, "an unknown preview goes back to what's wanted");
    }

    #[test]
    fn a_change_of_clip_blends_from_the_last_pose() {
        let s = skin();
        let tuning = AnimationTuning { blend: 0.2, ..still() };
        let mut a = Animator::new(&s, 7);
        let before = a.pose(&s, &tuning)[0].translation;
        a.update(&s, &tuning, None, EAT, 0.0);
        assert!((a.pose(&s, &tuning)[0].translation[2] - before[2]).abs() < 1e-4, "starts where stand was");
        a.update(&s, &tuning, None, EAT, 0.3);
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
