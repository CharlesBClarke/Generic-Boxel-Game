//! Hitting things on the client side: targets drawn with their models, the held tool in front of
//! the camera, and everything that makes a hit land. The simulation decides what a strike hits;
//! this only shows and plays it.
//!
//! A hit freezes the view for a moment (hit-stop), kicks the view, flashes the target white,
//! squashes and tumbles it and recoils the tool. A head hit (any hit box that multiplies damage)
//! freezes longer and flashes gold. Sounds are `sound`'s. A target slammed
//! into a wall freezes, flashes and kicks up dust from the wall; a hard landing squashes it and
//! kicks up dust from the ground. A target that breaks bursts into chunks. A swing that misses
//! whiffs: the tool follows through further. The last hit's damage, and why, shows under the
//! crosshair.

use std::collections::HashMap;

use api::{ModelDef, Skin, Tunable, Tunables, Vec3 as ZVec3, WorldPos};
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sim::{SwingStage, Target};

use crate::animation::{self, AnimationPreview, AnimationTuning, Animator};
use crate::drill::Drill;
use crate::editing::{Held, Palette};
use crate::world::ClientWorld;
use crate::{RenderOrigin, coords};

/// How much brighter than its texture a target is drawn at the start of a flash. The window
/// clamps colors at white, so this washes everything but the darkest texels out to white.
const FLASH_BRIGHTNESS: f32 = 12.0;
/// The direction the baked shading on models is lit from, Z up.
const LIGHT: ZVec3 = ZVec3::new(0.35, -0.5, 0.8);
/// The darkest and brightest a model's shading goes.
const SHADE_MIN: f32 = 0.55;
const SHADE_MAX: f32 = 1.0;
/// How long chunks of a broken target last, and how long they spend shrinking away at the end.
const CHUNK_LIFE: f32 = 2.5;
const CHUNK_FADE: f32 = 0.5;
const CHUNK_GRAVITY: f32 = 25.0;
/// Share of their speed chunks keep bouncing off the ground.
const CHUNK_BOUNCE: f32 = 0.35;
/// The gold a head hit flashes, as a share of white in each channel.
const CRIT_TINT: Vec3 = Vec3::new(1.0, 0.75, 0.2);
/// How long the hit readout under the crosshair stays, and fades over at the end, seconds.
const READOUT_TIME: f32 = 1.2;
const READOUT_FADE: f32 = 0.4;

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct CombatFeel {
    /// Seconds the game freezes on a hit, plus `hitstop_per_damage` for each point of damage,
    /// up to `hitstop_max`.
    pub hitstop: f32,
    pub hitstop_per_damage: f32,
    pub hitstop_max: f32,
    /// Extra freeze when the hit breaks the target, seconds.
    pub break_hitstop: f32,
    /// Upward kick of the view on a hit, degrees.
    pub kick: f32,
    /// Downward dip of the view on a miss, degrees: the swing carries the robot forward.
    pub whiff_dip: f32,
    /// How quickly the view's kick recovers, per second.
    pub kick_recover: f32,
    /// How far the view shakes on a hit, meters, and for how long, seconds.
    pub shake: f32,
    pub shake_time: f32,
    /// How long a hit target flashes white, seconds.
    pub flash_time: f32,
    /// How much a hit squashes the target along the hit and stretches it across, and for how
    /// long, seconds.
    pub squash: f32,
    pub squash_time: f32,
    /// Tumble speed, radians a second for each meter a second a hit pushes the target.
    pub tumble: f32,
    /// How quickly a target turns back upright on the ground, per second.
    pub settle: f32,
    /// How far the tool jolts back toward the camera on a hit, meters.
    pub recoil: f32,
    /// Spring stiffness of the tool's recoil, per second squared. Damped to settle in one bounce.
    pub recoil_stiffness: f32,
    /// How far past its strike pose a missed swing carries the tool, as a share of the swing.
    pub whiff_follow: f32,
    /// How many chunks a broken target bursts into, and how fast they fly, meters a second.
    pub chunks: f32,
    pub chunk_speed: f32,
    /// Extra freeze on a head hit, seconds, and how much higher its sound plays.
    pub crit_hitstop: f32,
    pub crit_pitch: f32,
    /// Freeze on a wall slam, seconds, and how far the view shakes, meters.
    pub slam_hitstop: f32,
    pub slam_shake: f32,
    /// Landing speed, meters a second, from which a target kicks up dust, and how many bits.
    pub dust_speed: f32,
    pub dust: f32,
    /// How much a landing squashes a target for each meter a second it lands at.
    pub landing_squash: f32,
    /// When a creature's blow hits the player: a freeze, seconds; a jolt of the view, degrees;
    /// a shake, meters; and a red flash over the screen, seconds and how strong at first.
    pub hurt_hitstop: f32,
    pub hurt_kick: f32,
    pub hurt_shake: f32,
    pub hurt_flash: f32,
    pub hurt_flash_alpha: f32,
    /// A crystal blast: how far the view shakes right beside it, meters, fading to nothing
    /// `blast_reach` meters away; how long its beam shows, seconds; and how wide, meters.
    pub blast_shake: f32,
    pub blast_reach: f32,
    pub blast_time: f32,
    pub blast_width: f32,
}

impl Default for CombatFeel {
    fn default() -> Self {
        Self {
            hitstop: 0.06,
            hitstop_per_damage: 0.006,
            hitstop_max: 0.15,
            break_hitstop: 0.08,
            kick: 1.6,
            whiff_dip: 0.8,
            kick_recover: 14.0,
            shake: 0.04,
            shake_time: 0.18,
            flash_time: 0.12,
            squash: 0.3,
            squash_time: 0.25,
            tumble: 1.2,
            settle: 8.0,
            recoil: 0.12,
            recoil_stiffness: 400.0,
            whiff_follow: 0.35,
            chunks: 24.0,
            chunk_speed: 6.0,
            crit_hitstop: 0.05,
            crit_pitch: 1.35,
            slam_hitstop: 0.07,
            slam_shake: 0.06,
            blast_shake: 0.18,
            blast_reach: 32.0,
            blast_time: 0.4,
            blast_width: 0.45,
            dust_speed: 5.0,
            dust: 8.0,
            landing_squash: 0.03,
            hurt_hitstop: 0.05,
            hurt_kick: 3.0,
            hurt_shake: 0.08,
            hurt_flash: 0.4,
            hurt_flash_alpha: 0.35,
        }
    }
}

macro_rules! tunable_fields {
    ($($field:ident: $min:expr, $max:expr;)*) => {
        impl Tunables for CombatFeel {
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
    hitstop: 0.0, 0.3;
    hitstop_per_damage: 0.0, 0.05;
    hitstop_max: 0.0, 0.5;
    break_hitstop: 0.0, 0.5;
    kick: 0.0, 10.0;
    whiff_dip: 0.0, 5.0;
    kick_recover: 1.0, 40.0;
    shake: 0.0, 0.3;
    shake_time: 0.0, 1.0;
    flash_time: 0.0, 0.5;
    squash: 0.0, 0.8;
    squash_time: 0.01, 1.0;
    tumble: 0.0, 6.0;
    settle: 0.5, 30.0;
    recoil: 0.0, 0.5;
    recoil_stiffness: 20.0, 2000.0;
    whiff_follow: 0.0, 1.0;
    chunks: 0.0, 100.0;
    chunk_speed: 0.0, 20.0;
    crit_hitstop: 0.0, 0.3;
    crit_pitch: 0.5, 2.0;
    slam_hitstop: 0.0, 0.3;
    slam_shake: 0.0, 0.3;
    blast_shake: 0.0, 0.5;
    blast_reach: 1.0, 100.0;
    blast_time: 0.05, 2.0;
    blast_width: 0.05, 1.5;
    dust_speed: 0.0, 20.0;
    dust: 0.0, 40.0;
    landing_squash: 0.0, 0.1;
    hurt_hitstop: 0.0, 0.3;
    hurt_kick: 0.0, 15.0;
    hurt_shake: 0.0, 0.3;
    hurt_flash: 0.0, 2.0;
    hurt_flash_alpha: 0.0, 1.0;
}

/// Freezes the view for a moment after a hit. The simulation carries on behind it (it may be
/// shared with other players); the view holds the last tick shown and snaps to the latest when
/// the freeze ends. New presses wait for the end.
#[derive(Resource, Debug, Default)]
pub struct HitStop {
    pub remaining: f32,
}

impl HitStop {
    /// Freezes for at least `seconds` from now.
    pub fn freeze(&mut self, seconds: f32) {
        self.remaining = self.remaining.max(seconds);
    }

    pub fn frozen(&self) -> bool {
        self.remaining > 0.0
    }

    /// Counts down `dt` seconds of real time.
    pub fn update(&mut self, dt: f32) {
        self.remaining = (self.remaining - dt).max(0.0);
    }
}

/// The view's reaction to hits: a pitch kick that recovers, and a shake that fades.
#[derive(Resource, Debug)]
pub struct ViewKick {
    /// Degrees up.
    pitch: f32,
    shake_left: f32,
    shake_length: f32,
    shake_amount: f32,
    phase: f32,
}

impl Default for ViewKick {
    fn default() -> Self {
        Self { pitch: 0.0, shake_left: 0.0, shake_length: 1.0, shake_amount: 0.0, phase: 0.0 }
    }
}

impl ViewKick {
    pub(crate) fn kick(&mut self, degrees: f32) {
        self.pitch += degrees;
    }

    pub(crate) fn shake(&mut self, amount: f32, seconds: f32) {
        self.shake_amount = amount;
        self.shake_length = seconds.max(1e-3);
        self.shake_left = seconds;
    }

    pub fn update(&mut self, feel: &CombatFeel, dt: f32) {
        self.pitch *= (-feel.kick_recover * dt).exp();
        self.shake_left = (self.shake_left - dt).max(0.0);
        self.phase += dt;
    }

    /// Upward pitch offset this frame, radians.
    pub fn pitch(&self) -> f32 {
        self.pitch.to_radians()
    }

    /// Render-space camera offset this frame: a quick wobble that fades out.
    pub fn shake_offset(&self) -> Vec3 {
        let fade = self.shake_left / self.shake_length;
        let t = self.phase * 60.0;
        Vec3::new((t * 1.7).sin(), (t * 2.3 + 1.0).sin(), 0.0) * self.shake_amount * fade * fade
    }
}

/// How each target is drawn, by the target's ID, and the media its models need.
#[derive(Resource, Default)]
pub(crate) struct TargetViews {
    views: HashMap<u64, TargetView>,
    models: HashMap<String, ModelAssets>,
}

struct ModelAssets {
    mesh: Handle<Mesh>,
    texture: Handle<Image>,
    /// Opaque texel colors, for chunks.
    colors: Vec<Color>,
    /// The skeleton and clips of a model that moves, and its bones' inverse bind poses.
    skin: Option<(Skin, Handle<SkinnedMeshInverseBindposes>)>,
}

/// A drawn model's bones, posed each frame by its animator.
struct Rig {
    bones: Vec<Entity>,
    animator: Animator,
}

#[derive(Component)]
pub(crate) struct BoneMarker;

/// One target's entities and motion. The outer entity sits at the middle of the target's box and
/// holds the squash, along the last hit; the inner one holds the tumble, about that middle; the
/// model hangs below with its feet at the box's bottom.
struct TargetView {
    entity: Entity,
    inner: Entity,
    /// The entity that turns the model to its facing, and its turn on top of the facing.
    turn: Entity,
    turn_extra: Quat,
    material: Handle<StandardMaterial>,
    rig: Option<Rig>,
    /// Render-space turn on top of its facing, from tumbling.
    tilt: Quat,
    /// Render-space axis times radians a second.
    spin: Vec3,
    /// Seconds of flash left, and its color as a share of white in each channel.
    flash: f32,
    tint: Vec3,
    /// Seconds since the last squash, how hard, and along which render direction.
    squash: (f32, f32, Vec3),
    /// Where it was last drawn, for a burst if it breaks.
    center: WorldPos,
    /// Its velocity last frame, Z up, for chunks and landings.
    velocity: ZVec3,
    was_on_ground: bool,
}

/// What the last hit did, shown under the crosshair.
#[derive(Resource, Debug, Default)]
pub(crate) struct HitReadout {
    text: String,
    /// Seconds since it was set.
    age: f32,
}

impl HitReadout {
    pub(crate) fn show(&mut self, text: String) {
        self.text = text;
        self.age = 0.0;
    }
}

/// The readout's words for a hit: its damage, and what made it more.
fn hit_words(damage: f32, zone: Option<&str>, crit: bool, carried: f32, broke: bool) -> String {
    let mut words = Vec::new();
    if crit {
        words.push(zone.unwrap_or("crit").to_uppercase());
    }
    words.push(format!("{damage:.0}"));
    if carried >= 0.5 {
        words.push(format!("+{carried:.1} m/s"));
    }
    if broke {
        words.push("BROKEN".into());
    }
    words.join("  ")
}

#[derive(Component)]
pub(crate) struct ReadoutText;

#[derive(Component)]
pub(crate) struct TargetMarker;

#[derive(Component)]
pub(crate) struct ChunkMarker {
    pos: WorldPos,
    velocity: ZVec3,
    spin: Vec3,
    rotation: Quat,
    size: f32,
    age: f32,
}

/// Builds every model's mesh and texture once.
pub(crate) fn setup_media(
    world: Res<ClientWorld>,
    mut views: ResMut<TargetViews>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
) {
    for model in world.models() {
        let skin = model.skin.as_ref().map(|skin| {
            let poses: Vec<Mat4> = skin.bones.iter().map(|b| Mat4::from_cols_array(&b.inverse_bind)).collect();
            (skin.clone(), bindposes.add(SkinnedMeshInverseBindposes::from(poses)))
        });
        let assets = ModelAssets {
            mesh: meshes.add(model_mesh(model)),
            texture: images.add(model_image(model)),
            colors: model.texture.rgba.chunks(4).filter(|p| p[3] > 128).map(|p| Color::srgb_u8(p[0], p[1], p[2])).collect(),
            skin,
        };
        views.models.insert(model.name.clone(), assets);
    }
}

/// A model as a render mesh, with shading baked into vertex colors like the world's. A model
/// with a skin stays in its own Z-up space, since its bones carry it into render space; one
/// without is turned to Y up here.
fn model_mesh(model: &ModelDef) -> Mesh {
    let (positions, light): (Vec<[f32; 3]>, Vec3) = match model.skin {
        Some(_) => (model.positions.clone(), LIGHT.normalize()),
        None => (
            model.positions.iter().map(|&p| coords::to_render(Vec3::from_array(p)).to_array()).collect(),
            coords::to_render(LIGHT.normalize()),
        ),
    };
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, model.uvs.clone())
        .with_inserted_indices(Indices::U32(model.indices.clone()))
        .with_computed_smooth_normals();
    if let Some(skin) = &model.skin {
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(skin.joints.clone()));
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, skin.weights.clone());
    }
    let colors: Vec<[f32; 4]> = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL).and_then(|n| n.as_float3()) {
        Some(normals) => normals
            .iter()
            .map(|&n| {
                let shade = SHADE_MIN + (SHADE_MAX - SHADE_MIN) * Vec3::from_array(n).dot(light).max(0.0);
                [shade, shade, shade, 1.0]
            })
            .collect(),
        None => vec![[1.0; 4]; model.positions.len()],
    };
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh
}

fn model_image(model: &ModelDef) -> Image {
    let t = &model.texture;
    let mut image = Image::new(
        Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        t.rgba.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// The middle of a target's box.
fn center(target: &Target, height: f32) -> WorldPos {
    target.pos.translate(ZVec3::Z * height * 0.5)
}

/// Freezes the game while a hit-stop lasts, and lets the view's kick recover.
pub(crate) fn update_hit_stop(time: Res<Time>, feel: Res<CombatFeel>, mut stop: ResMut<HitStop>, mut kick: ResMut<ViewKick>) {
    stop.update(time.delta_secs());
    kick.update(&feel, time.delta_secs());
}

/// Plays the juice of every strike since the last frame: hits on targets and on the chase
/// drill's ghost, and misses.
#[allow(clippy::too_many_arguments)]
pub(crate) fn play_strikes(
    mut commands: Commands,
    mut world: ResMut<ClientWorld>,
    feel: Res<CombatFeel>,
    origin: Res<RenderOrigin>,
    mut views: ResMut<TargetViews>,
    mut stop: ResMut<HitStop>,
    mut kick: ResMut<ViewKick>,
    mut weapon: ResMut<WeaponState>,
    mut drill: ResMut<Drill>,
    mut readout: ResMut<HitReadout>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let strikes = world.take_strikes();
    for strike in strikes {
        let tool = world.tool(strike.tool).clone();
        let ghost_hit = drill.hit(strike.eye, strike.look, tool.reach, tool.radius);
        let Some(hit) = strike.hit else {
            if ghost_hit {
                land_hit(&feel, tool.damage, false, &mut stop, &mut kick, &mut weapon);
                readout.show("GHOST".into());
            } else {
                kick.kick(-feel.whiff_dip);
                weapon.whiffed = true;
            }
            continue;
        };
        land_hit(&feel, hit.damage, hit.broke, &mut stop, &mut kick, &mut weapon);
        let def = world.target_def(hit.kind).clone();
        let zone = hit.zone.map(|z| &def.hit_boxes[z]);
        let crit = zone.is_some_and(|z| z.damage > 1.0);
        if crit {
            let longer = stop.remaining + feel.crit_hitstop;
            stop.freeze(longer);
        }
        readout.show(hit_words(hit.damage, zone.map(|z| z.name.as_str()), crit, hit.carried, hit.broke));
        if hit.broke {
            if let Some(view) = views.views.remove(&hit.target) {
                commands.entity(view.entity).despawn();
                let colors = views.models.get(&def.model).map_or(&[][..], |m| &m.colors[..]);
                let push = view.velocity + hit.push;
                let burst = Burst { seed: strike.tick, center: view.center, push, count: feel.chunks, speed: feel.chunk_speed, size: 0.12 };
                burst.spawn(&mut commands, colors, &origin, &mut meshes, &mut materials);
            }
            continue;
        }
        if let Some(view) = views.views.get_mut(&hit.target) {
            react(view, &feel, hit.push, if crit { CRIT_TINT } else { Vec3::ONE });
        }
    }
}

/// The color of the first solid block at `pos`, or `None` if there's nothing solid there.
fn block_color(world: &ClientWorld, pos: WorldPos) -> Option<Color> {
    let blocks = world.blocks();
    let cell = world.universe().main().cell(pos.boxel());
    let def = cell.iter().filter_map(|o| blocks.get(o.block)).find(|d| d.solid)?;
    Some(Color::srgb_u8(def.color[0], def.color[1], def.color[2]))
}

/// Plays the juice of every wall slam since the last frame: a freeze, a flash and a squash
/// against the wall, a shake, and dust from the wall. A slam that breaks
/// a target bursts it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn play_slams(
    mut commands: Commands,
    mut world: ResMut<ClientWorld>,
    feel: Res<CombatFeel>,
    origin: Res<RenderOrigin>,
    mut views: ResMut<TargetViews>,
    mut stop: ResMut<HitStop>,
    mut kick: ResMut<ViewKick>,
    mut readout: ResMut<HitReadout>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let slams = world.take_slams();
    for slam in slams {
        let def = world.target_def(slam.kind).clone();
        stop.freeze(feel.slam_hitstop + if slam.broke { feel.break_hitstop } else { 0.0 });
        kick.shake(feel.slam_shake, feel.shake_time);
        readout.show(format!("SLAM  {:.0}{}", slam.damage, if slam.broke { "  BROKEN" } else { "" }));
        let middle = slam.pos.translate(ZVec3::Z * def.height * 0.5);
        let wall = middle.translate(slam.into * (def.width * 0.5 + 0.1));
        if let Some(color) = block_color(&world, wall) {
            let dust = Burst { seed: slam.target ^ 0x51a4, center: wall, push: -slam.into * 2.0, count: feel.dust, speed: 2.5, size: 0.06 };
            dust.spawn(&mut commands, &[color], &origin, &mut meshes, &mut materials);
        }
        if slam.broke {
            if let Some(view) = views.views.remove(&slam.target) {
                commands.entity(view.entity).despawn();
                let colors = views.models.get(&def.model).map_or(&[][..], |m| &m.colors[..]);
                let burst = Burst { seed: slam.target, center: view.center, push: -slam.into * 3.0, count: feel.chunks, speed: feel.chunk_speed, size: 0.12 };
                burst.spawn(&mut commands, colors, &origin, &mut meshes, &mut materials);
            }
            continue;
        }
        if let Some(view) = views.views.get_mut(&slam.target) {
            view.flash = feel.flash_time;
            view.tint = Vec3::ONE;
            view.squash = (0.0, feel.squash, coords::to_render(slam.into));
        }
    }
}

/// The parts of a hit that don't depend on what was hit.
fn land_hit(feel: &CombatFeel, damage: f32, broke: bool, stop: &mut HitStop, kick: &mut ViewKick, weapon: &mut WeaponState) {
    let freeze = (feel.hitstop + feel.hitstop_per_damage * damage).min(feel.hitstop_max);
    stop.freeze(freeze + if broke { feel.break_hitstop } else { 0.0 });
    kick.kick(feel.kick);
    kick.shake(feel.shake, feel.shake_time);
    weapon.recoil_velocity += feel.recoil * feel.recoil_stiffness.sqrt();
    weapon.whiffed = false;
}

/// A target's flash, in `tint`, squash and tumble from a hit pushing it by `push`.
fn react(view: &mut TargetView, feel: &CombatFeel, push: ZVec3, tint: Vec3) {
    view.flash = feel.flash_time;
    view.tint = tint;
    let along = coords::to_render(push.normalize_or(ZVec3::X));
    view.squash = (0.0, feel.squash, along);
    // Tumbles forward, away from the hit: about the level axis across the push.
    let across = ZVec3::Z.cross(ZVec3::new(push.x, push.y, 0.0)).normalize_or(ZVec3::X);
    view.spin += coords::to_render(across) * push.length() * feel.tumble;
}

/// A cheap hash of `seed` to a number from 0 to 1, the same every run.
fn hash(seed: u64) -> f32 {
    let mut x = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// A spray of chunks, flying out from `center` and carried along by `push`.
struct Burst {
    /// Picks the directions, colors and sizes, the same every time.
    seed: u64,
    center: WorldPos,
    push: ZVec3,
    count: f32,
    /// Meters a second.
    speed: f32,
    /// The smallest chunk's size; the biggest is twice that, meters.
    size: f32,
}

impl Burst {
    /// Spawns the chunks, each one of `colors`.
    fn spawn(
        &self,
        commands: &mut Commands,
        colors: &[Color],
        origin: &RenderOrigin,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
    ) {
        let cube = meshes.add(Cuboid::from_length(1.0));
        for i in 0..self.count.round() as u64 {
            let r = |k: u64| hash(self.seed.wrapping_mul(977) ^ (i * 8 + k));
            let dir = ZVec3::new(r(0) * 2.0 - 1.0, r(1) * 2.0 - 1.0, r(2) * 1.2 - 0.2).normalize_or(ZVec3::Z);
            let color = colors.get((r(3) * colors.len() as f32) as usize).copied().unwrap_or(Color::WHITE);
            let size = self.size * (1.0 + r(4));
            let pos = self.center.translate(dir * 0.3);
            let chunk = ChunkMarker {
                pos,
                velocity: dir * self.speed * (0.5 + r(5)) + self.push * 0.6,
                spin: Vec3::new(r(6) - 0.5, r(7) - 0.5, r(5) - 0.5) * 20.0,
                rotation: Quat::IDENTITY,
                size,
                age: 0.0,
            };
            let material = materials.add(StandardMaterial { base_color: color, unlit: true, ..default() });
            let transform = Transform::from_translation(coords::render_translation(pos, origin.cell)).with_scale(Vec3::splat(size));
            commands.spawn((Mesh3d(cube.clone()), MeshMaterial3d(material), transform, chunk));
        }
    }
}

/// Moves chunks under gravity, bouncing them off solid boxels, and removes them when they've
/// had their time.
pub(crate) fn update_chunks(
    mut commands: Commands,
    world: Res<ClientWorld>,
    time: Res<Time>,
    stop: Res<HitStop>,
    origin: Res<RenderOrigin>,
    mut chunks: Query<(Entity, &mut ChunkMarker, &mut Transform)>,
) {
    let dt = if stop.frozen() { 0.0 } else { time.delta_secs() };
    let grid = world.universe().main();
    let blocks = world.blocks();
    let solid = |pos: WorldPos| grid.cell(pos.boxel()).iter().any(|o| blocks.is_solid(o.block));
    for (entity, mut chunk, mut transform) in &mut chunks {
        chunk.age += dt;
        if chunk.age >= CHUNK_LIFE {
            commands.entity(entity).despawn();
            continue;
        }
        chunk.velocity.z -= CHUNK_GRAVITY * dt;
        let half = chunk.size * 0.5;
        for axis in [2, 0, 1] {
            let mut step = ZVec3::ZERO;
            step[axis] = chunk.velocity[axis] * dt;
            let mut probe = step;
            probe[axis] += half * step[axis].signum();
            if step[axis] != 0.0 && solid(chunk.pos.translate(probe)) {
                chunk.velocity[axis] *= -CHUNK_BOUNCE;
                if axis == 2 {
                    chunk.velocity.x *= 0.6;
                    chunk.velocity.y *= 0.6;
                    chunk.spin *= 0.5;
                }
            } else {
                chunk.pos = chunk.pos.translate(step);
            }
        }
        let spin = chunk.spin;
        chunk.rotation = (Quat::from_scaled_axis(spin * dt) * chunk.rotation).normalize();
        let fade = ((CHUNK_LIFE - chunk.age) / CHUNK_FADE).min(1.0);
        *transform = Transform::from_translation(coords::render_translation(chunk.pos, origin.cell))
            .with_rotation(chunk.rotation)
            .with_scale(Vec3::splat(chunk.size * fade));
    }
}

/// A target's facing in render space: the model's front is +Y, the target's yaw counts from +X.
fn facing(yaw: f32) -> Quat {
    Quat::from_axis_angle(coords::to_render(ZVec3::Z), yaw - std::f32::consts::FRAC_PI_2)
}

/// Spawns, moves and despawns target entities to match the simulation's targets.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sync_targets(
    mut commands: Commands,
    world: Res<ClientWorld>,
    mut meshes: ResMut<Assets<Mesh>>,
    feel: Res<CombatFeel>,
    origin: Res<RenderOrigin>,
    fixed: Res<Time<Fixed>>,
    time: Res<Time>,
    stop: Res<HitStop>,
    tuning: Res<AnimationTuning>,
    preview: Res<AnimationPreview>,
    mut state: ResMut<TargetViews>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut transforms: Query<&mut Transform, (With<TargetMarker>, Without<BoneMarker>)>,
    mut bones: Query<&mut Transform, (With<BoneMarker>, Without<TargetMarker>)>,
) {
    // While frozen, everything holds still but the flash.
    let real_dt = time.delta_secs();
    let dt = if stop.frozen() { 0.0 } else { real_dt };
    let alpha = if stop.frozen() { 1.0 } else { fixed.overstep_fraction() };
    let TargetViews { views, models, .. } = &mut *state;
    let mut live = Vec::with_capacity(world.targets().len());
    for target in world.targets().iter() {
        live.push(target.id);
        let def = world.target_def(target.kind);
        let Some(model) = models.get(&def.model) else {
            continue;
        };
        let now = center(target, def.height);
        let drawn = target.prev_pos.translate(target.prev_pos.delta_to(target.pos) * alpha).translate(ZVec3::Z * def.height * 0.5);
        let view = views.entry(target.id).or_insert_with(|| {
            let material = materials.add(StandardMaterial {
                base_color_texture: Some(model.texture.clone()),
                unlit: true,
                alpha_mode: AlphaMode::Mask(0.5),
                ..default()
            });
            let facing = facing(target.yaw);
            let feet = Transform::from_translation(coords::to_render(ZVec3::Z * -def.height * 0.5)).with_rotation(facing);
            let inner = commands.spawn((Transform::default(), Visibility::default(), TargetMarker)).id();
            let (turn, turn_extra, rig) = match &model.skin {
                None => {
                    let turn = commands
                        .spawn((Mesh3d(model.mesh.clone()), MeshMaterial3d(material.clone()), feet, TargetMarker, ChildOf(inner)))
                        .id();
                    (turn, Quat::IDENTITY, None)
                }
                Some((skin, bindposes)) => {
                    // The rig turns the model's Z-up space into render space; bones and their
                    // keys stay Z-up beneath it.
                    let to_render = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
                    let space = commands
                        .spawn((Transform { rotation: facing * to_render, ..feet }, Visibility::default(), TargetMarker, ChildOf(inner)))
                        .id();
                    let mut entities: Vec<Entity> = Vec::with_capacity(skin.bones.len());
                    for bone in &skin.bones {
                        let parent = bone.parent.map_or(space, |p| entities[p as usize]);
                        entities.push(commands.spawn((animation::transform(&bone.rest), BoneMarker, ChildOf(parent))).id());
                    }
                    commands.spawn((
                        Mesh3d(model.mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        SkinnedMesh { inverse_bindposes: bindposes.clone(), joints: entities.clone() },
                        // Its bones place it, so its own bounds don't say where it is.
                        NoFrustumCulling,
                        Transform::default(),
                        ChildOf(space),
                    ));
                    (space, to_render, Some(Rig { bones: entities, animator: Animator::new(skin, target.id) }))
                }
            };
            let outer = Transform::from_translation(coords::render_translation(drawn, origin.cell));
            let entity = commands.spawn((outer, Visibility::default(), TargetMarker)).add_child(inner).id();
            TargetView {
                entity,
                inner,
                turn,
                turn_extra,
                material,
                rig,
                tilt: Quat::IDENTITY,
                spin: Vec3::ZERO,
                flash: 0.0,
                tint: Vec3::ONE,
                squash: (f32::INFINITY, 0.0, Vec3::Y),
                center: now,
                velocity: ZVec3::ZERO,
                was_on_ground: true,
            }
        });
        // A landing squashes it down, and a hard one kicks up dust from the ground.
        if target.on_ground && !view.was_on_ground {
            let impact = -view.velocity.z;
            if impact > 0.5 {
                view.squash = (0.0, (impact * feel.landing_squash).min(0.6), coords::to_render(ZVec3::Z));
            }
            if impact >= feel.dust_speed
                && let Some(color) = block_color(&world, target.pos.translate(ZVec3::Z * -0.05))
            {
                let dust = Burst { seed: target.id ^ world.tick(), center: target.pos, push: ZVec3::ZERO, count: feel.dust, speed: 2.0, size: 0.06 };
                dust.spawn(&mut commands, &[color], &origin, &mut meshes, &mut materials);
            }
        }
        view.was_on_ground = target.on_ground;
        view.center = now;
        view.velocity = target.velocity;

        view.flash = (view.flash - real_dt).max(0.0);
        if let Some(material) = materials.get_mut(&view.material) {
            let bright = Vec3::ONE + view.tint * (FLASH_BRIGHTNESS - 1.0) * (view.flash / feel.flash_time.max(1e-3));
            let wanted = Color::linear_rgb(bright.x, bright.y, bright.z);
            if material.base_color != wanted {
                material.base_color = wanted;
            }
        }
        if let (Some(rig), Some((skin, _))) = (&mut view.rig, &model.skin) {
            rig.animator.update(skin, &tuning, preview.clip.as_deref(), (&target.clip, target.clip_speed), dt);
            for (entity, pose) in rig.bones.iter().zip(rig.animator.pose(skin, &tuning)) {
                if let Ok(mut transform) = bones.get_mut(*entity) {
                    *transform = animation::transform(&pose);
                }
            }
        }
        view.squash.0 += dt;
        if target.on_ground {
            view.spin = Vec3::ZERO;
            view.tilt = view.tilt.slerp(Quat::IDENTITY, 1.0 - (-feel.settle * dt).exp());
        } else {
            view.tilt = (Quat::from_scaled_axis(view.spin * dt) * view.tilt).normalize();
        }

        let (since, amount, along) = view.squash;
        let squash = if since < feel.squash_time { amount * (1.0 - since / feel.squash_time).powi(2) } else { 0.0 };
        // Squashed along the hit and stretched across it, keeping its volume about the same: the
        // outer entity turns the hit's direction onto Y and scales there, the inner turns back.
        let to_hit = Quat::from_rotation_arc(Vec3::Y, along);
        if let Ok(mut transform) = transforms.get_mut(view.entity) {
            *transform = Transform::from_translation(coords::render_translation(drawn, origin.cell))
                .with_rotation(to_hit)
                .with_scale(Vec3::new(1.0 + squash * 0.5, 1.0 - squash, 1.0 + squash * 0.5));
        }
        if let Ok(mut transform) = transforms.get_mut(view.inner) {
            *transform = Transform::from_rotation(to_hit.inverse() * view.tilt);
        }
        // Facing, blended between ticks the short way round.
        let (pi, tau) = (std::f32::consts::PI, std::f32::consts::TAU);
        let yaw = target.prev_yaw + ((target.yaw - target.prev_yaw + pi).rem_euclid(tau) - pi) * alpha;
        if let Ok(mut transform) = transforms.get_mut(view.turn) {
            transform.rotation = facing(yaw) * view.turn_extra;
        }
    }
    views.retain(|id, view| {
        let keep = live.contains(id);
        if !keep {
            commands.entity(view.entity).despawn();
        }
        keep
    });
}

/// The held tool's animation state beyond what the simulation's swing says.
#[derive(Resource, Debug, Default)]
pub(crate) struct WeaponState {
    /// How far the tool is jolted back from a hit, meters, and how fast that's changing.
    recoil: f32,
    recoil_velocity: f32,
    /// The last strike missed, so the recovery follows through further.
    whiffed: bool,
    /// The pose it's easing toward outside a swing, and its current pose there.
    pose: Option<Transform>,
}

#[derive(Component)]
pub(crate) struct WeaponView;

/// A club in front of the camera, hidden until a tool is held.
pub(crate) fn setup_weapon(
    mut commands: Commands,
    camera: Single<Entity, With<crate::PlayerCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let wood = materials.add(StandardMaterial { base_color: Color::srgb(0.45, 0.3, 0.18), unlit: true, ..default() });
    let metal = materials.add(StandardMaterial { base_color: Color::srgb(0.55, 0.57, 0.6), unlit: true, ..default() });
    let band = materials.add(StandardMaterial { base_color: Color::srgb(0.25, 0.25, 0.28), unlit: true, ..default() });
    // Built along +Y from the grip, in the camera's space.
    let weapon = commands
        .spawn((Transform::default(), Visibility::Hidden, WeaponView))
        .with_children(|club| {
            club.spawn((Mesh3d(meshes.add(Cuboid::new(0.045, 0.5, 0.045))), MeshMaterial3d(wood), Transform::from_xyz(0.0, 0.2, 0.0)));
            club.spawn((Mesh3d(meshes.add(Cuboid::new(0.13, 0.2, 0.13))), MeshMaterial3d(metal), Transform::from_xyz(0.0, 0.5, 0.0)));
            club.spawn((Mesh3d(meshes.add(Cuboid::new(0.06, 0.08, 0.06))), MeshMaterial3d(band), Transform::from_xyz(0.0, -0.02, 0.0)));
        })
        .id();
    commands.entity(*camera).add_child(weapon);
}

/// Poses the tool in camera space, from the grip: resting low on the right, cocked back in the
/// wind-up, swung down across the middle at the strike, and held across the view to guard.
mod pose {
    use super::*;

    pub fn rest() -> Transform {
        Transform::from_xyz(0.3, -0.34, -0.5).with_rotation(Quat::from_euler(EulerRot::XYZ, -0.3, 0.0, -0.3))
    }

    pub fn cocked() -> Transform {
        Transform::from_xyz(0.36, -0.22, -0.48).with_rotation(Quat::from_euler(EulerRot::XYZ, 0.55, 0.15, -0.55))
    }

    pub fn struck() -> Transform {
        Transform::from_xyz(0.08, -0.42, -0.6).with_rotation(Quat::from_euler(EulerRot::XYZ, -1.55, 0.1, 0.55))
    }

    /// Past the strike, for a miss.
    pub fn followed_through() -> Transform {
        Transform::from_xyz(-0.08, -0.5, -0.55).with_rotation(Quat::from_euler(EulerRot::XYZ, -2.1, 0.2, 0.9))
    }

    pub fn guard() -> Transform {
        Transform::from_xyz(0.05, -0.2, -0.5).with_rotation(Quat::from_euler(EulerRot::XYZ, -0.15, 0.0, 1.45))
    }
}

fn blend(a: Transform, b: Transform, t: f32) -> Transform {
    Transform {
        translation: a.translation.lerp(b.translation, t),
        rotation: a.rotation.slerp(b.rotation, t),
        scale: Vec3::ONE,
    }
}

/// How quickly the tool eases between rest and guard, per second.
const POSE_RATE: f32 = 18.0;

/// Moves the held tool: the swing straight from the simulation's stage and progress, easing
/// between rest and guard otherwise, with the recoil spring on top.
#[allow(clippy::type_complexity)]
pub(crate) fn animate_weapon(
    world: Res<ClientWorld>,
    palette: Res<Palette>,
    feel: Res<CombatFeel>,
    time: Res<Time>,
    stop: Res<HitStop>,
    mut state: ResMut<WeaponState>,
    mut weapon: Single<(&mut Transform, &mut Visibility), With<WeaponView>>,
) {
    let (transform, visibility) = &mut *weapon;
    let held = matches!(palette.held(&world), Held::Tool(_));
    visibility.set_if_neq(if held { Visibility::Inherited } else { Visibility::Hidden });
    if !held {
        state.pose = None;
        return;
    }
    let dt = if stop.frozen() { 0.0 } else { time.delta_secs() };

    // A critically damped spring, so a hit jolts the tool back once and it returns.
    let k = feel.recoil_stiffness;
    let c = 2.0 * k.sqrt();
    let steps = (dt * 240.0).ceil().max(1.0);
    for _ in 0..steps as u32 {
        let h = dt / steps;
        state.recoil_velocity += (-k * state.recoil - c * state.recoil_velocity) * h;
        state.recoil += state.recoil_velocity * h;
    }

    let swing = world.swing().map(|s| (s.stage, s.progress(world.tool(s.tool))));
    let base = match swing {
        Some((SwingStage::Windup, p)) => {
            state.pose = None;
            blend(pose::rest(), pose::cocked(), ease_out(p))
        }
        Some((SwingStage::Recover, p)) => {
            state.pose = None;
            let end = if state.whiffed { blend(pose::struck(), pose::followed_through(), feel.whiff_follow) } else { pose::struck() };
            // Snap to the strike, linger a moment, then come back.
            let back = ((p - 0.25) / 0.75).clamp(0.0, 1.0);
            blend(end, pose::rest(), smooth(back))
        }
        None => {
            let wanted = if world.guarding() { pose::guard() } else { pose::rest() };
            let current = state.pose.unwrap_or(wanted);
            let eased = blend(current, wanted, 1.0 - (-POSE_RATE * dt).exp());
            state.pose = Some(eased);
            eased
        }
    };
    // Recoil pushes the grip back toward the camera and tips the head up.
    **transform = base * Transform::from_xyz(0.0, 0.0, 0.0).with_rotation(Quat::from_rotation_x(state.recoil * 3.0));
    transform.translation.z += state.recoil;
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(2)
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// The hit readout under the crosshair.
pub(crate) fn setup_readout(mut commands: Commands) {
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            top: percent(54),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_child((Text::new(""), TextFont { font_size: 22.0, ..default() }, TextColor(Color::WHITE), ReadoutText, crate::dev::DevText));
}

/// Shows the last hit's readout, fading it out after a moment.
pub(crate) fn update_readout(
    time: Res<Time>,
    mut readout: ResMut<HitReadout>,
    mut text: Single<(&mut Text, &mut TextColor), With<ReadoutText>>,
) {
    readout.age += time.delta_secs();
    let left = READOUT_TIME - readout.age;
    let shown = if left > 0.0 { readout.text.as_str() } else { "" };
    if text.0.0 != shown {
        text.0.0 = shown.to_owned();
    }
    let alpha = (left / READOUT_FADE).clamp(0.0, 1.0);
    let gold = readout.text.chars().next().is_some_and(|c| c.is_ascii_uppercase());
    let color = if gold { Color::srgba(1.0, 0.8, 0.25, alpha) } else { Color::srgba(1.0, 1.0, 1.0, alpha) };
    if text.1.0 != color {
        text.1.0 = color;
    }
}

/// While the tuning panel is open and dev text is shown, outlines every target's hit boxes, head-hit boxes in gold.
pub(crate) fn draw_hit_boxes(
    world: Res<ClientWorld>,
    panel: Res<crate::tuning::TuningPanel>,
    dev: Res<crate::dev::DevMode>,
    origin: Res<RenderOrigin>,
    mut gizmos: Gizmos,
) {
    if !panel.open || !dev.show_text {
        return;
    }
    for target in world.targets().iter() {
        let def = world.target_def(target.kind);
        let turn = facing(target.yaw);
        for b in &def.hit_boxes {
            let (min, max) = (ZVec3::from(b.min), ZVec3::from(b.max));
            let middle = coords::render_translation(target.pos, origin.cell) + turn * coords::to_render((min + max) * 0.5);
            let size = coords::to_render(max - min).abs();
            let color = if b.damage > 1.0 { Color::srgb(1.0, 0.8, 0.2) } else { Color::srgb(0.9, 0.2, 0.2) };
            gizmos.cube(Transform::from_translation(middle).with_rotation(turn).with_scale(size), color);
        }
    }
}

/// While the `show_routes` key has them on, draws every creature's path from where it stands:
/// green when the path gets to its goal, orange when it only gets as near as it can, with a
/// ring at each hop.
pub(crate) fn draw_routes(world: Res<ClientWorld>, dev: Res<crate::dev::DevMode>, origin: Res<RenderOrigin>, mut gizmos: Gizmos) {
    if !dev.show_routes {
        return;
    }
    // A little above the ground, so the line isn't hidden in it.
    let lift = ZVec3::Z * 0.15;
    for target in world.targets().iter() {
        let Some(plan) = &target.plan else {
            continue;
        };
        let color = if plan.path.reaches { Color::srgb(0.2, 0.9, 0.3) } else { Color::srgb(1.0, 0.55, 0.1) };
        let point = |p: api::WorldPos| coords::render_translation(p.translate(lift), origin.cell);
        let rest = &plan.path.points[plan.at..];
        gizmos.linestrip(std::iter::once(point(target.pos)).chain(rest.iter().map(|(p, _)| point(*p))), color);
        for (p, step) in rest {
            if *step == sim::path::Step::Hop {
                let ring = Isometry3d::new(point(*p), Quat::from_rotation_arc(Vec3::Z, Vec3::Y));
                gizmos.circle(ring, 0.3, Color::srgb(1.0, 1.0, 0.3));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_stop_holds_the_longest_freeze_and_counts_down() {
        let mut stop = HitStop::default();
        assert!(!stop.frozen());
        stop.freeze(0.1);
        stop.freeze(0.05);
        assert!((stop.remaining - 0.1).abs() < 1e-6, "a shorter freeze doesn't cut a longer one short");
        stop.update(0.06);
        assert!(stop.frozen());
        stop.update(0.06);
        assert!(!stop.frozen());
        assert_eq!(stop.remaining, 0.0);
    }

    #[test]
    fn a_bigger_hit_freezes_longer_up_to_the_cap_and_a_break_adds_more() {
        let feel = CombatFeel::default();
        let freeze = |damage: f32, broke: bool| {
            let (mut stop, mut kick, mut weapon) = (HitStop::default(), ViewKick::default(), WeaponState::default());
            land_hit(&feel, damage, broke, &mut stop, &mut kick, &mut weapon);
            assert!(kick.pitch() > 0.0 && weapon.recoil_velocity > 0.0, "the view and the tool react");
            stop.remaining
        };
        assert!(freeze(10.0, false) > freeze(2.0, false));
        assert!((freeze(1000.0, false) - feel.hitstop_max).abs() < 1e-6);
        assert!((freeze(5.0, true) - freeze(5.0, false) - feel.break_hitstop).abs() < 1e-6);
    }

    #[test]
    fn the_view_kick_and_shake_settle() {
        let feel = CombatFeel::default();
        let mut kick = ViewKick::default();
        kick.kick(feel.kick);
        kick.shake(feel.shake, feel.shake_time);
        kick.update(&feel, 1.0 / 60.0);
        assert!(kick.pitch() > 0.0);
        for _ in 0..120 {
            kick.update(&feel, 1.0 / 60.0);
        }
        assert!(kick.pitch().abs() < 1e-4);
        assert_eq!(kick.shake_offset(), Vec3::ZERO);
    }

    #[test]
    fn a_hit_tumbles_a_target_away_from_the_hit() {
        let feel = CombatFeel::default();
        let mut view = TargetView {
            entity: Entity::PLACEHOLDER,
            inner: Entity::PLACEHOLDER,
            turn: Entity::PLACEHOLDER,
            turn_extra: Quat::IDENTITY,
            material: Handle::default(),
            rig: None,
            tilt: Quat::IDENTITY,
            spin: Vec3::ZERO,
            flash: 0.0,
            tint: Vec3::ONE,
            squash: (f32::INFINITY, 0.0, Vec3::Y),
            center: WorldPos::default(),
            velocity: ZVec3::ZERO,
            was_on_ground: true,
        };
        react(&mut view, &feel, ZVec3::new(4.0, 0.0, 2.0), CRIT_TINT);
        assert_eq!(view.tint, CRIT_TINT);
        assert_eq!(view.flash, feel.flash_time);
        // Spinning the top of the target a moment moves it east, the way it was pushed.
        let top = coords::to_render(ZVec3::Z);
        let moved = Quat::from_scaled_axis(view.spin * 0.05) * top - top;
        assert!(coords::from_render(moved).x > 0.0, "tips away: {moved}");
    }

    #[test]
    fn the_readout_says_what_made_a_hit_count() {
        assert_eq!(hit_words(5.0, Some("body"), false, 0.0, false), "5");
        assert_eq!(hit_words(10.0, Some("head"), true, 0.0, false), "HEAD  10");
        assert_eq!(hit_words(9.2, None, false, 8.4, true), "9  +8.4 m/s  BROKEN");
    }

    #[test]
    fn every_feel_value_is_tunable() {
        let mut feel = CombatFeel::default();
        for t in feel.tunables() {
            assert!(t.min <= t.value && t.value <= t.max, "{} default outside its range", t.name);
            assert!(feel.set_tunable(&t.name, t.max));
        }
        assert!(!feel.set_tunable("nonsense", 1.0));
    }
}
