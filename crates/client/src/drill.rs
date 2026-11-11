//! The chase drill: a temporary tool for feeling the movement, not a feature. A ghost floats
//! straight at the player through walls and resets the score on touch; objectives appear at
//! random landmarks. A tool's strike knocks the ghost back and dazes it for a moment. Meant to be deleted once later toys no longer need it, so it lives here
//! on its own rather than in the API (see the API debt list in `SLICES.md`).
//!
//! Landmarks are waypoints whose names start with `landmark:`, supplied by the mod.

use api::{Tunable, Tunables, Vec3 as ZVec3, WorldPos};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::controls::Controls;
use crate::world::ClientWorld;
use crate::{RenderOrigin, coords};

/// Waypoint name prefix that marks a landmark.
pub const LANDMARK_PREFIX: &str = "landmark:";
/// Only landmarks this close count, so the drill stays in whichever copy of the area the
/// player is in.
const NEAR: f32 = 300.0;
/// If the ghost is farther than this, the player teleported; bring the ghost along.
const LOST: f32 = 400.0;
/// Objective markers float this high above the landmark's floor, about chest height.
const OBJECTIVE_HEIGHT: f32 = 0.9;
/// The ghost's radius, as drawn and as struck.
const GHOST_RADIUS: f32 = 0.45;
/// How long a struck ghost flashes, seconds.
const GHOST_FLASH: f32 = 0.12;

#[derive(Clone, Debug, PartialEq)]
pub struct DrillTuning {
    /// Ghost speed at score 0, meters per second.
    pub ghost_speed: f32,
    /// Extra ghost speed per point.
    pub speed_per_point: f32,
    /// How far away the ghost appears, meters.
    pub spawn_distance: f32,
    /// The ghost catches the player within this distance of the body's center.
    pub catch_radius: f32,
    /// An objective counts as reached within this distance of the body's center.
    pub reach_radius: f32,
    /// Speed a strike knocks the ghost back at, meters per second.
    pub ghost_knockback: f32,
    /// How quickly the knock wears off, per second.
    pub knock_drag: f32,
    /// Seconds a struck ghost stops chasing.
    pub daze_time: f32,
}

impl Default for DrillTuning {
    fn default() -> Self {
        Self {
            ghost_speed: 5.5,
            speed_per_point: 0.25,
            spawn_distance: 40.0,
            catch_radius: 0.9,
            reach_radius: 1.3,
            ghost_knockback: 18.0,
            knock_drag: 4.0,
            daze_time: 0.6,
        }
    }
}

/// The tuning panel's groups for [`DrillTuning`]'s values.
const DRILL_GROUPS: &[(&str, &[&str])] = &[
    ("Ghost", &["ghost_speed", "speed_per_point", "spawn_distance"]),
    ("Reaching and catching", &["catch_radius", "reach_radius"]),
    ("Hitting the ghost", &["ghost_knockback", "knock_drag", "daze_time"]),
];
/// What each of [`DrillTuning`]'s values does, for the tuning panel.
const DRILL_HELP: &[(&str, &str)] = &[
    ("ghost_speed", "The chasing ghost's speed at score 0, m/s."),
    ("speed_per_point", "Extra ghost speed per point scored, m/s."),
    ("spawn_distance", "How far away the ghost appears, meters."),
    ("catch_radius", "The ghost catches you within this distance of your body's center, meters."),
    ("reach_radius", "An objective counts as reached within this distance of your body's center, meters."),
    ("ghost_knockback", "Speed a strike knocks the ghost back at, m/s."),
    ("knock_drag", "How quickly the knock wears off, per second."),
    ("daze_time", "Seconds a struck ghost stops chasing."),
];

impl Tunables for DrillTuning {
    fn tunables(&self) -> Vec<Tunable> {
        api::tuning::described(vec![
            Tunable::new("ghost_speed", self.ghost_speed, 0.0, 20.0),
            Tunable::new("speed_per_point", self.speed_per_point, 0.0, 2.0),
            Tunable::new("spawn_distance", self.spawn_distance, 5.0, 150.0),
            Tunable::new("catch_radius", self.catch_radius, 0.2, 3.0),
            Tunable::new("reach_radius", self.reach_radius, 0.3, 4.0),
            Tunable::new("ghost_knockback", self.ghost_knockback, 0.0, 60.0),
            Tunable::new("knock_drag", self.knock_drag, 0.0, 20.0),
            Tunable::new("daze_time", self.daze_time, 0.0, 3.0),
        ], DRILL_GROUPS, DRILL_HELP)
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        let field = match name {
            "ghost_speed" => &mut self.ghost_speed,
            "speed_per_point" => &mut self.speed_per_point,
            "spawn_distance" => &mut self.spawn_distance,
            "catch_radius" => &mut self.catch_radius,
            "reach_radius" => &mut self.reach_radius,
            "ghost_knockback" => &mut self.ghost_knockback,
            "knock_drag" => &mut self.knock_drag,
            "daze_time" => &mut self.daze_time,
            _ => return false,
        };
        *field = value;
        true
    }
}

/// A landmark: its name without the prefix, and where its floor is.
#[derive(Clone, Debug, PartialEq)]
pub struct Landmark {
    pub name: String,
    pub pos: WorldPos,
}

impl Landmark {
    fn target(&self) -> WorldPos {
        self.pos.translate(ZVec3::Z * OBJECTIVE_HEIGHT)
    }
}

/// The landmarks among `waypoints` near `near`.
pub fn landmarks(waypoints: &[(String, WorldPos)], near: WorldPos) -> Vec<Landmark> {
    waypoints
        .iter()
        .filter_map(|(name, pos)| {
            let name = name.strip_prefix(LANDMARK_PREFIX)?;
            (near.delta_to(*pos).length() < NEAR).then(|| Landmark { name: name.into(), pos: *pos })
        })
        .collect()
}

/// What happened on a tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DrillEvent {
    Scored,
    Caught,
}

#[derive(Resource)]
pub struct Drill {
    pub active: bool,
    pub tuning: DrillTuning,
    ghost: WorldPos,
    prev_ghost: WorldPos,
    objective: Option<Landmark>,
    score: u32,
    best: u32,
    rng: u64,
    /// The ghost's velocity from being struck, Z up.
    knock: ZVec3,
    /// Seconds until a struck ghost chases again.
    dazed: f32,
    /// Seconds of flash left from being struck, counted in real time.
    flash: f32,
}

impl Default for Drill {
    fn default() -> Self {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        Self::with_seed(seed)
    }
}

impl Drill {
    pub fn with_seed(seed: u64) -> Self {
        Self {
            active: false,
            tuning: DrillTuning::default(),
            ghost: WorldPos::default(),
            prev_ghost: WorldPos::default(),
            objective: None,
            score: 0,
            best: 0,
            rng: seed | 1,
            knock: ZVec3::ZERO,
            dazed: 0.0,
            flash: 0.0,
        }
    }

    pub fn score(&self) -> u32 {
        self.score
    }

    pub fn best(&self) -> u32 {
        self.best
    }

    pub fn ghost(&self) -> WorldPos {
        self.ghost
    }

    pub fn objective(&self) -> Option<&Landmark> {
        self.objective.as_ref()
    }

    /// Starts a fresh run around a player whose body center is at `center`.
    pub fn start(&mut self, center: WorldPos, landmarks: &[Landmark]) {
        self.score = 0;
        self.respawn_ghost(center);
        self.objective = None;
        self.pick_objective(center, landmarks);
    }

    /// Whether the ghost is dazed from a strike.
    pub fn dazed(&self) -> bool {
        self.dazed > 0.0
    }

    /// A strike from `eye` along `look` (a unit vector), out to `reach` and widened by `radius`.
    /// Walls don't stop it, since the ghost goes through them anyway. If it reaches the ghost
    /// during a run, the ghost is knocked away along the look and dazed. Returns whether it hit.
    pub fn hit(&mut self, eye: WorldPos, look: ZVec3, reach: f32, radius: f32) -> bool {
        if !self.active {
            return false;
        }
        let to_ghost = eye.delta_to(self.ghost);
        let along = to_ghost.dot(look).clamp(0.0, reach);
        if (to_ghost - look * along).length() > GHOST_RADIUS + radius || to_ghost.dot(look) < -GHOST_RADIUS {
            return false;
        }
        self.knock = look * self.tuning.ghost_knockback;
        self.dazed = self.tuning.daze_time;
        self.flash = GHOST_FLASH;
        true
    }

    /// Advances one tick of `dt` seconds.
    pub fn tick(&mut self, center: WorldPos, landmarks: &[Landmark], dt: f32) -> Option<DrillEvent> {
        self.prev_ghost = self.ghost;
        self.ghost = self.ghost.translate(self.knock * dt);
        self.knock *= (-self.tuning.knock_drag * dt).exp();
        self.dazed = (self.dazed - dt).max(0.0);
        let to_player = self.ghost.delta_to(center);
        if to_player.length() > LOST {
            self.respawn_ghost(center);
        } else if self.dazed <= 0.0 {
            let speed = self.tuning.ghost_speed + self.tuning.speed_per_point * self.score as f32;
            let step = (speed * dt).min(to_player.length());
            self.ghost = self.ghost.translate(to_player.normalize_or_zero() * step);
        }

        if self.ghost.delta_to(center).length() < self.tuning.catch_radius {
            self.score = 0;
            self.respawn_ghost(center);
            return Some(DrillEvent::Caught);
        }
        let reached = self.objective.as_ref().is_some_and(|o| o.target().delta_to(center).length() < self.tuning.reach_radius);
        if reached {
            self.score += 1;
            self.best = self.best.max(self.score);
        }
        if reached || self.objective.is_none() {
            self.pick_objective(center, landmarks);
        }
        reached.then_some(DrillEvent::Scored)
    }

    /// The ghost's position `alpha` of the way from the last tick to this one.
    pub fn ghost_at(&self, alpha: f32) -> WorldPos {
        self.prev_ghost.translate(self.prev_ghost.delta_to(self.ghost) * alpha)
    }

    fn respawn_ghost(&mut self, center: WorldPos) {
        let angle = self.random() as f32 / u64::MAX as f32 * std::f32::consts::TAU;
        let (sin, cos) = angle.sin_cos();
        self.ghost = center.translate(ZVec3::new(cos, sin, 0.0) * self.tuning.spawn_distance + ZVec3::Z * 3.0);
        self.prev_ghost = self.ghost;
        self.knock = ZVec3::ZERO;
        self.dazed = 0.0;
    }

    /// A random landmark other than the current one, preferring ones at least 15 meters away.
    fn pick_objective(&mut self, center: WorldPos, landmarks: &[Landmark]) {
        let current = self.objective.as_ref().map(|o| o.name.clone());
        let others: Vec<&Landmark> = landmarks.iter().filter(|l| Some(&l.name) != current.as_ref()).collect();
        let far: Vec<&Landmark> = others.iter().copied().filter(|l| l.target().delta_to(center).length() >= 15.0).collect();
        let pool = if far.is_empty() { others } else { far };
        if pool.is_empty() {
            return;
        }
        let i = (self.random() % pool.len() as u64) as usize;
        self.objective = Some(pool[i].clone());
    }

    /// xorshift64: plenty for picking spots, and no dependency.
    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }
}

/// The ghost's sphere, with its own material so it can flash.
#[derive(Component)]
pub(crate) struct GhostMarker(Handle<StandardMaterial>);

/// The ghost's color at rest.
const GHOST_COLOR: Color = Color::srgba(0.85, 0.9, 1.0, 0.55);

#[derive(Component)]
pub(crate) struct ObjectiveMarker;

#[derive(Component)]
pub(crate) struct DrillHud;

pub(crate) fn setup_drill(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let ghost = materials.add(StandardMaterial { base_color: GHOST_COLOR, alpha_mode: AlphaMode::Blend, unlit: true, ..default() });
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(GHOST_RADIUS))),
        MeshMaterial3d(ghost.clone()),
        Transform::default(),
        Visibility::Hidden,
        GhostMarker(ghost),
        NotShadowCaster,
    ));

    let glow = materials.add(StandardMaterial { base_color: Color::srgb(0.3, 1.0, 0.4), unlit: true, ..default() });
    let beam = materials.add(StandardMaterial {
        base_color: Color::srgba(0.3, 1.0, 0.4, 0.3),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });
    // A beam above the objective so it can be found from anywhere.
    let beam_height = 60.0;
    commands
        .spawn((Mesh3d(meshes.add(Cuboid::from_length(0.5))), MeshMaterial3d(glow), Transform::default(), Visibility::Hidden, ObjectiveMarker, NotShadowCaster))
        .with_child((
            Mesh3d(meshes.add(Cuboid::new(0.12, 0.12, beam_height))),
            MeshMaterial3d(beam),
            NotShadowCaster,
            Transform::from_translation(coords::to_render(ZVec3::Z * beam_height * 0.5))
                .with_rotation(Quat::from_rotation_arc(Vec3::Z, coords::to_render(ZVec3::Z))),
        ));
    commands.spawn((
        Text::new(""),
        Node { position_type: PositionType::Absolute, top: px(8), right: px(8), ..default() },
        DrillHud,
        crate::dev::DevText,
    ));
}

fn player_center(world: &ClientWorld) -> Option<WorldPos> {
    let player = world.player()?;
    Some(player.pos().translate(ZVec3::Z * player.height() * 0.5))
}

pub(crate) fn drill_keys(keys: Res<ButtonInput<KeyCode>>, controls: Res<Controls>, world: Res<ClientWorld>, mut drill: ResMut<Drill>) {
    if !keys.just_pressed(controls.drill) {
        return;
    }
    drill.active = !drill.active;
    if drill.active
        && let Some(center) = player_center(&world)
    {
        let marks = landmarks(world.waypoints(), center);
        drill.start(center, &marks);
    }
}

pub(crate) fn drill_tick(world: Res<ClientWorld>, mut drill: ResMut<Drill>) {
    if !drill.active {
        return;
    }
    let Some(center) = player_center(&world) else {
        return;
    };
    let marks = landmarks(world.waypoints(), center);
    if let Some(event) = drill.tick(center, &marks, protocol::TICK_SECONDS) {
        info!("chase drill: {event:?}, score {} (best {})", drill.score(), drill.best());
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn draw_drill(
    world: Res<ClientWorld>,
    mut drill: ResMut<Drill>,
    fixed: Res<Time<Fixed>>,
    time: Res<Time>,
    origin: Res<RenderOrigin>,
    controls: Res<Controls>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut ghost: Single<(&mut Transform, &mut Visibility, &GhostMarker), Without<ObjectiveMarker>>,
    mut objective: Single<(&mut Transform, &mut Visibility), (With<ObjectiveMarker>, Without<GhostMarker>)>,
    mut text: Single<&mut Text, With<DrillHud>>,
) {
    let shown = if drill.active { Visibility::Inherited } else { Visibility::Hidden };
    *ghost.1 = shown;
    *objective.1 = if drill.objective().is_some() { shown } else { Visibility::Hidden };
    if !drill.active {
        text.0 = format!("{} chase drill", crate::controls::label(controls.drill));
        return;
    }
    let ghost_pos = drill.ghost_at(fixed.overstep_fraction());
    ghost.0.translation = coords::render_translation(ghost_pos, origin.cell);
    // A struck ghost flashes solid white and swells for a moment.
    drill.flash = (drill.flash - time.delta_secs()).max(0.0);
    let flash = drill.flash / GHOST_FLASH;
    ghost.0.scale = Vec3::splat(1.0 + flash * 0.4);
    if let Some(material) = materials.get_mut(&ghost.2.0) {
        let wanted = if flash > 0.0 { Color::WHITE } else { GHOST_COLOR };
        if material.base_color != wanted {
            material.base_color = wanted;
        }
    }
    let center = player_center(&world);
    let distance = |pos: WorldPos| center.map_or(0.0, |c| c.delta_to(pos).length());
    let dazed = if drill.dazed() { " (dazed)" } else { "" };
    let mut line = format!("CHASE  score {}  best {}\nghost {:.0} m{dazed}", drill.score(), drill.best(), distance(ghost_pos));
    if let Some(target) = drill.objective() {
        objective.0.translation = coords::render_translation(target.target(), origin.cell);
        line += &format!("\nnext: {}, {:.0} m", target.name, distance(target.target()));
    }
    line += &format!("\n{} stop", crate::controls::label(controls.drill));
    text.0 = line;
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::I64Vec3;

    const DT: f32 = 1.0 / 60.0;

    fn at(x: f32, y: f32, z: f32) -> WorldPos {
        WorldPos::new(I64Vec3::ZERO, ZVec3::new(x, y, z))
    }

    fn marks() -> Vec<Landmark> {
        vec![
            Landmark { name: "a".into(), pos: at(30.0, 0.0, 0.0) },
            Landmark { name: "b".into(), pos: at(-30.0, 0.0, 0.0) },
            Landmark { name: "c".into(), pos: at(0.0, 30.0, 0.0) },
        ]
    }

    #[test]
    fn finds_nearby_landmarks_by_prefix() {
        let waypoints = vec![
            ("origin".to_string(), at(0.0, 0.0, 0.0)),
            ("landmark:pit".to_string(), at(10.0, 0.0, -10.0)),
            ("landmark:far".to_string(), WorldPos::new(I64Vec3::new(3_000_000, 0, 0), ZVec3::ZERO)),
        ];
        let found = landmarks(&waypoints, at(0.0, 0.0, 0.0));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "pit");
    }

    #[test]
    fn the_ghost_closes_in_at_its_speed() {
        let mut drill = Drill::with_seed(7);
        let player = at(0.5, 0.5, 0.9);
        drill.start(player, &marks());
        let before = drill.ghost().delta_to(player).length();
        assert!((before - (40f32.powi(2) + 9.0).sqrt()).abs() < 1e-3, "spawns at the set distance");
        for _ in 0..60 {
            drill.tick(player, &marks(), DT);
        }
        let after = drill.ghost().delta_to(player).length();
        assert!((before - after - 5.5).abs() < 1e-2, "moved {} in a second", before - after);
    }

    #[test]
    fn being_caught_resets_the_score_and_reaching_an_objective_scores() {
        let mut drill = Drill::with_seed(3);
        let player = at(0.5, 0.5, 0.9);
        drill.start(player, &marks());
        let target = drill.objective().unwrap().target();
        assert_eq!(drill.tick(target, &marks(), DT), Some(DrillEvent::Scored));
        assert_eq!(drill.score(), 1);
        assert_ne!(drill.objective().unwrap().target(), target, "a new objective appears elsewhere");

        // Stand still until the ghost arrives.
        let mut caught = false;
        for _ in 0..60 * 20 {
            if drill.tick(target, &marks(), DT) == Some(DrillEvent::Caught) {
                caught = true;
                break;
            }
        }
        assert!(caught);
        assert_eq!(drill.score(), 0);
        assert_eq!(drill.best(), 1);
        assert!(drill.ghost().delta_to(target).length() > 30.0, "the ghost respawns far away");
    }

    #[test]
    fn a_strike_in_reach_knocks_the_ghost_back_and_dazes_it() {
        let mut drill = Drill::with_seed(9);
        let player = at(0.5, 0.5, 0.9);
        drill.active = true;
        drill.start(player, &marks());
        // Let it come close, then strike straight at it.
        while drill.ghost().delta_to(player).length() > 2.0 {
            drill.tick(player, &marks(), DT);
        }
        let eye = player.translate(ZVec3::Z * 0.7);
        let look = eye.delta_to(drill.ghost()).normalize();
        let side = look.cross(ZVec3::Z).normalize();
        assert!(!drill.hit(eye, side, 2.5, 0.3), "aimed away");
        assert!(!drill.hit(eye, look, 1.0, 0.3), "out of reach");
        assert!(drill.hit(eye, look, 2.5, 0.3));
        assert!(drill.dazed());
        let before = drill.ghost().delta_to(player).length();
        for _ in 0..20 {
            assert_eq!(drill.tick(player, &marks(), DT), None);
        }
        let after = drill.ghost().delta_to(player).length();
        assert!(after > before + 2.0, "knocked back from {before} to {after}");
        let mut ticks = 20;
        while drill.dazed() {
            drill.tick(player, &marks(), DT);
            ticks += 1;
        }
        let expected = (drill.tuning.daze_time / DT).round() as i64;
        assert!((ticks - expected).abs() <= 1, "dazed for its daze time: {ticks} ticks");
        let later = drill.ghost().delta_to(player).length();
        for _ in 0..10 {
            drill.tick(player, &marks(), DT);
        }
        assert!(drill.ghost().delta_to(player).length() < later, "chasing again");
    }

    #[test]
    fn strikes_miss_the_ghost_when_the_drill_is_off() {
        let mut drill = Drill::with_seed(9);
        let ghost = drill.ghost();
        assert!(!drill.hit(ghost.translate(ZVec3::X * -1.0), ZVec3::X, 2.0, 0.3));
    }

    #[test]
    fn the_ghost_follows_teleports() {
        let mut drill = Drill::with_seed(5);
        drill.start(at(0.5, 0.5, 0.9), &marks());
        let far = WorldPos::new(I64Vec3::new(3_000_000, -2_000_000, 1_000), ZVec3::splat(0.5));
        drill.tick(far, &[], DT);
        assert!(drill.ghost().delta_to(far).length() < 50.0);
    }
}
