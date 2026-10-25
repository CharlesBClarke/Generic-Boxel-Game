//! The Bevy client: opens a window, feeds input to the simulation at a fixed 60 Hz, and
//! draws the world relative to a floating render origin, interpolated between ticks.

pub mod controls;
pub mod coords;
pub mod mesher;

use std::time::{Duration, Instant};

use api::{ChunkPos, I64Vec3, Vec3 as ZVec3, WorldPos};
use bevy::asset::RenderAssetUsages;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PresentMode, PrimaryWindow};
use sim::{PlayerInput, Simulation};

use crate::controls::{Controls, label};
use crate::mesher::MeshData;

/// How far, in boxels along any axis, the camera may get from the render origin before
/// the origin recenters on it.
pub const RECENTER_DISTANCE: i64 = 256;

const MOUSE_SENSITIVITY: f32 = 0.0025;

/// Runs the game until the window closes. `sim` must already have mods loaded and a player.
pub fn run(sim: Simulation) {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: "Boxel".into(), ..default() }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.55, 0.75, 0.95)))
        .insert_resource(Time::<Fixed>::from_hz(Simulation::TICK_RATE))
        .insert_resource(Sim(sim))
        .insert_resource(Controls::load())
        .init_resource::<Look>()
        .init_resource::<PendingToggles>()
        .init_resource::<RenderOrigin>()
        .init_resource::<FrameCap>()
        .init_resource::<WaypointCursor>()
        .add_systems(Startup, (setup_scene, spawn_chunk_meshes))
        .add_systems(FixedUpdate, tick_simulation)
        .add_systems(
            Update,
            (
                grab_cursor,
                mouse_look,
                debug_keys,
                (update_camera, sync_chunk_transforms).chain(),
                update_hud,
            )
                .chain(),
        )
        .add_systems(Last, limit_frame_rate)
        .run();
}

#[derive(Resource)]
pub struct Sim(pub Simulation);

/// The camera's facing, applied every frame rather than every tick so looking feels instant.
#[derive(Resource, Default)]
struct Look {
    yaw: f32,
    pitch: f32,
}

/// One-shot inputs seen in `Update`, held until the next tick consumes them. A frame can
/// run zero ticks, so reading presses inside `FixedUpdate` would drop some.
#[derive(Resource, Default)]
struct PendingToggles {
    toggle_fly: bool,
}

/// The world view's origin. Everything is drawn relative to this cell, so the GPU only
/// sees small numbers. One per world view; there is one world view for now.
#[derive(Resource, Default)]
pub struct RenderOrigin {
    pub cell: I64Vec3,
}

#[derive(Resource, Default)]
struct WaypointCursor(usize);

#[derive(Component)]
struct ChunkMesh(ChunkPos);

#[derive(Component)]
struct PlayerCamera;

#[derive(Component)]
struct Hud;

fn setup_scene(mut commands: Commands, mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>) {
    commands.spawn((Camera3d::default(), Transform::default(), PlayerCamera));
    commands.spawn((
        Text::new(""),
        Node { position_type: PositionType::Absolute, top: px(8), left: px(8), ..default() },
        Hud,
    ));
    set_grab(&mut cursor, true);
}

fn spawn_chunk_meshes(
    mut commands: Commands,
    sim: Res<Sim>,
    origin: Res<RenderOrigin>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Unlit: shading is baked into vertex colors until there is a lighting system.
    let material = materials.add(StandardMaterial { unlit: true, ..default() });
    let grid = sim.0.universe().main();
    for (pos, _) in grid.chunks() {
        let data = mesher::mesh_chunk(grid, pos, sim.0.blocks());
        if data.is_empty() {
            continue;
        }
        commands.spawn((
            Mesh3d(meshes.add(to_bevy_mesh(data))),
            MeshMaterial3d(material.clone()),
            chunk_transform(pos, origin.cell),
            ChunkMesh(pos),
        ));
    }
}

fn to_bevy_mesh(data: MeshData) -> Mesh {
    let convert = |v: Vec<[f32; 3]>| -> Vec<[f32; 3]> {
        v.into_iter().map(|p| coords::to_render(Vec3::from_array(p)).to_array()).collect()
    };
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, convert(data.positions))
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, convert(data.normals))
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, data.colors)
        .with_inserted_indices(Indices::U32(data.indices))
}

fn chunk_transform(pos: ChunkPos, origin: I64Vec3) -> Transform {
    Transform::from_translation(coords::render_translation(WorldPos::at_boxel(pos.origin()), origin))
}

fn tick_simulation(
    mut sim: ResMut<Sim>,
    keys: Res<ButtonInput<KeyCode>>,
    controls: Res<Controls>,
    look: Res<Look>,
    mut toggles: ResMut<PendingToggles>,
) {
    let axis = |pos: KeyCode, neg: KeyCode| keys.pressed(pos) as i32 as f32 - keys.pressed(neg) as i32 as f32;
    let input = PlayerInput {
        movement: Vec2::new(axis(controls.right, controls.left), axis(controls.forward, controls.back)),
        vertical: axis(controls.jump, controls.down),
        yaw: look.yaw,
        jump: keys.pressed(controls.jump),
        toggle_fly: std::mem::take(&mut toggles.toggle_fly),
    };
    sim.0.step(&input);
}

fn grab_cursor(
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if mouse.just_pressed(MouseButton::Left) {
        set_grab(&mut cursor, true);
    }
    if keys.just_pressed(KeyCode::Escape) {
        set_grab(&mut cursor, false);
    }
}

fn set_grab(cursor: &mut CursorOptions, grab: bool) {
    cursor.grab_mode = if grab { CursorGrabMode::Locked } else { CursorGrabMode::None };
    cursor.visible = !grab;
}

fn mouse_look(
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    mut look: ResMut<Look>,
) {
    if cursor.grab_mode == CursorGrabMode::None {
        return;
    }
    look.yaw = (look.yaw - motion.delta.x * MOUSE_SENSITIVITY).rem_euclid(std::f32::consts::TAU);
    let limit = 89f32.to_radians();
    look.pitch = (look.pitch - motion.delta.y * MOUSE_SENSITIVITY).clamp(-limit, limit);
}

fn debug_keys(
    keys: Res<ButtonInput<KeyCode>>,
    controls: Res<Controls>,
    mut sim: ResMut<Sim>,
    mut toggles: ResMut<PendingToggles>,
    mut cap: ResMut<FrameCap>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    mut waypoint: ResMut<WaypointCursor>,
) {
    if keys.just_pressed(controls.fly) {
        toggles.toggle_fly = !toggles.toggle_fly;
    }
    if keys.just_pressed(controls.teleport) && !sim.0.waypoints().is_empty() {
        waypoint.0 = (waypoint.0 + 1) % sim.0.waypoints().len();
        let target = sim.0.waypoints()[waypoint.0].1;
        sim.0.teleport_player(target);
    }
    if keys.just_pressed(controls.frame_cap) {
        cap.mode = cap.mode.next();
        window.present_mode = match cap.mode {
            CapMode::Vsync => PresentMode::AutoVsync,
            _ => PresentMode::AutoNoVsync,
        };
    }
}

/// Places the camera at the player's eyes, blended between the last two ticks, and
/// recenters the render origin if the camera has strayed too far from it.
fn update_camera(
    sim: Res<Sim>,
    look: Res<Look>,
    fixed: Res<Time<Fixed>>,
    mut origin: ResMut<RenderOrigin>,
    mut camera: Single<&mut Transform, With<PlayerCamera>>,
) {
    let Some(player) = sim.0.player() else {
        return;
    };
    let alpha = fixed.overstep_fraction();
    let prev = player.prev_pos();
    let feet = prev.translate(prev.delta_to(player.pos()) * alpha);
    let eye = feet.translate(ZVec3::new(0.0, 0.0, player.def().eye_height));

    let from_origin = (eye.cell() - origin.cell).abs().max_element();
    if from_origin > RECENTER_DISTANCE {
        origin.cell = eye.cell();
    }
    camera.translation = coords::render_translation(eye, origin.cell);
    camera.rotation = coords::look_rotation(look.yaw, look.pitch);
}

fn sync_chunk_transforms(origin: Res<RenderOrigin>, mut chunks: Query<(&ChunkMesh, &mut Transform)>) {
    if !origin.is_changed() {
        return;
    }
    for (chunk, mut transform) in &mut chunks {
        *transform = chunk_transform(chunk.0, origin.cell);
    }
}

fn update_hud(
    sim: Res<Sim>,
    origin: Res<RenderOrigin>,
    cap: Res<FrameCap>,
    controls: Res<Controls>,
    time: Res<Time>,
    mut text: Single<&mut Text, With<Hud>>,
) {
    let Some(player) = sim.0.player() else {
        return;
    };
    let pos = player.pos();
    let precise = pos.cell().as_dvec3() + pos.offset().as_dvec3();
    let fps = 1.0 / time.delta_secs().max(1e-6);
    text.0 = format!(
        "pos {:.2}, {:.2}, {:.2}\norigin {}, {}, {}\n{} | {:.0} fps | cap {} ({})\n{}{}{}{} move, {} jump, {} down, {} fly, {} teleport, Esc release mouse",
        precise.x,
        precise.y,
        precise.z,
        origin.cell.x,
        origin.cell.y,
        origin.cell.z,
        if player.flying() { "flying" } else { "walking" },
        fps,
        cap.mode.label(),
        label(controls.frame_cap),
        label(controls.forward),
        label(controls.left),
        label(controls.back),
        label(controls.right),
        label(controls.jump),
        label(controls.down),
        label(controls.fly),
        label(controls.teleport),
    );
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
enum CapMode {
    #[default]
    Vsync,
    Fps(u32),
    Uncapped,
}

impl CapMode {
    fn next(self) -> Self {
        match self {
            CapMode::Vsync => CapMode::Fps(30),
            CapMode::Fps(30) => CapMode::Fps(60),
            CapMode::Fps(60) => CapMode::Fps(144),
            CapMode::Fps(_) => CapMode::Uncapped,
            CapMode::Uncapped => CapMode::Vsync,
        }
    }

    fn label(self) -> String {
        match self {
            CapMode::Vsync => "vsync".into(),
            CapMode::Fps(n) => format!("{n}"),
            CapMode::Uncapped => "none".into(),
        }
    }
}

/// A frame rate cap for checking that motion stays smooth at rates other than 60.
#[derive(Resource)]
struct FrameCap {
    mode: CapMode,
    last_frame: Instant,
}

impl Default for FrameCap {
    fn default() -> Self {
        Self { mode: CapMode::default(), last_frame: Instant::now() }
    }
}

fn limit_frame_rate(mut cap: ResMut<FrameCap>) {
    if let CapMode::Fps(fps) = cap.mode {
        let target = Duration::from_secs_f64(1.0 / fps as f64);
        let elapsed = cap.last_frame.elapsed();
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
    }
    cap.last_frame = Instant::now();
}
