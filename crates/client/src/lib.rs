//! The Bevy client: opens a window, sends input to the server at a fixed 60 Hz, and draws its
//! copy of the world (see `world`) relative to a floating render origin, interpolated between
//! ticks. It never touches the simulation itself.

pub mod animation;
pub mod combat;
pub mod controls;
pub mod coords;
pub mod crafting;
pub mod dev;
pub mod drill;
pub mod editing;
pub mod feel;
pub mod hurt;
pub mod items;
pub mod mesher;
pub mod perf;
pub mod shading;
pub mod solid;
pub mod sound;
pub mod tuning;
pub mod tuning_file;
pub mod view;
pub mod volumes;
pub mod world;
pub mod world_menu;

use std::time::{Duration, Instant};

use api::{ChunkPos, I64Vec3, Vec3 as ZVec3, WorldPos};
use bevy::asset::RenderAssetUsages;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PresentMode, PrimaryWindow};
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
use protocol::{ClientMsg, Command, Connection, WorldNote};
use sim::PlayerInput;

use crate::controls::{Controls, DoubleTapSprint, SprintToggle, label};
use crate::editing::{ChunkMeshes, Ghost, HiddenGhosts, Palette, PartNames};
use crate::feel::{CameraFeel, FeelState, Motion};
use crate::mesher::MeshData;
use crate::solid::{ATTRIBUTE_EDGES, ATTRIBUTE_FACE, ATTRIBUTE_SOLID, SolidMaterial, SolidPlugin};
use crate::tuning::TuningPanel;
use crate::volumes::Volumes;
use crate::world::{ClientWorld, Link};

/// How far, in boxels along any axis, the camera may get from the render origin before
/// the origin recenters on it.
pub const RECENTER_DISTANCE: i64 = 256;

const MOUSE_SENSITIVITY: f32 = 0.0025;

/// Runs the game until the window closes, talking to a server through `connection`. The server's
/// world must already have its mods loaded and a player.
pub fn run(connection: Box<dyn Connection>) {
    let mut link = Link::new(connection);
    let Some(world) = link.connect() else {
        eprintln!("the server closed before welcoming the client");
        return;
    };
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: "Boxel".into(), ..default() }),
            ..default()
        }))
        .add_plugins((EguiPlugin::default(), SolidPlugin, shading::ShadingPlugin))
        .insert_resource(ClearColor(Color::srgb(0.55, 0.75, 0.95)))
        .insert_resource(Time::<Fixed>::from_hz(protocol::TICK_RATE))
        .insert_resource(world)
        .insert_resource(link)
        .insert_resource(Controls::load())
        .init_resource::<Look>()
        .init_resource::<PendingToggles>()
        .init_resource::<RenderOrigin>()
        .init_resource::<FrameCap>()
        .init_resource::<WaypointCursor>()
        .init_resource::<CameraFeel>()
        .init_resource::<mesher::AoTuning>()
        .init_resource::<view::ViewTuning>()
        .init_resource::<view::ViewStats>()
        .init_resource::<FeelState>()
        .init_resource::<TuningPanel>()
        .init_resource::<dev::DevMode>()
        .init_resource::<Palette>()
        .init_resource::<crafting::CraftingMenu>()
        .init_resource::<world_menu::WorldMenu>()
        .init_resource::<PartNames>()
        .init_resource::<items::ItemViews>()
        .init_resource::<DoubleTapSprint>()
        .init_resource::<SprintToggle>()
        .init_resource::<drill::Drill>()
        .init_resource::<combat::CombatFeel>()
        .init_resource::<animation::AnimationTuning>()
        .init_resource::<animation::AnimationPreview>()
        .init_resource::<combat::HitStop>()
        .init_resource::<combat::ViewKick>()
        .init_resource::<combat::TargetViews>()
        .init_resource::<combat::WeaponState>()
        .init_resource::<combat::HitReadout>()
        .init_resource::<hurt::HurtFlash>()
        .init_resource::<sound::SoundTuning>()
        .init_resource::<sound::Hearing>()
        .init_resource::<sound::SoundPreview>()
        .init_resource::<sound::SoundPicks>()
        .init_resource::<perf::PerfRecorder>()
        .init_resource::<perf::PerfTags>()
        .add_systems(
            Startup,
            (
                setup_scene,
                spawn_chunk_meshes,
                drill::setup_drill,
                combat::setup_media,
                combat::setup_weapon.after(setup_scene),
                combat::setup_readout,
                hurt::setup_health,
                sound::setup_audio,
                perf::setup_perf_text,
            ),
        )
        .add_systems(FixedUpdate, (tick_simulation, drill::drill_tick).chain())
        .add_systems(
            Update,
            (
                // Before grab_cursor, so the click that captures the mouse doesn't edit.
                (editing::edit_input, crafting::crafting_keys, grab_cursor).chain(),
                mouse_look,
                debug_keys,
                (dev::dev_keys, dev::show_dev_text).chain(),
                (double_tap_sprint, sprint_toggle),
                drill::drill_keys,
                editing::remesh_changed_chunks,
                // Strikes first: a broken target bursts from where its view was drawn.
                (combat::play_strikes, combat::play_slams, hurt::play_player_hurts, combat::update_hit_stop).chain(),
                (update_camera, sync_chunk_transforms, view::apply_render_distance, sound::play_sounds).chain(),
                editing::draw_outline,
                editing::update_ghost,
                editing::update_hidden_ghosts,
                items::sync_items,
                (combat::sync_targets, combat::update_chunks, combat::animate_weapon, combat::draw_hit_boxes, combat::draw_routes, combat::update_readout, hurt::update_health)
                    .chain(),
                editing::draw_region_guide,
                drill::draw_drill,
                update_hud,
                perf::record_perf,
                crafting::update_table_hint,
            )
                .chain(),
        )
        .add_systems(EguiPrimaryContextPass, (editing::hotbar, tuning::tuning_panel, crafting::crafting_menu, world_menu::world_menu).chain())
        .add_systems(Update, world_notes)
        .add_systems(Last, (save_on_exit, limit_frame_rate))
        .run();
}

/// Shows how saving or loading the world went, and turns the view to where a loaded world's
/// player was looking.
fn world_notes(mut world: ResMut<ClientWorld>, mut look: ResMut<Look>, mut readout: ResMut<combat::HitReadout>) {
    if let Some((yaw, pitch)) = world.take_look() {
        look.yaw = yaw;
        look.pitch = pitch;
    }
    for note in world.take_world_notes() {
        readout.show(match note {
            WorldNote::Saved { name } => format!("WORLD {name} SAVED"),
            WorldNote::Loaded { name, fresh: true, .. } => format!("NEW WORLD {name}"),
            WorldNote::Loaded { name, skipped, .. } if skipped.is_empty() => format!("WORLD {name} LOADED"),
            WorldNote::Loaded { name, skipped, .. } => {
                for s in &skipped {
                    warn!("loading world {name}: left out {s}");
                }
                format!("WORLD {name} LOADED, {} THINGS LEFT OUT (see the log)", skipped.len())
            }
            WorldNote::Failed(why) => {
                warn!("{why}");
                format!("WORLD NOT SAVED OR LOADED: {why}")
            }
        });
    }
}

/// Saves the world as the game closes.
fn save_on_exit(mut exits: MessageReader<AppExit>, mut link: ResMut<Link>, mut world: ResMut<ClientWorld>) {
    if exits.read().next().is_none() {
        return;
    }
    link.command(Command::SaveWorld, &mut world);
    for note in world.take_world_notes() {
        match note {
            WorldNote::Saved { name } => info!("world {name} saved"),
            WorldNote::Failed(why) => error!("saving the world on quitting: {why}"),
            WorldNote::Loaded { .. } => {}
        }
    }
}

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
    use_pressed: bool,
    alt_pressed: bool,
    drop_pressed: bool,
    interact_pressed: bool,
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
pub(crate) struct PlayerCamera;

#[derive(Component)]
struct Hud;

fn setup_scene(mut commands: Commands, mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>) {
    commands.spawn((Camera3d::default(), Transform::default(), PlayerCamera));
    commands.spawn((
        Text::new(""),
        Node { position_type: PositionType::Absolute, top: px(8), left: px(8), ..default() },
        Hud,
        dev::DevText,
    ));
    // A crosshair in the middle of the screen. The hotbar along the bottom is drawn with egui
    // (see `editing::hotbar`).
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
    // What aiming at a crafting table would do, just under the crosshair.
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            top: percent(53),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_child((Text::new(""), crafting::TableHint));
    set_grab(&mut cursor, true);
}

fn spawn_chunk_meshes(
    mut commands: Commands,
    world: Res<ClientWorld>,
    origin: Res<RenderOrigin>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SolidMaterial>>,
    mut images: ResMut<Assets<Image>>,
    ao: Res<mesher::AoTuning>,
) {
    let volumes = Volumes::build(world.looks(), world.blocks());
    let image = images.add(solid::volume_image(&volumes));
    let sheens = images.add(solid::sheen_image(&volumes));
    let solid = |tint: Color, alpha_mode: AlphaMode| SolidMaterial {
        tint: tint.into(),
        volumes: image.clone(),
        // Filled in by `shading` before the first frame is drawn.
        atmosphere: default(),
        sheens: sheens.clone(),
        alpha_mode,
    };
    let material = materials.add(solid(Color::WHITE, AlphaMode::Opaque));
    let ghost_material = |color: Color| solid(color, AlphaMode::Blend);
    let ghost = Ghost {
        entity: commands
            .spawn((
                Mesh3d(meshes.add(to_bevy_mesh(MeshData::default()))),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                Visibility::Hidden,
            ))
            .id(),
        allowed: materials.add(ghost_material(Color::srgba(1.0, 1.0, 1.0, 0.5))),
        refused: materials.add(ghost_material(Color::srgba(1.0, 0.2, 0.2, 0.6))),
        shown: None,
    };
    commands.insert_resource(ghost);
    let hidden = HiddenGhosts {
        entity: commands
            .spawn((
                Mesh3d(meshes.add(to_bevy_mesh(MeshData::default()))),
                MeshMaterial3d(materials.add(ghost_material(Color::srgba(1.0, 1.0, 1.0, 0.3)))),
                Transform::default(),
                Visibility::Hidden,
            ))
            .id(),
        around: None,
        dirty: false,
    };
    commands.insert_resource(hidden);
    let grid = world.universe().main();
    let mut entities = std::collections::HashMap::new();
    let mut quads = std::collections::HashMap::new();
    let start = Instant::now();
    let positions: Vec<ChunkPos> = grid.chunks().map(|(pos, _)| pos).collect();
    let meshed = mesher::mesh_chunks(grid, &positions, world.blocks(), &volumes, &ao);
    let last_meshing = Some((positions.len(), start.elapsed().as_secs_f32()));
    for (pos, data) in positions.into_iter().zip(meshed) {
        if data.is_empty() {
            continue;
        }
        quads.insert(pos, data.quad_count());
        entities.insert(pos, spawn_chunk(&mut commands, &mut meshes, material.clone(), pos, data, origin.cell));
    }
    info!("meshed {} chunks in {:.2} s", grid.chunks().count(), last_meshing.map_or(0.0, |(_, s)| s));
    commands.insert_resource(ChunkMeshes { material, volumes, entities, quads, last_meshing });
}

pub(crate) fn spawn_chunk(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<SolidMaterial>,
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
        .with_inserted_attribute(ATTRIBUTE_SOLID, data.solid)
        .with_inserted_attribute(ATTRIBUTE_EDGES, VertexAttributeValues::Uint32x4(data.edges))
        .with_inserted_attribute(ATTRIBUTE_FACE, VertexAttributeValues::Unorm16x4(data.face))
        .with_inserted_indices(Indices::U32(data.indices))
}

pub(crate) fn chunk_transform(pos: ChunkPos, origin: I64Vec3) -> Transform {
    Transform::from_translation(coords::render_translation(WorldPos::at_boxel(pos.origin()), origin))
}

#[allow(clippy::too_many_arguments)]
fn tick_simulation(
    mut world: ResMut<ClientWorld>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    controls: Res<Controls>,
    look: Res<Look>,
    palette: Res<Palette>,
    double_tap: Res<DoubleTapSprint>,
    sprint_toggle: Res<SprintToggle>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    mut toggles: ResMut<PendingToggles>,
    mut menu: ResMut<crafting::CraftingMenu>,
    mut names: ResMut<editing::PartNames>,
    world_menu: Res<world_menu::WorldMenu>,
    stop: Res<combat::HitStop>,
    mut link: ResMut<Link>,
) {
    // The simulation never waits for a hit-stop: only the view holds still. New presses wait for
    // it to end, as they did when it paused the simulation; held buttons and keys carry on.
    let frozen = stop.frozen();
    let take = |pressed: &mut bool| !frozen && std::mem::take(pressed);
    // With the crafting or world menu open, keys belong to it (a name is being typed, say), so
    // the body stands still.
    let free = menu.open.is_none() && !world_menu.open;
    let down = |key: KeyCode| free && keys.pressed(key);
    let axis = |pos: KeyCode, neg: KeyCode| down(pos) as i32 as f32 - down(neg) as i32 as f32;
    let playing = editing::playing(&cursor);
    let input = PlayerInput {
        movement: Vec2::new(axis(controls.right, controls.left), axis(controls.forward, controls.back)),
        vertical: axis(controls.jump, controls.down),
        yaw: look.yaw,
        pitch: look.pitch,
        jump_pressed: take(&mut toggles.jump_pressed),
        jump_held: down(controls.jump),
        sprint: free && (sprint_toggle.active || double_tap.active),
        crouch: down(controls.crouch),
        toggle_fly: take(&mut toggles.toggle_fly),
        use_pressed: take(&mut toggles.use_pressed),
        use_held: playing && controls.use_main.pressed(&keys, &mouse),
        alt_pressed: take(&mut toggles.alt_pressed),
        alt_held: playing && controls.use_alt.pressed(&keys, &mouse),
        tool: palette.tool(&world),
        place_block: palette.selected(&world),
        place_precise: palette.precise,
        drop_pressed: take(&mut toggles.drop_pressed),
        drop_held: playing && keys.pressed(controls.drop),
        drop_material: palette.material(&world),
        craft: if frozen { None } else { menu.take_pending() },
        blueprint: if frozen { None } else { menu.take_blueprint_edit() },
        discard: if frozen { None } else { menu.discard.take() },
        interact_pressed: take(&mut toggles.interact_pressed),
    };
    link.send(ClientMsg::Input(Box::new(input)), &mut world);
    if let Some(result) = world.take_craft() {
        menu.crafted(world.blocks(), world.blueprints(), &mut names, result);
    }
    if let Some(result) = world.take_blueprint_note() {
        menu.blueprint_edited(result);
    }
    if !frozen {
        world.show_latest();
    }
}

fn grab_cursor(
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    controls: Res<Controls>,
    mut panel: ResMut<TuningPanel>,
    mut menu: ResMut<crafting::CraftingMenu>,
    mut worlds: ResMut<world_menu::WorldMenu>,
) {
    // Names are typed into the world menu, so the tuning key is just a letter there.
    if keys.just_pressed(controls.tuning) && !worlds.open {
        panel.open = !panel.open;
        set_grab(&mut cursor, !panel.open && menu.open.is_none());
    }
    // While a panel or menu is open, clicks belong to it.
    if mouse.just_pressed(MouseButton::Left) && !panel.open && menu.open.is_none() && !worlds.open {
        set_grab(&mut cursor, true);
    }
    if keys.just_pressed(KeyCode::Escape) {
        // Escape closes the crafting menu, the tuning panel or the world menu and goes back to
        // playing; while playing it opens the world menu.
        let closed = menu.open.take().is_some() | std::mem::take(&mut panel.open) | std::mem::take(&mut worlds.open);
        if !closed {
            worlds.show();
        }
        set_grab(&mut cursor, closed);
    }
}

pub(crate) fn set_grab(cursor: &mut CursorOptions, grab: bool) {
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

fn double_tap_sprint(
    keys: Res<ButtonInput<KeyCode>>,
    controls: Res<Controls>,
    time: Res<Time>,
    mut double_tap: ResMut<DoubleTapSprint>,
) {
    double_tap.update(
        keys.just_pressed(controls.forward),
        keys.pressed(controls.forward),
        time.elapsed_secs_f64(),
        controls.double_tap_window,
    );
}

fn sprint_toggle(
    keys: Res<ButtonInput<KeyCode>>,
    controls: Res<Controls>,
    menu: Res<crafting::CraftingMenu>,
    worlds: Res<world_menu::WorldMenu>,
    mut toggle: ResMut<SprintToggle>,
) {
    // Shift typed into a menu (a capital in a name) isn't a sprint.
    let pressed = menu.open.is_none() && !worlds.open && keys.just_pressed(controls.sprint);
    toggle.update(pressed, keys.pressed(controls.forward));
}

#[allow(clippy::too_many_arguments)]
fn debug_keys(
    keys: Res<ButtonInput<KeyCode>>,
    controls: Res<Controls>,
    mut world: ResMut<ClientWorld>,
    mut link: ResMut<Link>,
    mut toggles: ResMut<PendingToggles>,
    mut cap: ResMut<FrameCap>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    mut waypoint: ResMut<WaypointCursor>,
    menu: Res<crafting::CraftingMenu>,
    worlds: Res<world_menu::WorldMenu>,
) {
    // Keys belong to the crafting or world menu while it's open.
    if menu.open.is_some() || worlds.open {
        return;
    }
    if keys.just_pressed(controls.fly) {
        toggles.toggle_fly = !toggles.toggle_fly;
    }
    if keys.just_pressed(controls.jump) {
        toggles.jump_pressed = true;
    }

    if keys.just_pressed(controls.respawn_targets) {
        link.command(Command::RespawnTargets, &mut world);
    }
    if keys.just_pressed(controls.reload_sounds) {
        link.command(Command::ReloadSounds, &mut world);
    }
    if keys.just_pressed(controls.save_world) {
        link.command(Command::SaveWorld, &mut world);
    }
    if keys.just_pressed(controls.reload_world) {
        link.command(Command::LoadWorld, &mut world);
    }
    if keys.just_pressed(controls.teleport) && !world.waypoints().is_empty() {
        waypoint.0 = (waypoint.0 + 1) % world.waypoints().len();
        let target = world.waypoints()[waypoint.0].1;
        link.command(Command::TeleportPlayer(target), &mut world);
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
    world: Res<ClientWorld>,
    look: Res<Look>,
    fixed: Res<Time<Fixed>>,
    time: Res<Time>,
    feel: Res<CameraFeel>,
    mut feel_state: ResMut<FeelState>,
    kick: Res<combat::ViewKick>,
    mut origin: ResMut<RenderOrigin>,
    mut camera: Single<(&mut Transform, &mut Projection), With<PlayerCamera>>,
) {
    let Some(player) = world.player() else {
        return;
    };
    let feel = &landing_feel(&feel, &world);
    let alpha = fixed.overstep_fraction();
    let prev = player.prev_pos();
    let step = prev.delta_to(player.pos());
    let feet = prev.translate(step * alpha);

    feel_state.update(
        feel,
        &Motion {
            horizontal_speed: player.velocity().truncate().length(),
            grounded: player.on_ground() && !player.flying(),
            landings: player.landings(),
            last_landing_speed: player.last_landing_speed(),
            stepped_total: player.stepped_total(),
            crouch_shift_total: player.crouch_shift_total(),
        },
        time.delta_secs(),
    );
    let eye_height = player.eye_height() + feel_state.vertical_offset(feel);
    let eye = feet.translate(ZVec3::new(0.0, 0.0, eye_height));

    let from_origin = (eye.cell() - origin.cell).abs().max_element();
    if from_origin > RECENTER_DISTANCE {
        origin.cell = eye.cell();
    }
    let (transform, projection) = &mut *camera;
    let limit = 89f32.to_radians();
    transform.rotation = coords::look_rotation(look.yaw, (look.pitch - feel_state.nod() + kick.pitch()).clamp(-limit, limit));
    transform.translation = coords::render_translation(eye, origin.cell) + transform.rotation * kick.shake_offset();
    if let Projection::Perspective(perspective) = &mut **projection {
        perspective.fov = feel_state.fov.to_radians();
    }
}

/// The camera feel, with landings bottoming out from the speed at which falls start to hurt
/// when there is fall damage, so the camera says when a fall hurt.
pub(crate) fn landing_feel(feel: &CameraFeel, world: &ClientWorld) -> CameraFeel {
    let hard_landing_speed = world.health_def().filter(|h| h.fall_damage > 0.0).map_or(feel.hard_landing_speed, |h| h.fall_speed);
    CameraFeel { hard_landing_speed, ..feel.clone() }
}

fn sync_chunk_transforms(origin: Res<RenderOrigin>, mut chunks: Query<(&ChunkMesh, &mut Transform)>) {
    if !origin.is_changed() {
        return;
    }
    for (chunk, mut transform) in &mut chunks {
        *transform = chunk_transform(chunk.0, origin.cell);
    }
}

#[allow(clippy::too_many_arguments)]
fn update_hud(
    world: Res<ClientWorld>,
    origin: Res<RenderOrigin>,
    cap: Res<FrameCap>,
    controls: Res<Controls>,
    feel: Res<CameraFeel>,
    time: Res<Time>,
    chunk_meshes: Option<Res<ChunkMeshes>>,
    (view, drawn): (Res<view::ViewTuning>, Res<view::ViewStats>),
    mut text: Single<&mut Text, With<Hud>>,
) {
    let Some(player) = world.player() else {
        return;
    };
    let cost = world_cost(&world, chunk_meshes.as_deref(), &view, &drawn);
    let pos = player.pos();
    let precise = pos.cell().as_dvec3() + pos.offset().as_dvec3();
    let fps = 1.0 / time.delta_secs().max(1e-6);
    text.0 = format!(
        "pos {:.2}, {:.2}, {:.2}  speed {:.1} m/s  last landing {:.1} m/s ({})\norigin {}, {}, {}\n{cost}\n{} | {:.0} fps | cap {} ({})\n{}",
        precise.x,
        precise.y,
        precise.z,
        player.velocity().truncate().length(),
        player.last_landing_speed(),
        if player.last_landing_speed() >= landing_feel(&feel, &world).hard_landing_speed { "hard" } else { "soft" },
        origin.cell.x,
        origin.cell.y,
        origin.cell.z,
        if player.flying() { "flying" } else { "walking" },
        fps,
        cap.mode.label(),
        label(controls.frame_cap),
        controls.hint(),
    );
}

/// What the world costs to hold and draw: terrain generation, meshing, chunks loaded and
/// drawn, triangles, memory, and the render and simulation distances.
fn world_cost(world: &ClientWorld, meshes: Option<&ChunkMeshes>, view: &view::ViewTuning, drawn: &view::ViewStats) -> String {
    let generation = match &world.defs().generation {
        Some(Ok(g)) => format!("terrain {} chunks in {:.2} s", g.chunks, g.seconds),
        Some(Err(e)) => format!("terrain failed: {e}"),
        None => "no terrain".into(),
    };
    let meshing = meshes.and_then(|m| m.last_meshing).map_or(String::new(), |(n, s)| format!(" | meshed {n} in {s:.2} s"));
    let meshed = meshes.map_or(0, |m| m.entities.len());
    let sim_distance = world
        .defs()
        .tuning
        .iter()
        .find(|s| s.name == "world")
        .and_then(|s| s.values.iter().find(|t| t.name == "sim_distance"))
        .map_or(String::from("?"), |t| format!("{:.0}", t.value));
    format!(
        "{generation}{meshing} | chunks {} loaded, {meshed} with faces, {} drawn | {:.2}M triangles | {} | render {:.0} m, sim {sim_distance} m",
        world.universe().main().chunks().count(),
        drawn.drawn,
        drawn.triangles as f64 / 1e6,
        memory_used().map_or("memory ?".into(), |b| format!("{:.0} MB", b as f64 / 1e6)),
        view.render_distance,
    )
}

/// The game's resident memory in bytes, where the system says (Linux).
fn memory_used() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096)
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
