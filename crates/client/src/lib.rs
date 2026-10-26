//! The Bevy client: opens a window, feeds input to the simulation at a fixed 60 Hz, and
//! draws the world relative to a floating render origin, interpolated between ticks.

pub mod controls;
pub mod coords;
pub mod editing;
pub mod feel;
pub mod mesher;
pub mod tuning;

use std::time::{Duration, Instant};

use api::{ChunkPos, I64Vec3, Vec3 as ZVec3, WorldPos};
use bevy::asset::RenderAssetUsages;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PresentMode, PrimaryWindow};
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
use sim::{PlayerInput, Simulation};

use crate::controls::{Controls, label};
use crate::editing::{ChunkMeshes, Palette, PaletteHud};
use crate::feel::{CameraFeel, FeelState, Motion};
use crate::mesher::MeshData;
use crate::tuning::TuningPanel;

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
        .add_plugins(EguiPlugin::default())
        .insert_resource(ClearColor(Color::srgb(0.55, 0.75, 0.95)))
        .insert_resource(Time::<Fixed>::from_hz(Simulation::TICK_RATE))
        .insert_resource(Sim(sim))
        .insert_resource(Controls::load())
        .init_resource::<Look>()
        .init_resource::<PendingToggles>()
        .init_resource::<RenderOrigin>()
        .init_resource::<FrameCap>()
        .init_resource::<WaypointCursor>()
        .init_resource::<CameraFeel>()
        .init_resource::<FeelState>()
        .init_resource::<TuningPanel>()
        .init_resource::<Palette>()
        .add_systems(Startup, (setup_scene, spawn_chunk_meshes))
        .add_systems(FixedUpdate, tick_simulation)
        .add_systems(
            Update,
            (
                // Before grab_cursor, so the click that captures the mouse doesn't edit.
                editing::edit_input,
                grab_cursor,
                mouse_look,
                debug_keys,
                editing::remesh_changed_chunks,
                (update_camera, sync_chunk_transforms).chain(),
                editing::draw_outline,
                update_hud,
                editing::update_palette_hud,
            )
                .chain(),
        )
        .add_systems(EguiPrimaryContextPass, tuning::tuning_panel)
        .add_systems(Last, limit_frame_rate)
        .run();
}

#[derive(Resource)]
pub struct Sim(pub Simulation);

/// The camera's facing, applied every frame rather than every tick so looking feels instant.
#[derive(Resource, Default)]
pub(crate) struct Look {
    yaw: f32,
    pitch: f32,
}

/// One-shot inputs seen in `Update`, held until the next tick consumes them. A frame can
/// run zero ticks, so reading presses inside `FixedUpdate` would drop some.
#[derive(Resource, Default)]
pub(crate) struct PendingToggles {
    toggle_fly: bool,
    jump_pressed: bool,
    break_pressed: bool,
    place_pressed: bool,
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
pub(crate) struct ChunkMesh(ChunkPos);

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
    // A crosshair in the middle of the screen, and the block palette along the bottom.
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_child(Text::new("+"));
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            bottom: px(12),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_child((Text::new(""), PaletteHud));
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
    let mut entities = std::collections::HashMap::new();
    for (pos, _) in grid.chunks() {
        let data = mesher::mesh_chunk(grid, pos, sim.0.blocks());
        if data.is_empty() {
            continue;
        }
        entities.insert(pos, spawn_chunk(&mut commands, &mut meshes, material.clone(), pos, data, origin.cell));
    }
    commands.insert_resource(ChunkMeshes { material, entities });
}

pub(crate) fn spawn_chunk(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<StandardMaterial>,
    pos: ChunkPos,
    data: MeshData,
    origin: I64Vec3,
) -> Entity {
    commands
        .spawn((Mesh3d(meshes.add(to_bevy_mesh(data))), MeshMaterial3d(material), chunk_transform(pos, origin), ChunkMesh(pos)))
        .id()
}

pub(crate) fn to_bevy_mesh(data: MeshData) -> Mesh {
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

#[allow(clippy::too_many_arguments)]
fn tick_simulation(
    mut sim: ResMut<Sim>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    controls: Res<Controls>,
    look: Res<Look>,
    palette: Res<Palette>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    mut toggles: ResMut<PendingToggles>,
) {
    let axis = |pos: KeyCode, neg: KeyCode| keys.pressed(pos) as i32 as f32 - keys.pressed(neg) as i32 as f32;
    let playing = editing::playing(&cursor);
    let input = PlayerInput {
        movement: Vec2::new(axis(controls.right, controls.left), axis(controls.forward, controls.back)),
        vertical: axis(controls.jump, controls.down),
        yaw: look.yaw,
        pitch: look.pitch,
        jump_pressed: std::mem::take(&mut toggles.jump_pressed),
        jump_held: keys.pressed(controls.jump),
        sprint: keys.pressed(controls.sprint),
        toggle_fly: std::mem::take(&mut toggles.toggle_fly),
        break_pressed: std::mem::take(&mut toggles.break_pressed),
        break_held: playing && controls.break_block.pressed(&keys, &mouse),
        place_pressed: std::mem::take(&mut toggles.place_pressed),
        place_held: playing && controls.place_block.pressed(&keys, &mouse),
        place_block: palette.selected(sim.0.blocks()),
    };
    sim.0.step(&input);
}

fn grab_cursor(
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    controls: Res<Controls>,
    mut panel: ResMut<TuningPanel>,
) {
    if keys.just_pressed(controls.tuning) {
        panel.open = !panel.open;
        set_grab(&mut cursor, !panel.open);
    }
    // While the panel is open, clicks belong to it.
    if mouse.just_pressed(MouseButton::Left) && !panel.open {
        set_grab(&mut cursor, true);
    }
    if keys.just_pressed(KeyCode::Escape) {
        panel.open = false;
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
    if keys.just_pressed(controls.jump) {
        toggles.jump_pressed = true;
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

/// Places the camera at the player's eyes, blended between the last two ticks, adds the
/// camera feel effects, and recenters the render origin if the camera has strayed too far.
#[allow(clippy::too_many_arguments)]
fn update_camera(
    sim: Res<Sim>,
    look: Res<Look>,
    fixed: Res<Time<Fixed>>,
    time: Res<Time>,
    feel: Res<CameraFeel>,
    mut feel_state: ResMut<FeelState>,
    mut origin: ResMut<RenderOrigin>,
    mut camera: Single<(&mut Transform, &mut Projection), With<PlayerCamera>>,
) {
    let Some(player) = sim.0.player() else {
        return;
    };
    let alpha = fixed.overstep_fraction();
    let prev = player.prev_pos();
    let step = prev.delta_to(player.pos());
    let feet = prev.translate(step * alpha);

    feel_state.update(
        &feel,
        &Motion {
            horizontal_speed: player.velocity().truncate().length(),
            grounded: player.on_ground() && !player.flying(),
            landings: player.landings(),
            last_landing_speed: player.last_landing_speed(),
            stepped_total: player.stepped_total(),
        },
        time.delta_secs(),
    );
    let eye_height = player.shape().eye_height + feel_state.vertical_offset(&feel);
    let eye = feet.translate(ZVec3::new(0.0, 0.0, eye_height));

    let from_origin = (eye.cell() - origin.cell).abs().max_element();
    if from_origin > RECENTER_DISTANCE {
        origin.cell = eye.cell();
    }
    let (transform, projection) = &mut *camera;
    transform.translation = coords::render_translation(eye, origin.cell);
    let limit = 89f32.to_radians();
    transform.rotation = coords::look_rotation(look.yaw, (look.pitch - feel_state.nod()).clamp(-limit, limit));
    if let Projection::Perspective(perspective) = &mut **projection {
        perspective.fov = feel_state.fov.to_radians();
    }
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
    feel: Res<CameraFeel>,
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
        "pos {:.2}, {:.2}, {:.2}  speed {:.1} m/s  last landing {:.1} m/s ({})\norigin {}, {}, {}\n{} | {:.0} fps | cap {} ({})\n{}{}{}{} move, {} jump, {} sprint, {} down, {} fly, {} teleport, {} tuning, Esc release mouse\n{} break, {} place, 1-9 or scroll pick block",
        precise.x,
        precise.y,
        precise.z,
        player.velocity().truncate().length(),
        player.last_landing_speed(),
        if player.last_landing_speed() >= feel.hard_landing_speed { "hard" } else { "soft" },
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
        label(controls.sprint),
        label(controls.down),
        label(controls.fly),
        label(controls.teleport),
        label(controls.tuning),
        controls.break_block.label(),
        controls.place_block.label(),
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
