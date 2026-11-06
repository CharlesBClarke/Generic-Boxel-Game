//! Models and sounds: media a mod hands the engine as plain data, for its content to name. The
//! simulation never looks inside them; a client draws and plays them.

use glam::{Quat, Vec3};

/// A textured triangle mesh, optionally bent by a skeleton.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelDef {
    /// Namespaced, such as `"base:cow"`.
    pub name: String,
    /// Vertex positions in meters, +Z up, with the feet at the origin and the front facing +Y.
    pub positions: Vec<[f32; 3]>,
    /// Texture coordinates for each vertex, from the texture's top left corner.
    pub uvs: Vec<[f32; 2]>,
    /// Three vertex indices per triangle, counterclockwise seen from outside.
    pub indices: Vec<u32>,
    pub texture: Texture,
    /// Bones and animation clips, for a model that moves.
    pub skin: Option<Skin>,
}

/// A model's skeleton and its clips. Positions are in meters, Z up, like the mesh's.
#[derive(Clone, Debug, PartialEq)]
pub struct Skin {
    /// For each vertex, up to four bones it follows, indices into `bones`.
    pub joints: Vec<[u16; 4]>,
    /// How much each of those bones moves the vertex; they add up to 1.
    pub weights: Vec<[f32; 4]>,
    /// Every bone comes after its parent.
    pub bones: Vec<Bone>,
    pub clips: Vec<Clip>,
}

/// One bone, posed relative to its parent.
#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    pub name: String,
    /// An index into the skin's bones, always lower than this one's; `None` for a root, which is
    /// posed relative to the model.
    pub parent: Option<u16>,
    /// Its pose when no clip moves it.
    pub rest: BonePose,
    /// Takes the model's space into this bone's, as it was when the mesh was bound to it:
    /// a 4×4 matrix, column by column.
    pub inverse_bind: [f32; 16],
}

/// A bone's place relative to its parent: scaled, then turned, then moved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BonePose {
    pub translation: [f32; 3],
    /// A unit quaternion as x, y, z, w.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

/// A named animation, such as `"walk"`.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub name: String,
    /// Seconds.
    pub length: f32,
    /// Whether it starts over at the end, or holds its last pose.
    pub looping: bool,
    pub tracks: Vec<Track>,
}

/// How one bone moves through a clip. Between keys it blends evenly; before the first and after
/// the last it holds. With no keys of a kind the bone keeps its rest value.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub bone: u16,
    /// Seconds into the clip and the bone's translation then.
    pub translations: Vec<(f32, [f32; 3])>,
    /// Seconds into the clip and the bone's rotation then, x, y, z, w.
    pub rotations: Vec<(f32, [f32; 4])>,
}

/// An image, drawn with hard pixel edges.
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    /// sRGB with alpha, four bytes per pixel, rows from the top.
    pub rgba: Vec<u8>,
}

impl ModelDef {
    /// Checks the mesh is well formed.
    pub fn validate(&self) -> Result<(), String> {
        if self.uvs.len() != self.positions.len() {
            return Err(format!("{} positions but {} texture coordinates", self.positions.len(), self.uvs.len()));
        }
        if self.indices.is_empty() || !self.indices.len().is_multiple_of(3) {
            return Err(format!("indices must be whole triangles, got {}", self.indices.len()));
        }
        if let Some(i) = self.indices.iter().find(|&&i| i as usize >= self.positions.len()) {
            return Err(format!("index {i} is past the last vertex"));
        }
        if self.positions.iter().flatten().chain(self.uvs.iter().flatten()).any(|v| !v.is_finite()) {
            return Err("vertices must be finite".into());
        }
        if let Some(skin) = &self.skin {
            skin.validate(self.positions.len())?;
        }
        let t = &self.texture;
        if t.width == 0 || t.height == 0 || t.rgba.len() != (t.width * t.height * 4) as usize {
            return Err(format!("texture is {}×{} but holds {} bytes", t.width, t.height, t.rgba.len()));
        }
        Ok(())
    }
}

impl Skin {
    /// Checks the skin fits a mesh of `vertices` vertices and its clips are well formed.
    pub fn validate(&self, vertices: usize) -> Result<(), String> {
        if self.joints.len() != vertices || self.weights.len() != vertices {
            return Err(format!(
                "{vertices} vertices but {} with bones and {} with weights",
                self.joints.len(),
                self.weights.len()
            ));
        }
        if self.bones.is_empty() || self.bones.len() > u16::MAX as usize {
            return Err(format!("a skin needs 1 to {} bones, got {}", u16::MAX, self.bones.len()));
        }
        let bones = self.bones.len();
        for (v, (joints, weights)) in self.joints.iter().zip(&self.weights).enumerate() {
            if let Some(j) = joints.iter().zip(weights).find(|(j, w)| **w > 0.0 && **j as usize >= bones) {
                return Err(format!("vertex {v} follows bone {}, past the last", j.0));
            }
            if weights.iter().any(|w| !w.is_finite() || *w < 0.0) || (weights.iter().sum::<f32>() - 1.0).abs() > 0.01 {
                return Err(format!("vertex {v}'s weights must be positive and add up to 1, got {weights:?}"));
            }
        }
        for (i, bone) in self.bones.iter().enumerate() {
            if bone.parent.is_some_and(|p| p as usize >= i) {
                return Err(format!("bone {:?} must come after its parent", bone.name));
            }
            if !pose_is_finite(&bone.rest) || bone.inverse_bind.iter().any(|v| !v.is_finite()) {
                return Err(format!("bone {:?} must be finite", bone.name));
            }
        }
        for clip in &self.clips {
            let at = |why: String| format!("clip {:?}: {why}", clip.name);
            if clip.name.is_empty() {
                return Err("a clip needs a name".into());
            }
            if self.clips.iter().filter(|c| c.name == clip.name).count() > 1 {
                return Err(at("named twice".into()));
            }
            if !(clip.length.is_finite() && clip.length > 0.0) {
                return Err(at(format!("length must be above 0, got {}", clip.length)));
            }
            for track in &clip.tracks {
                if track.bone as usize >= bones {
                    return Err(at(format!("moves bone {}, past the last", track.bone)));
                }
                if !in_time_order(track.translations.iter().map(|k| k.0)) || !in_time_order(track.rotations.iter().map(|k| k.0)) {
                    return Err(at(format!("bone {}'s keys must be in time order", track.bone)));
                }
                let values = track.translations.iter().flat_map(|k| k.1).chain(track.rotations.iter().flat_map(|k| k.1));
                if values.into_iter().any(|v| !v.is_finite()) {
                    return Err(at(format!("bone {}'s keys must be finite", track.bone)));
                }
            }
        }
        Ok(())
    }

    /// The clip named `name`.
    pub fn clip(&self, name: &str) -> Option<usize> {
        self.clips.iter().position(|c| c.name == name)
    }

    /// Every bone's pose `time` seconds into clip `clip`, relative to its parent. A looping clip
    /// wraps around; another holds its last pose. Bones the clip doesn't move stay at rest.
    pub fn pose(&self, clip: usize, time: f32) -> Vec<BonePose> {
        let mut poses: Vec<BonePose> = self.bones.iter().map(|b| b.rest).collect();
        let Some(clip) = self.clips.get(clip) else {
            return poses;
        };
        let time = clip.time_at(time);
        for track in &clip.tracks {
            let Some(pose) = poses.get_mut(track.bone as usize) else {
                continue;
            };
            if let Some(t) = sample(&track.translations, time, |a, b, f| Vec3::from(a).lerp(Vec3::from(b), f).to_array()) {
                pose.translation = t;
            }
            if let Some(r) = sample(&track.rotations, time, |a, b, f| {
                Quat::from_array(a).normalize().slerp(Quat::from_array(b).normalize(), f).to_array()
            }) {
                pose.rotation = r;
            }
        }
        poses
    }
}

impl BonePose {
    /// The pose `f` of the way from `self` to `to`: 0 is `self`, 1 is `to`.
    pub fn blend(&self, to: &BonePose, f: f32) -> BonePose {
        let lerp = |a: [f32; 3], b: [f32; 3]| Vec3::from(a).lerp(Vec3::from(b), f).to_array();
        BonePose {
            translation: lerp(self.translation, to.translation),
            rotation: Quat::from_array(self.rotation).normalize().slerp(Quat::from_array(to.rotation).normalize(), f).to_array(),
            scale: lerp(self.scale, to.scale),
        }
    }
}

impl Clip {
    /// Where in the clip `time` seconds after it started lands.
    pub fn time_at(&self, time: f32) -> f32 {
        let time = time.max(0.0);
        if self.looping { time.rem_euclid(self.length) } else { time.min(self.length) }
    }

    /// Whether a clip that doesn't loop has played to its end by `time`.
    pub fn finished(&self, time: f32) -> bool {
        !self.looping && time >= self.length
    }
}

fn in_time_order(mut times: impl Iterator<Item = f32>) -> bool {
    let mut last = f32::NEG_INFINITY;
    times.all(|t| {
        let ok = t.is_finite() && t >= last;
        last = t;
        ok
    })
}

fn pose_is_finite(p: &BonePose) -> bool {
    p.translation.iter().chain(&p.rotation).chain(&p.scale).all(|v| v.is_finite())
}

/// The value at `time` between the keys either side of it.
fn sample<T: Copy>(keys: &[(f32, T)], time: f32, blend: impl Fn(T, T, f32) -> T) -> Option<T> {
    let first = keys.first()?;
    if time <= first.0 {
        return Some(first.1);
    }
    let next = keys.partition_point(|k| k.0 <= time);
    let Some(&(t1, b)) = keys.get(next) else {
        return keys.last().map(|k| k.1);
    };
    let (t0, a) = keys[next - 1];
    let span = t1 - t0;
    Some(if span > 0.0 { blend(a, b, (time - t0) / span) } else { b })
}

/// A sound: one or more variations of it, each the bytes of an Ogg Vorbis or WAV file. Every time it
/// plays, one variation is picked at random, so a sound heard often doesn't repeat exactly.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundDef {
    /// Namespaced, such as `"base:moo"`.
    pub name: String,
    pub variations: Vec<Vec<u8>>,
    /// Loudness as a gain: 1 plays the file as recorded.
    pub volume: f32,
    /// Playback speed, which is also pitch: 1 as recorded, 0.5 an octave down and slower.
    pub pitch: f32,
    /// How much its pitch varies from one play to the next, as a share either way.
    pub pitch_spread: f32,
    /// Whether it repeats until stopped, such as a motor's hum.
    pub looping: bool,
}

impl SoundDef {
    /// One file played as recorded, once.
    pub fn new(name: impl Into<String>, ogg: Vec<u8>) -> Self {
        Self { name: name.into(), variations: vec![ogg], volume: 1.0, pitch: 1.0, pitch_spread: 0.0, looping: false }
    }

    /// Checks it has something to play and its numbers make sense.
    pub fn validate(&self) -> Result<(), String> {
        if self.variations.is_empty() || self.variations.iter().any(Vec::is_empty) {
            return Err(format!("sound {:?} has no audio", self.name));
        }
        if !(self.volume.is_finite() && self.volume >= 0.0) {
            return Err(format!("sound {:?}: volume must be at least 0, got {}", self.name, self.volume));
        }
        if !(self.pitch.is_finite() && self.pitch > 0.0) {
            return Err(format!("sound {:?}: pitch must be positive, got {}", self.name, self.pitch));
        }
        if !(0.0..1.0).contains(&self.pitch_spread) {
            return Err(format!("sound {:?}: pitch_spread must be from 0 to below 1, got {}", self.name, self.pitch_spread));
        }
        Ok(())
    }
}

/// The sounds a block makes, by registered sound name. Blocks crafted into other shapes make
/// their material's full block's sounds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockSounds {
    /// A footstep on it.
    pub step: Option<String>,
    /// Breaking it.
    pub broken: Option<String>,
    /// Placing it.
    pub placed: Option<String>,
}

/// The sounds a melee tool makes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolSounds {
    /// Swinging it, at the strike.
    pub swing: Option<String>,
    /// It landing on a target.
    pub hit: Option<String>,
}

/// The sounds a creature makes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TargetSounds {
    /// Its voice when hit or slammed, and when its blow lands on the player.
    pub hit: Option<String>,
    /// A footstep, every stride it walks or runs.
    pub step: Option<String>,
    /// Being slammed into a wall.
    pub slam: Option<String>,
    /// Breaking: a blow or a slam that takes the last of its health.
    pub death: Option<String>,
}

/// The sounds the player's body makes. Clients play them from what they see happen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerSounds {
    /// A footstep on a block that has no step sound of its own.
    pub step: Option<String>,
    /// Landing, louder for a harder landing.
    pub land: Option<String>,
    /// A landing hard enough to bottom out, played over `land`.
    pub hard_land: Option<String>,
    /// Being hurt by a blow or a fall.
    pub hurt: Option<String>,
    /// Being knocked out: a blow or a fall that takes the last of its health.
    pub knocked_out: Option<String>,
    /// A blow caught by a raised guard.
    pub guard: Option<String>,
    /// A loop whose loudness and pitch follow how fast the view turns: the neck's motor.
    pub servo: Option<String>,
    /// A loop whose loudness and pitch follow how fast the body moves: rushing air.
    pub wind: Option<String>,
    /// Picking up loose items.
    pub pickup: Option<String>,
    /// Crafting a part.
    pub craft: Option<String>,
    /// A craft that made nothing.
    pub craft_refused: Option<String>,
}

impl BlockSounds {
    /// Every sound named.
    pub fn names(&self) -> impl Iterator<Item = &String> {
        [&self.step, &self.broken, &self.placed].into_iter().flatten()
    }
}

impl ToolSounds {
    /// Every sound named.
    pub fn names(&self) -> impl Iterator<Item = &String> {
        [&self.swing, &self.hit].into_iter().flatten()
    }
}

impl TargetSounds {
    /// Every sound named.
    pub fn names(&self) -> impl Iterator<Item = &String> {
        [&self.hit, &self.step, &self.slam, &self.death].into_iter().flatten()
    }
}

impl PlayerSounds {
    /// Every sound named.
    pub fn names(&self) -> impl Iterator<Item = &String> {
        [
            &self.step,
            &self.land,
            &self.hard_land,
            &self.hurt,
            &self.knocked_out,
            &self.guard,
            &self.servo,
            &self.wind,
            &self.pickup,
            &self.craft,
            &self.craft_refused,
        ]
        .into_iter()
        .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sound_checks_its_values() {
        let sound = SoundDef::new("test:thud", b"OggS".to_vec());
        assert_eq!(sound.validate(), Ok(()));
        assert!(SoundDef { variations: Vec::new(), ..sound.clone() }.validate().is_err());
        assert!(SoundDef { variations: vec![Vec::new()], ..sound.clone() }.validate().is_err());
        assert!(SoundDef { volume: -1.0, ..sound.clone() }.validate().is_err());
        assert!(SoundDef { pitch: 0.0, ..sound.clone() }.validate().is_err());
        assert!(SoundDef { pitch_spread: 1.0, ..sound }.validate().is_err());
    }

    fn triangle() -> ModelDef {
        ModelDef {
            name: "test:tri".into(),
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            uvs: vec![[0.0; 2]; 3],
            indices: vec![0, 1, 2],
            texture: Texture { width: 1, height: 1, rgba: vec![255; 4] },
            skin: None,
        }
    }

    const REST: BonePose = BonePose { translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] };
    const IDENTITY: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

    /// Two bones, the second a child, and a one-second clip sliding the child up 1 m and turning
    /// it a quarter turn about Z.
    fn skin(looping: bool) -> Skin {
        let quarter = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array();
        Skin {
            joints: vec![[0, 0, 0, 0], [1, 0, 0, 0], [1, 0, 0, 0]],
            weights: vec![[1.0, 0.0, 0.0, 0.0]; 3],
            bones: vec![
                Bone { name: "root".into(), parent: None, rest: REST, inverse_bind: IDENTITY },
                Bone { name: "arm".into(), parent: Some(0), rest: REST, inverse_bind: IDENTITY },
            ],
            clips: vec![Clip {
                name: "lift".into(),
                length: 1.0,
                looping,
                tracks: vec![Track {
                    bone: 1,
                    translations: vec![(0.0, [0.0; 3]), (1.0, [0.0, 0.0, 1.0])],
                    rotations: vec![(0.0, REST.rotation), (1.0, quarter)],
                }],
            }],
        }
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4)
    }

    #[test]
    fn checks_the_mesh() {
        assert_eq!(triangle().validate(), Ok(()));
        assert!(ModelDef { indices: vec![0, 1], ..triangle() }.validate().is_err());
        assert!(ModelDef { indices: vec![0, 1, 3], ..triangle() }.validate().is_err());
        assert!(ModelDef { uvs: vec![[0.0; 2]; 2], ..triangle() }.validate().is_err());
        let texture = Texture { width: 2, height: 1, rgba: vec![255; 4] };
        assert!(ModelDef { texture, ..triangle() }.validate().is_err());
    }

    #[test]
    fn checks_the_skin() {
        let ok = ModelDef { skin: Some(skin(true)), ..triangle() };
        assert_eq!(ok.validate(), Ok(()));
        let broken = |change: fn(&mut Skin)| {
            let mut s = skin(true);
            change(&mut s);
            ModelDef { skin: Some(s), ..triangle() }.validate().is_err()
        };
        assert!(broken(|s| s.joints.truncate(2)), "a vertex without bones");
        assert!(broken(|s| s.joints[0] = [2, 0, 0, 0]), "a bone past the last");
        assert!(broken(|s| s.weights[0] = [0.5, 0.0, 0.0, 0.0]), "weights not adding up");
        assert!(broken(|s| s.bones[0].parent = Some(1)), "a parent after its child");
        assert!(broken(|s| s.clips[0].length = 0.0));
        assert!(broken(|s| s.clips[0].tracks[0].bone = 2));
        assert!(broken(|s| s.clips[0].tracks[0].translations.reverse()), "keys out of order");
        assert!(broken(|s| s.clips.push(s.clips[0].clone())), "a name used twice");
        assert!(broken(|s| s.bones[1].rest.scale[0] = f32::NAN));
    }

    #[test]
    fn a_clip_blends_between_keys_and_leaves_other_bones_at_rest() {
        let s = skin(true);
        let half = s.pose(0, 0.5);
        assert_eq!(half[0], REST, "bone 0 has no track");
        assert!(close(half[1].translation, [0.0, 0.0, 0.5]));
        let turned = Quat::from_array(half[1].rotation) * Vec3::X;
        let eighth = std::f32::consts::FRAC_1_SQRT_2;
        assert!(close(turned.to_array(), [eighth, eighth, 0.0]), "halfway round the quarter turn, got {turned}");
        assert_eq!(s.pose(5, 0.5), vec![REST; 2], "no such clip");
    }

    #[test]
    fn a_looping_clip_wraps_and_another_holds_its_last_pose() {
        let looping = skin(true);
        assert!(close(looping.pose(0, 1.25)[1].translation, [0.0, 0.0, 0.25]));
        assert!(!looping.clips[0].finished(9.0));
        let once = skin(false);
        assert!(close(once.pose(0, 1.25)[1].translation, [0.0, 0.0, 1.0]));
        assert!(close(once.pose(0, -1.0)[1].translation, [0.0; 3]), "before the start holds the first key");
        assert!(once.clips[0].finished(1.0) && !once.clips[0].finished(0.9));
        assert_eq!(looping.clip("lift"), Some(0));
        assert_eq!(looping.clip("walk"), None);
    }

    #[test]
    fn poses_blend_part_way() {
        let a = REST;
        let b = BonePose { translation: [2.0, 0.0, 0.0], rotation: Quat::from_rotation_z(1.0).to_array(), scale: [3.0; 3] };
        let mid = a.blend(&b, 0.5);
        assert!(close(mid.translation, [1.0, 0.0, 0.0]) && close(mid.scale, [2.0; 3]));
        assert!(Quat::from_array(mid.rotation).abs_diff_eq(Quat::from_rotation_z(0.5), 1e-5));
        assert_eq!(a.blend(&b, 0.0), a);
    }
}
