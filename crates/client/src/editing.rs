//! Breaking and placing blocks on the client side: picking a tool or a material and its shape
//! from the palette, catching button presses between ticks, the outline on the targeted block,
//! the ghost of what placing would do, and remeshing changed chunks. The simulation decides
//! whether an edit happens; this only asks and shows.

use std::collections::HashMap;

use api::{ChunkPos, Vec3 as ZVec3, WorldPos};
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use sim::{BlockId, BlockRegistry, FACE_CENTER, MaterialId, Occupant, ToolId, face_region, occupant_boxes};

use crate::controls::Controls;
use crate::solid::SolidMaterial;
use crate::volumes::Volumes;
use crate::world::ClientWorld;
use crate::{ChunkMesh, Look, PendingToggles, RenderOrigin, coords, mesher};

/// Scroll distance, in pixels, that moves the palette one slot on touchpads and other
/// pixel-scrolling devices.
const PIXELS_PER_SLOT: f32 = 40.0;

/// What's in hand, and how blocks get placed. Every tool is on offer, then every material with
/// a placeable full block that's carried, then every part carried. A material places its full
/// block; shapes come from crafting, as parts. See [`entries`].
#[derive(Resource, Default)]
pub struct Palette {
    pub slot: usize,
    scrolled: f32,
    /// Precise placement instead of basic placement.
    pub precise: bool,
}

/// What a palette slot offers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Entry {
    Tool(ToolId),
    /// A material and its full block.
    Material(MaterialId, BlockId),
    /// A crafted part and its block.
    Part(MaterialId, BlockId),
    /// Any other placeable block, such as a mod's fixed shape or one made of no material.
    Block(BlockId),
}

/// The palette's slots: every tool, then each material with a placeable full block that's
/// carried, in the order materials were first named, then each part carried, in the order they
/// were first made, then any other placeable block. Without an inventory, placing is free and
/// nothing is carried, so every material is listed.
pub fn entries(world: &ClientWorld) -> Vec<Entry> {
    let blocks = world.blocks();
    let inventory = world.inventory();
    let carried = |m: MaterialId| world.inventory_def().is_none() || inventory.amount(m) >= blocks.unit(m);
    let mut out: Vec<Entry> = world.tool_ids().map(Entry::Tool).collect();
    let all = (0..blocks.material_count()).filter_map(|i| blocks.material_by_index(i));
    let (materials, parts): (Vec<_>, Vec<_>) = all.partition(|&m| blocks.part_of(m).is_none());
    let placeable = |b: BlockId| blocks.get(b).is_some_and(|def| def.placeable && def.is_full());
    let main: Vec<(MaterialId, BlockId)> =
        materials.into_iter().map(|m| (m, blocks.material_block(m))).filter(|&(_, b)| placeable(b)).collect();
    out.extend(main.iter().filter(|&&(m, _)| carried(m)).map(|&(m, b)| Entry::Material(m, b)));
    let held = parts.into_iter().filter(|&p| inventory.amount(p) >= blocks.unit(p));
    out.extend(held.map(|p| Entry::Part(p, blocks.material_block(p))));
    let part = |b: BlockId| blocks.drop(b).is_some_and(|d| blocks.part_of(d.material).is_some());
    let listed = |b: BlockId| main.iter().any(|&(_, m)| m == b) || part(b);
    out.extend(placeable_blocks(blocks).into_iter().filter(|&b| !listed(b)).map(Entry::Block));
    out
}

/// What a palette slot holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Held {
    Tool(ToolId),
    Block(BlockId),
    Nothing,
}

impl Palette {
    /// What's in the selected slot.
    pub fn held(&self, world: &ClientWorld) -> Held {
        match entries(world).get(self.slot) {
            Some(Entry::Tool(tool)) => Held::Tool(*tool),
            Some(Entry::Material(_, block) | Entry::Part(_, block) | Entry::Block(block)) => Held::Block(*block),
            None => Held::Nothing,
        }
    }

    /// The material or part in the selected slot, if it holds one: what the drop key throws.
    pub fn material(&self, world: &ClientWorld) -> Option<MaterialId> {
        match entries(world).get(self.slot) {
            Some(Entry::Material(m, _) | Entry::Part(m, _)) => Some(*m),
            _ => None,
        }
    }

    /// Whether the player sees hidden pieces: holding a phase material, as the simulation
    /// judges it.
    pub fn sees_hidden(&self, world: &ClientWorld) -> bool {
        world.blocks().get(self.selected(world)).is_some_and(|d| d.has(api::Property::Phase))
    }

    /// The occupant breaking would remove looking along `look`, seeing hidden pieces when
    /// holding a phase material.
    pub(crate) fn aimed(&self, world: &ClientWorld, look: &Look) -> Option<sim::Hit> {
        world.target_seeing(look.yaw, look.pitch, self.sees_hidden(world))
    }

    /// Selects the slot holding part `part`, if it's carried.
    pub fn select_part(&mut self, world: &ClientWorld, part: MaterialId) {
        if let Some(i) = entries(world).iter().position(|e| matches!(e, Entry::Part(p, _) if *p == part)) {
            self.slot = i;
        }
    }

    /// The selected block, or empty if a tool or nothing is selected.
    pub fn selected(&self, world: &ClientWorld) -> BlockId {
        match self.held(world) {
            Held::Block(block) => block,
            _ => BlockId::EMPTY,
        }
    }

    /// The selected tool, if one is.
    pub fn tool(&self, world: &ClientWorld) -> Option<ToolId> {
        match self.held(world) {
            Held::Tool(tool) => Some(tool),
            _ => None,
        }
    }
}

/// How many slots the palette has.
pub fn slot_count(world: &ClientWorld) -> usize {
    entries(world).len()
}

/// Every block a player may place, in registration order.
pub fn placeable_blocks(blocks: &BlockRegistry) -> Vec<BlockId> {
    blocks.iter().filter(|(_, def)| def.placeable).map(|(id, _)| id).collect()
}

/// The mesh entity for each chunk that has geometry, and the material and look volumes they
/// share.
#[derive(Resource)]
pub(crate) struct ChunkMeshes {
    pub material: Handle<SolidMaterial>,
    pub volumes: Volumes,
    pub entities: HashMap<ChunkPos, Entity>,
    /// How many quads each chunk's mesh has, for the readout.
    pub quads: HashMap<ChunkPos, usize>,
    /// How long the last big meshing took (the startup's, or a remesh of many chunks), and of
    /// how many chunks.
    pub last_meshing: Option<(usize, f32)>,
}

/// Whether the mouse is captured for playing. Clicks only edit while it is, so the click
/// that captures it doesn't also break a block.
pub(crate) fn playing(cursor: &CursorOptions) -> bool {
    cursor.grab_mode != CursorGrabMode::None
}

/// Latches use, drop and interact presses for the next tick, and picks the palette slot.
#[allow(clippy::too_many_arguments)]
pub(crate) fn edit_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    controls: Res<Controls>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    world: Res<ClientWorld>,
    mut palette: ResMut<Palette>,
    mut toggles: ResMut<PendingToggles>,
) {
    if !playing(&cursor) {
        return;
    }
    if controls.use_main.just_pressed(&keys, &mouse) {
        toggles.use_pressed = true;
    }
    if controls.use_alt.just_pressed(&keys, &mouse) {
        toggles.alt_pressed = true;
    }
    if keys.just_pressed(controls.drop) {
        toggles.drop_pressed = true;
    }
    if keys.just_pressed(controls.interact) {
        toggles.interact_pressed = true;
    }
    if controls.placement_mode.just_pressed(&keys, &mouse) {
        palette.precise = !palette.precise;
    }

    let count = slot_count(&world);
    if count == 0 {
        return;
    }
    // Running out of the last thing listed keeps the hand on the last slot there is.
    palette.slot = palette.slot.min(count - 1);
    if let Some(i) = controls.slots.iter().take(count).position(|&key| keys.just_pressed(key)) {
        palette.slot = i;
    }
    // Like Minecraft: scrolling down moves to the next slot.
    palette.scrolled += match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / PIXELS_PER_SLOT,
    };
    let steps = palette.scrolled.trunc();
    palette.scrolled -= steps;
    palette.slot = (palette.slot as i64 - steps as i64).rem_euclid(count as i64) as usize;
}

/// Outlines the boxes of the occupant that breaking would remove, unless a tool is in hand.
pub(crate) fn draw_outline(world: Res<ClientWorld>, look: Res<Look>, palette: Res<Palette>, origin: Res<RenderOrigin>, mut gizmos: Gizmos) {
    if palette.tool(&world).is_some() {
        return;
    }
    let Some(hit) = palette.aimed(&world, &look) else {
        return;
    };
    let Some(&occupant) = world.universe().main().cell(hit.pos).get(hit.occupant) else {
        return;
    };
    for b in occupant_boxes(occupant, world.blocks()) {
        let center = WorldPos::at_boxel(hit.pos).translate((b.min + b.max) * 0.5);
        let size = coords::to_render(Vec3::from_array((b.max - b.min).to_array())).abs();
        // Slightly larger than the box, so the lines aren't hidden by its own faces.
        let transform = Transform::from_translation(coords::render_translation(center, origin.cell))
            .with_scale(size + Vec3::splat(0.005));
        gizmos.cube(transform, Color::srgb(0.05, 0.05, 0.05));
    }
}

/// How far off the face the precise placement guide floats, so it isn't hidden by the face.
const GUIDE_LIFT: f32 = 0.003;

/// In precise placement, draws the five parts of the aimed-at face (the middle square and the
/// four triangles around it) and picks out the one the crosshair is in.
pub(crate) fn draw_region_guide(
    world: Res<ClientWorld>,
    look: Res<Look>,
    palette: Res<Palette>,
    origin: Res<RenderOrigin>,
    mut gizmos: Gizmos,
) {
    if !palette.precise || palette.tool(&world).is_some() {
        return;
    }
    let Some(hit) = palette.aimed(&world, &look) else {
        return;
    };
    if hit.normal == api::I64Vec3::ZERO {
        return;
    }
    let axis = hit.normal.abs().max_position();
    // A point on the face, from coordinates across it (0 to 1 along each of its two axes).
    let lift = hit.point[axis] + hit.normal.as_vec3()[axis] * GUIDE_LIFT;
    let at = |(u, v): (f32, f32)| {
        let mut local = ZVec3::ZERO;
        local[axis] = lift;
        local[(axis + 1) % 3] = u;
        local[(axis + 2) % 3] = v;
        coords::render_translation(WorldPos::at_boxel(hit.pos).translate(local), origin.cell)
    };
    let dim = Color::srgba(0.05, 0.05, 0.05, 0.6);
    let lo = 0.5 - FACE_CENTER;
    let hi = 0.5 + FACE_CENTER;
    let outer = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    let inner = [(lo, lo), (hi, lo), (hi, hi), (lo, hi)];
    gizmos.linestrip(outer.iter().chain(&outer[..1]).map(|&p| at(p)), dim);
    gizmos.linestrip(inner.iter().chain(&inner[..1]).map(|&p| at(p)), dim);
    for (o, i) in outer.iter().zip(&inner) {
        gizmos.line(at(*o), at(*i), dim);
    }
    let part = region_corners(face_region(&hit), axis);
    gizmos.linestrip(part.iter().chain(&part[..1]).map(|&p| at(p)), Color::srgb(1.0, 0.9, 0.2));
}

/// The corners of one part of a face, in coordinates across it (0 to 1 along the face's two
/// axes, the ones after `axis`), going around: the middle square for `None`, or the triangle,
/// cut off by the middle square, toward the edge in direction `edge`.
pub(crate) fn region_corners(region: Option<[i32; 3]>, axis: usize) -> Vec<(f32, f32)> {
    let (lo, hi) = (0.5 - FACE_CENTER, 0.5 + FACE_CENTER);
    let Some(edge) = region else {
        return vec![(lo, lo), (hi, lo), (hi, hi), (lo, hi)];
    };
    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
    // In (u, v) with u along `a`: which side, and whether the edge runs along u or v.
    let (along_u, positive) = if edge[a] != 0 { (true, edge[a] > 0) } else { (false, edge[b] > 0) };
    let (o, i) = if positive { (1.0, hi) } else { (0.0, lo) };
    let corners = [(o, 0.0), (o, 1.0), (i, hi), (i, lo)];
    corners.into_iter().map(|(x, y)| if along_u { (x, y) } else { (y, x) }).collect()
}

/// A see-through copy of what placing would put down, red when it isn't allowed.
#[derive(Resource)]
pub(crate) struct Ghost {
    pub entity: Entity,
    pub allowed: Handle<SolidMaterial>,
    pub refused: Handle<SolidMaterial>,
    /// The occupant its mesh was built for.
    pub shown: Option<Occupant>,
}

/// How much smaller than the real block the ghost is drawn, so its faces don't flicker against
/// neighbors they touch.
const GHOST_SCALE: f32 = 0.99;

/// Moves the ghost to where placing would go, rebuilding its mesh when the block or its turn
/// changes, or hides it when placing would do nothing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_ghost(
    world: Res<ClientWorld>,
    look: Res<Look>,
    palette: Res<Palette>,
    origin: Res<RenderOrigin>,
    chunk_meshes: Res<ChunkMeshes>,
    mut ghost: ResMut<Ghost>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut parts: Query<(&mut Mesh3d, &mut MeshMaterial3d<SolidMaterial>, &mut Transform, &mut Visibility)>,
) {
    let Ok((mut mesh, mut material, mut transform, mut visibility)) = parts.get_mut(ghost.entity) else {
        return;
    };
    let blocks = world.blocks();
    let place = palette
        .aimed(&world, &look)
        .and_then(|hit| world.placement(&hit, palette.selected(&world), look.yaw, look.pitch, palette.precise));
    let Some(place) = place else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    visibility.set_if_neq(Visibility::Visible);
    // A bundle's first piece brings the rest, so it stands for the whole ghost.
    if ghost.shown != Some(place.occupant) {
        let mut data = mesher::MeshData::default();
        let (volumes, off) = (&chunk_meshes.volumes, &mesher::AoTuning::OFF);
        for occupant in place.occupants() {
            // Built around the boxel's center, so scaling shrinks it in place.
            let boxes = mesher::alone(occupant, blocks);
            let around = || boxes.as_slice();
            mesher::mesh_occupant(&mut data, occupant, [-0.5; 3], blocks, volumes, |_| false, &around, off, &mut Vec::new());
        }
        mesh.0 = meshes.add(crate::to_bevy_mesh(data));
        ghost.shown = Some(place.occupant);
    }
    let wanted = if place.allowed { &ghost.allowed } else { &ghost.refused };
    if material.0 != *wanted {
        material.0 = wanted.clone();
    }
    let center = WorldPos::at_boxel(place.pos).translate(ZVec3::splat(0.5));
    *transform = Transform::from_translation(coords::render_translation(center, origin.cell))
        .with_scale(Vec3::splat(GHOST_SCALE));
}

/// See-through copies of the hidden pieces near the player, shown while they hold a phase
/// material, so what's hidden can be found, aimed at and broken.
#[derive(Resource)]
pub(crate) struct HiddenGhosts {
    pub entity: Entity,
    /// The chunk the mesh was built around, while it's shown.
    pub around: Option<ChunkPos>,
    /// Blocks changed since it was built.
    pub dirty: bool,
}

/// How far from the player hidden pieces are shown, in boxels.
const HIDDEN_RANGE: i64 = 24;

/// Shows the hidden pieces near the player while they hold a phase material, rebuilding the
/// mesh when blocks change or the player reaches another chunk.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_hidden_ghosts(
    world: Res<ClientWorld>,
    palette: Res<Palette>,
    origin: Res<RenderOrigin>,
    chunk_meshes: Res<ChunkMeshes>,
    mut ghosts: ResMut<HiddenGhosts>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut parts: Query<(&mut Mesh3d, &mut Transform, &mut Visibility)>,
) {
    let Ok((mut mesh, mut transform, mut visibility)) = parts.get_mut(ghosts.entity) else {
        return;
    };
    let (Some(player), true) = (world.player(), palette.sees_hidden(&world)) else {
        visibility.set_if_neq(Visibility::Hidden);
        ghosts.around = None;
        return;
    };
    let center = player.pos().boxel();
    let chunk = center.chunk();
    if ghosts.around != Some(chunk) || ghosts.dirty {
        let (grid, blocks) = (world.universe().main(), world.blocks());
        let anchor = chunk.origin();
        let mut data = mesher::MeshData::default();
        let r = HIDDEN_RANGE;
        for z in -r..=r {
            for y in -r..=r {
                for x in -r..=r {
                    let pos = center.offset(api::I64Vec3::new(x, y, z));
                    for &o in grid.cell(pos).iter().filter(|o| blocks.is_hidden(o.block)) {
                        let boxes = mesher::alone(o, blocks);
                        let around = || boxes.as_slice();
                        let at = (pos.0 - anchor.0).as_vec3().to_array();
                        let (volumes, off) = (&chunk_meshes.volumes, &mesher::AoTuning::OFF);
                        mesher::mesh_occupant(&mut data, o, at, blocks, volumes, |_| false, &around, off, &mut Vec::new());
                    }
                }
            }
        }
        mesh.0 = meshes.add(crate::to_bevy_mesh(data));
        ghosts.around = Some(chunk);
        ghosts.dirty = false;
    }
    visibility.set_if_neq(Visibility::Visible);
    *transform = crate::chunk_transform(chunk, origin.cell);
}

/// Remeshing at least this many chunks at once is timed for the readout.
const BIG_REMESH: usize = 64;

/// Rebuilds the meshes of chunks the simulation changed, and of every chunk when the ambient
/// occlusion tuning changes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn remesh_changed_chunks(
    mut commands: Commands,
    mut world: ResMut<ClientWorld>,
    origin: Res<RenderOrigin>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut chunk_meshes: ResMut<ChunkMeshes>,
    mut handles: Query<&mut Mesh3d, With<ChunkMesh>>,
    ao: Res<mesher::AoTuning>,
    mut hidden: ResMut<HiddenGhosts>,
) {
    // Crafting registers blocks while playing; their looks must be known before they're drawn.
    chunk_meshes.volumes.sync(world.blocks());
    let mut changed = world.take_changed_chunks();
    let grid = world.universe().main();
    if ao.is_changed() && !ao.is_added() {
        changed = grid.chunks().map(|(pos, _)| pos).collect();
    }
    hidden.dirty |= !changed.is_empty();
    let start = std::time::Instant::now();
    let meshed = mesher::mesh_chunks(grid, &changed, world.blocks(), &chunk_meshes.volumes, &ao);
    if changed.len() >= BIG_REMESH {
        chunk_meshes.last_meshing = Some((changed.len(), start.elapsed().as_secs_f32()));
    }
    for (pos, data) in changed.into_iter().zip(meshed) {
        if data.is_empty() {
            chunk_meshes.quads.remove(&pos);
        } else {
            chunk_meshes.quads.insert(pos, data.quad_count());
        }
        match chunk_meshes.entities.get(&pos).copied() {
            Some(entity) if data.is_empty() => {
                commands.entity(entity).despawn();
                chunk_meshes.entities.remove(&pos);
            }
            Some(entity) => {
                if let Ok(mut mesh) = handles.get_mut(entity) {
                    mesh.0 = meshes.add(crate::to_bevy_mesh(data));
                }
            }
            None if data.is_empty() => {}
            None => {
                let material = chunk_meshes.material.clone();
                let entity = crate::spawn_chunk(&mut commands, &mut meshes, material, pos, data, origin.cell);
                chunk_meshes.entities.insert(pos, entity);
            }
        }
    }
}

/// A name without its namespace.
pub(crate) fn short(name: &str) -> &str {
    name.split_once(':').map_or(name, |(_, path)| path)
}

/// A part's name for people: its material, how much of a block it fills, and the first four
/// digits of its shape's hash to tell parts of one size apart, like `wood ½ #3fa2`.
pub(crate) fn part_name(blocks: &BlockRegistry, part: MaterialId) -> String {
    let material = match blocks.pieces(part) {
        [] => blocks.part_of(part).map_or("?", |m| short(blocks.material_name(m))).to_owned(),
        pieces => {
            let names: Vec<&str> =
                pieces.iter().filter_map(|&b| blocks.made_of(b)).map(|m| short(blocks.material_name(m))).collect();
            names.join("+")
        }
    };
    let name = blocks.material_name(part);
    let hash = name.rsplit_once("_s").or_else(|| name.rsplit_once("bundle_")).map_or("", |(_, hash)| hash);
    let tag = &hash[..hash.len().min(4)];
    format!("{material} {} #{tag}", fraction(blocks.unit(part)))
}

/// A volume as a fraction of a boxel, in the simplest texels, sixty-fourths and so on it
/// makes: `½`, `¾`, `3/16`.
pub(crate) fn fraction(volume: u32) -> String {
    let full = sim::FULL_VOLUME;
    let gcd = |mut a: u32, mut b: u32| {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    };
    let g = gcd(volume, full).max(1);
    match (volume / g, full / g) {
        (1, 2) => "½".to_owned(),
        (1, 4) => "¼".to_owned(),
        (3, 4) => "¾".to_owned(),
        (n, 1) => n.to_string(),
        (n, d) => format!("{n}/{d}"),
    }
}

/// Names players give parts, by the part's name, for this session. A part without one goes by
/// [`part_name`].
#[derive(Resource, Default)]
pub struct PartNames(HashMap<String, String>);

impl PartNames {
    /// What a material or part is called: a part's given name or its automatic one, or a
    /// material's name without its namespace.
    pub fn label(&self, blocks: &BlockRegistry, id: MaterialId) -> String {
        match blocks.part_of(id) {
            Some(_) => self.0.get(blocks.material_name(id)).cloned().unwrap_or_else(|| part_name(blocks, id)),
            None => short(blocks.material_name(id)).to_owned(),
        }
    }

    /// The name given to `part`, if any.
    pub fn given(&self, blocks: &BlockRegistry, part: MaterialId) -> Option<&str> {
        self.0.get(blocks.material_name(part)).map(String::as_str)
    }

    /// Names `part`; a blank name goes back to the automatic one.
    pub fn set(&mut self, blocks: &BlockRegistry, part: MaterialId, name: &str) {
        let key = blocks.material_name(part).to_owned();
        match name.trim() {
            "" => self.0.remove(&key),
            name => self.0.insert(key, name.to_owned()),
        };
    }
}

/// How full the inventory is, for the line above the hotbar. Without a limit, materials that
/// never run out are left out of the count.
pub(crate) fn carried_line(world: &ClientWorld) -> String {
    let inventory = world.inventory();
    let amount = crate::items::amount_label;
    match world.capacity() {
        Some(capacity) => {
            let full = if inventory.room(Some(capacity)) == 0 { "  FULL" } else { "" };
            format!("carrying {} / {}{full}", amount(inventory.total()), amount(capacity))
        }
        None => {
            let counted = inventory.iter().filter(|&(m, _)| !is_endless(world, m)).map(|(_, a)| a).sum();
            format!("carrying {}, no limit", amount(counted))
        }
    }
}

/// Whether material `m` never runs out.
fn is_endless(world: &ClientWorld, m: MaterialId) -> bool {
    let name = world.blocks().material_name(m);
    world.inventory_def().is_some_and(|d| d.endless.iter().any(|e| e == name))
}

/// How much of material `m` is carried, as a label: `∞` for one that never runs out.
pub(crate) fn carried_label(world: &ClientWorld, m: MaterialId, amount: u32) -> String {
    match is_endless(world, m) {
        true => "∞".to_owned(),
        false => crate::items::amount_label(amount),
    }
}

/// A slot's name on the hotbar, and how much of it is carried, if that's counted.
pub(crate) fn slot_label(world: &ClientWorld, names: &PartNames, entry: &Entry) -> (String, Option<String>) {
    let blocks = world.blocks();
    let inventory = world.inventory();
    let counted = world.inventory_def().is_some();
    match entry {
        Entry::Tool(tool) => (short(&world.tool(*tool).name).to_owned(), None),
        Entry::Material(m, _) => (names.label(blocks, *m), counted.then(|| carried_label(world, *m, inventory.amount(*m)))),
        Entry::Part(p, _) => (names.label(blocks, *p), Some(format!("×{}", inventory.amount(*p) / blocks.unit(*p)))),
        Entry::Block(block) => (blocks.get(*block).map_or("?".to_owned(), |def| short(&def.name).to_owned()), None),
    }
}

/// What's held and how it's placed, for the line under the hotbar.
pub(crate) fn held_line(world: &ClientWorld, palette: &Palette, names: &PartNames, controls: &Controls) -> String {
    let blocks = world.blocks();
    let mode = if palette.precise {
        "precise placement: the middle of a face lays it flat, an edge stands it there"
    } else {
        "basic placement"
    };
    let mode = format!("{mode} ({} switches)", controls.placement_mode.label());
    match (palette.material(world), palette.held(world)) {
        (Some(m), Held::Block(_)) => {
            let what = match blocks.part_of(m) {
                Some(_) => format!("{}, a crafted shape", names.label(blocks, m)),
                None => format!("{} block, shapes are crafted at a table", short(blocks.material_name(m))),
            };
            let throw = if world.inventory_def().is_some() {
                format!(", {} throws", crate::controls::label(controls.drop))
            } else {
                String::new()
            };
            format!("{what}{throw}  ·  {mode}")
        }
        (_, Held::Block(_)) => mode,
        _ => String::new(),
    }
}

/// The size of a hotbar slot's picture.
const SLOT_ICON: f32 = 40.0;

/// The hotbar along the bottom of the screen, above the health bar: how full the inventory
/// is, a slot for each tool, material and part with its picture, number and amount, the
/// selected one picked out, and what's held with the keys that change it. It only shows; keys
/// and the wheel pick slots.
pub(crate) fn hotbar(
    mut contexts: bevy_egui::EguiContexts,
    world: Res<ClientWorld>,
    palette: Res<Palette>,
    names: Res<PartNames>,
    controls: Res<Controls>,
) -> Result {
    draw_hotbar(contexts.ctx_mut()?, &world, &palette, &names, &controls);
    Ok(())
}

/// Draws the hotbar (see [`hotbar`]) in `ctx`.
fn draw_hotbar(ctx: &bevy_egui::egui::Context, world: &ClientWorld, palette: &Palette, names: &PartNames, controls: &Controls) {
    use bevy_egui::egui;
    let blocks = world.blocks();
    let shadow = egui::Color32::from_black_alpha(150);
    egui::Area::new(egui::Id::new("hotbar"))
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -26.0])
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new().fill(shadow).corner_radius(6.0).inner_margin(6.0).show(ui, |ui| {
                // Centering spreads out to the width egui remembers from the last frame, so
                // the bar could grow but never shrink: give it this frame's width instead.
                let line = |text: String| {
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    ui.painter().layout_no_wrap(text, font, egui::Color32::WHITE)
                };
                let carried = line(carried_line(world));
                let held = Some(held_line(world, palette, names, controls)).filter(|h| !h.is_empty()).map(line);
                let slots = entries(world);
                let size = egui::vec2(SLOT_ICON + 16.0, SLOT_ICON + 30.0);
                let gap = ui.spacing().item_spacing.x;
                let row = slots.len() as f32 * (size.x + gap) - gap;
                let texts = held.iter().chain([&carried]).map(|g| g.size().x);
                ui.set_width(texts.fold(row, f32::max));
                ui.vertical_centered(|ui| {
                    ui.label(carried);
                    ui.horizontal(|ui| {
                        for (i, entry) in slots.iter().enumerate() {
                            let (name, amount) = slot_label(world, names, entry);
                            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                            let selected = i == palette.slot;
                            let painter = ui.painter();
                            let fill = if selected { egui::Color32::from_gray(90) } else { egui::Color32::from_gray(35) };
                            painter.rect_filled(rect, 4.0, fill);
                            if selected {
                                let gold = egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(255, 210, 60));
                                painter.rect_stroke(rect, 4.0, gold, egui::StrokeKind::Inside);
                            }
                            let icon = egui::Rect::from_center_size(
                                egui::pos2(rect.center().x, rect.min.y + 4.0 + SLOT_ICON / 2.0),
                                egui::vec2(SLOT_ICON, SLOT_ICON),
                            );
                            match entry {
                                Entry::Tool(_) => {
                                    let font = egui::FontId::proportional(13.0);
                                    painter.text(icon.center(), egui::Align2::CENTER_CENTER, "tool", font, egui::Color32::LIGHT_GRAY);
                                }
                                Entry::Material(m, _) | Entry::Part(m, _) => crate::crafting::paint_thing(painter, icon, blocks, *m),
                                Entry::Block(b) => {
                                    let c = blocks.get(*b).map_or([200; 3], |d| d.color);
                                    painter.rect_filled(icon, 2.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
                                }
                            }
                            let small = egui::FontId::proportional(11.0);
                            let white = egui::Color32::WHITE;
                            painter.text(rect.min + egui::vec2(4.0, 2.0), egui::Align2::LEFT_TOP, (i + 1).to_string(), small.clone(), white);
                            if let Some(amount) = amount {
                                let at = icon.right_bottom() + egui::vec2(2.0, 2.0);
                                painter.text(at, egui::Align2::RIGHT_BOTTOM, amount, small.clone(), white);
                            }
                            let label: String = name.chars().take(9).collect();
                            let at = egui::pos2(rect.center().x, rect.max.y - 3.0);
                            painter.text(at, egui::Align2::CENTER_BOTTOM, label, small, egui::Color32::LIGHT_GRAY);
                        }
                    });
                    if let Some(held) = held {
                        ui.label(held);
                    }
                });
            });
        });
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::testing::{base_sim, connect, step};
    use api::{BlockDef, I64Vec3};
    use protocol::Command;
    use sim::PlayerInput;

    #[test]
    fn the_palette_lists_only_placeable_blocks() {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(BlockDef::new("test:stone", true, [0, 0, 0])).unwrap();
        blocks.register(BlockDef::new("test:bedrock", true, [0, 0, 0]).with_placeable(false)).unwrap();
        let dirt = blocks.register(BlockDef::new("test:dirt", true, [0, 0, 0])).unwrap();
        assert_eq!(placeable_blocks(&blocks), [stone, dirt]);
    }

    #[test]
    fn the_palette_holds_the_tools_then_what_is_carried() {
        // Dirt and a part lie at the player's feet, to be picked up once the game runs.
        let mut sim = base_sim();
        let wood = sim.blocks().material(base::WOOD).unwrap();
        let dirt = sim.blocks().material(base::DIRT).unwrap();
        let slab = sim.shaped_block(wood, &base::shapes::slab()).unwrap();
        let part = sim.blocks().drop(slab).unwrap().material;
        sim.spawn_player().unwrap();
        let feet = sim.player().unwrap().pos();
        sim.spawn_item(part, sim::FULL_VOLUME / 2, feet);
        sim.spawn_item(dirt, sim::FULL_VOLUME, feet);
        let (mut link, mut sim) = connect(sim);
        let club = sim.tool_ids().next().expect("base registers a tool");
        assert_eq!(sim.tool(club).name, base::CLUB);
        let blocks = sim.blocks();
        let material = |name| Entry::Material(blocks.material(name).unwrap(), blocks.id(name).unwrap());
        let (brick, brick_block) = (blocks.material(base::BRICK).unwrap(), blocks.id(base::BRICK).unwrap());
        // The base game starts with brick, wood, the metals, the phase materials and the
        // experimental ones; nothing else is carried, so nothing else is on offer, not even the
        // test area's crafted shapes.
        let carried = [
            base::BRICK, base::WOOD, base::BRASS, base::IRON, base::COPPER, base::PHASITE, base::ANTIPHASITE, base::LUMITE,
            base::BLUE_CRYSTAL, base::GREEN_CRYSTAL,
        ];
        let start = carried.map(material);
        assert_eq!(entries(&sim), [&[Entry::Tool(club)][..], &start].concat());

        let at = |slot| Palette { slot, ..Default::default() };
        assert_eq!(at(0).held(&sim), Held::Tool(club));
        assert_eq!((at(0).tool(&sim), at(0).selected(&sim)), (Some(club), BlockId::EMPTY), "a tool places nothing");
        assert_eq!(at(0).material(&sim), None, "nor throws");
        assert_eq!((at(1).tool(&sim), at(1).selected(&sim), at(1).material(&sim)), (None, brick_block, Some(brick)));
        assert_eq!(at(slot_count(&sim)).held(&sim), Held::Nothing);

        // Picking up dirt and a part puts them on offer: materials in the order they were
        // first named, then parts.
        for _ in 0..5 {
            step(&mut link, &mut sim, &PlayerInput::default());
        }
        let blocks = sim.blocks();
        let material = |name| Entry::Material(blocks.material(name).unwrap(), blocks.id(name).unwrap());
        let mut slots = vec![Entry::Tool(club), material(base::DIRT)];
        slots.extend(carried.map(material));
        slots.push(Entry::Part(part, slab));
        assert_eq!(entries(&sim), slots);
        let mut palette = at(0);
        palette.select_part(&sim, part);
        assert_eq!((palette.slot, palette.selected(&sim), palette.material(&sim)), (carried.len() + 2, slab, Some(part)));

        // Throwing the part away takes it off the palette.
        step(&mut link, &mut sim, &PlayerInput { discard: Some(part), ..Default::default() });
        assert_eq!(entries(&sim).len(), carried.len() + 2);
    }

    #[test]
    fn the_hotbar_shows_amounts_the_fill_and_what_is_held() {
        let (_, sim) = connect(base_sim());
        let names = PartNames::default();
        let palette = Palette { slot: 1, ..Default::default() };
        assert_eq!(carried_line(&sim), "carrying 36, no limit", "brick, wood and iron; endless ones aren't counted");
        let slots = entries(&sim);
        assert_eq!(slot_label(&sim, &names, &slots[0]), ("club".into(), None));
        assert_eq!(slot_label(&sim, &names, &slots[1]), ("brick".into(), Some("16".into())));
        assert_eq!(slot_label(&sim, &names, &slots[5]), ("copper".into(), Some("∞".into())), "copper never runs out");
        assert_eq!(slot_label(&sim, &names, &slots[4]), ("iron".into(), Some("4".into())), "iron does");
        let held = held_line(&sim, &palette, &names, &Controls::default());
        assert!(held.starts_with("brick block, shapes are crafted at a table, Q throws"), "{held}");
        assert_eq!((fraction(sim::FULL_VOLUME / 2), fraction(sim::FULL_VOLUME * 3 / 16)), ("½".into(), "3/16".into()));
        assert_eq!(fraction(sim::FULL_VOLUME * 2), "2");
    }

    #[test]
    fn the_hotbar_shrinks_when_slots_go() {
        // Dirt and a part lie at the player's feet, to be picked up once the game runs.
        let mut sim = base_sim();
        let wood = sim.blocks().material(base::WOOD).unwrap();
        let dirt = sim.blocks().material(base::DIRT).unwrap();
        let slab = sim.shaped_block(wood, &base::shapes::slab()).unwrap();
        let part = sim.blocks().drop(slab).unwrap().material;
        sim.spawn_player().unwrap();
        let feet = sim.player().unwrap().pos();
        sim.spawn_item(part, sim::FULL_VOLUME / 2, feet);
        sim.spawn_item(dirt, sim::FULL_VOLUME, feet);
        let (mut link, mut world) = connect(sim);
        let ctx = bevy_egui::egui::Context::default();
        // The club is held, so there's no line under the slots and the row sets the width.
        let width = |world: &ClientWorld| {
            let input = || bevy_egui::egui::RawInput {
                screen_rect: Some(bevy_egui::egui::Rect::from_min_size(Default::default(), bevy_egui::egui::vec2(1600.0, 1000.0))),
                ..Default::default()
            };
            let (palette, names) = (Palette::default(), PartNames::default());
            for _ in 0..4 {
                let _ = ctx.run(input(), |ctx| draw_hotbar(ctx, world, &palette, &names, &Controls::default()));
            }
            ctx.memory(|m| m.area_rect(bevy_egui::egui::Id::new("hotbar"))).expect("the hotbar is drawn").width()
        };
        let before = width(&world);
        for _ in 0..5 {
            step(&mut link, &mut world, &PlayerInput::default());
        }
        let wider = width(&world);
        assert!(wider > before + 100.0, "picking up two things widens it: {before} to {wider}");
        step(&mut link, &mut world, &PlayerInput { discard: Some(part), ..Default::default() });
        step(&mut link, &mut world, &PlayerInput { discard: Some(dirt), ..Default::default() });
        let after = width(&world);
        assert!((after - before).abs() < 1.0, "throwing them away shrinks it back: {before}, {wider}, then {after}");
    }

    #[test]
    fn parts_go_by_the_names_players_give_them() {
        let (_, sim) = connect(base_sim());
        let blocks = sim.blocks();
        let stone = blocks.material(base::STONE).unwrap();
        let stairs = blocks.drop(blocks.find_shaped(stone, &base::shapes::stairs()).unwrap()).unwrap().material;
        let mut names = PartNames::default();
        let auto = names.label(blocks, stairs);
        assert!(auto.starts_with("stone ¾ #") && auto.len() == "stone ¾ #".len() + 4, "{auto}");
        assert_eq!(names.given(blocks, stairs), None);
        names.set(blocks, stairs, "  steps ");
        assert_eq!((names.label(blocks, stairs), names.given(blocks, stairs)), ("steps".into(), Some("steps")));
        names.set(blocks, stairs, " ");
        assert_eq!(names.label(blocks, stairs), auto, "a blank name goes back to the automatic one");
        assert_eq!(names.label(blocks, stone), "stone", "materials keep their own names");
    }

    #[test]
    fn the_region_guide_marks_the_part_placement_uses() {
        // For points all over each kind of face, the part drawn holds the point.
        for normal in [I64Vec3::X, I64Vec3::NEG_Y, I64Vec3::Z, I64Vec3::NEG_Z] {
            let axis = normal.abs().max_position();
            for i in 0..20 {
                for j in 0..20 {
                    let (u, v) = ((i as f32 + 0.5) / 20.0, (j as f32 + 0.5) / 20.0);
                    let mut point = ZVec3::splat(0.5) + normal.as_vec3() * 0.5;
                    point[(axis + 1) % 3] = u;
                    point[(axis + 2) % 3] = v;
                    let hit = sim::Hit { pos: api::BoxelPos::new(0, 0, 0), occupant: 0, normal, distance: 1.0, point };
                    let part = region_corners(face_region(&hit), axis);
                    assert!(inside(&part, (u, v)), "{normal} at {u}, {v}: {part:?}");
                }
            }
        }
    }

    /// Whether `p` is inside the convex polygon `corners`, going around either way.
    fn inside(corners: &[(f32, f32)], p: (f32, f32)) -> bool {
        let sides: Vec<f32> = (0..corners.len())
            .map(|k| {
                let (a, b) = (corners[k], corners[(k + 1) % corners.len()]);
                (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
            })
            .collect();
        sides.iter().all(|&s| s >= -1e-6) || sides.iter().all(|&s| s <= 1e-6)
    }

    #[test]
    fn breaking_one_occupant_remeshes_and_leaves_the_other() {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        let (mut link, mut sim) = connect(sim);
        // Stand east of a slab with a fence post through it and aim down at the post's top.
        let blocks = sim.blocks();
        let shaped = |m: &str, shape| blocks.find_shaped(blocks.material(m).unwrap(), &shape).unwrap();
        let (fence, slab) = (shaped(base::WOOD, base::shapes::post()), shaped(base::BRICK, base::shapes::slab()));
        let cell = api::BoxelPos::new(-9, -4, 0);
        assert_eq!(sim.universe().main().cell(cell).len(), 2);
        let to = WorldPos::new(I64Vec3::new(-8, -4, 0), ZVec3::new(0.5, 0.5, 0.0));
        link.command(Command::TeleportPlayer(to), &mut sim);
        step(&mut link, &mut sim, &PlayerInput::default());
        sim.take_changed_chunks();
        let eye = sim.player().unwrap().eye();
        let aim = eye.delta_to(WorldPos::new(cell.0, ZVec3::new(0.5, 0.5, 1.0)));
        let (yaw, pitch) = (aim.y.atan2(aim.x), aim.z.atan2(aim.truncate().length()));
        let hit = sim.target(yaw, pitch).unwrap();
        assert_eq!(sim.universe().main().cell(hit.pos)[hit.occupant].block, fence);

        let volumes = Volumes::build(sim.looks(), sim.blocks());
        let chunk = ChunkPos::new(-1, -1, 0);
        let before = mesher::mesh_chunk(sim.universe().main(), chunk, sim.blocks(), &volumes, &mesher::AoTuning::default()).quad_count();
        step(&mut link, &mut sim, &PlayerInput { yaw, pitch, use_pressed: true, use_held: true, ..Default::default() });
        let left: Vec<_> = sim.universe().main().cell(cell).iter().map(|o| o.block).collect();
        assert_eq!(left, [slab]);
        assert!(sim.take_changed_chunks().contains(&chunk));
        let after = mesher::mesh_chunk(sim.universe().main(), chunk, sim.blocks(), &volumes, &mesher::AoTuning::default()).quad_count();
        assert_eq!(after, before - 5, "the post's faces are gone; its bottom was already hidden by the floor");
    }

    #[test]
    fn breaking_a_block_remeshes_its_chunk() {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        sim.teleport_player(WorldPos::new(I64Vec3::new(10, 10, 0), ZVec3::new(0.5, 0.5, 0.0)));
        let (mut link, mut sim) = connect(sim);
        let below = ChunkPos::new(0, 0, -1);
        let volumes = Volumes::build(sim.looks(), sim.blocks());
        let before = mesher::mesh_chunk(sim.universe().main(), below, sim.blocks(), &volumes, &mesher::AoTuning::default()).quad_count();

        let down = -89f32.to_radians();
        step(&mut link, &mut sim, &PlayerInput { pitch: down, use_pressed: true, use_held: true, ..Default::default() });
        let changed = sim.take_changed_chunks();
        assert!(changed.contains(&below));
        // The floor's top layer is the top of this chunk, so the chunk above is told too.
        assert!(changed.contains(&ChunkPos::new(0, 0, 0)));

        // One top face gone; four side faces and the floor of the hole appear.
        let after = mesher::mesh_chunk(sim.universe().main(), below, sim.blocks(), &volumes, &mesher::AoTuning::default()).quad_count();
        assert_eq!(after, before + 4);
    }
}
#[cfg(test)]
mod remesh_timing {
    use super::*;
    use crate::volumes::Volumes;
    use crate::world::testing::{base_sim, connect};

    /// Times meshing every chunk of the base game's test area. Run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn time_a_full_remesh() {
        let (_, sim) = connect(base_sim());
        let volumes = Volumes::build(sim.looks(), sim.blocks());
        let ao = mesher::AoTuning::default();
        let grid = sim.universe().main();
        let chunks: Vec<_> = grid.chunks().map(|(pos, _)| pos).collect();
        let mut best = std::time::Duration::MAX;
        let mut quads = 0;
        for _ in 0..5 {
            let start = std::time::Instant::now();
            quads = chunks.iter().map(|&pos| mesher::mesh_chunk(grid, pos, sim.blocks(), &volumes, &ao).quad_count()).sum();
            best = best.min(start.elapsed());
        }
        println!("{} chunks, {quads} quads, best of 5: {best:?}", chunks.len());
    }

    /// Times meshing the base game's whole world with its terrain generated, on every core, as
    /// at startup. Run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn time_meshing_the_terrain() {
        let mut sim = base_sim();
        sim.spawn_player().unwrap();
        let generated = sim.generate_terrain().unwrap();
        let (_, world) = connect(sim);
        let volumes = Volumes::build(world.looks(), world.blocks());
        let ao = mesher::AoTuning::default();
        let grid = world.universe().main();
        let chunks: Vec<_> = grid.chunks().map(|(pos, _)| pos).collect();
        let start = std::time::Instant::now();
        let meshes = mesher::mesh_chunks(grid, &chunks, world.blocks(), &volumes, &ao);
        let took = start.elapsed();
        let with_faces = meshes.iter().filter(|m| !m.is_empty()).count();
        let quads: usize = meshes.iter().map(|m| m.quad_count()).sum();
        println!(
            "generated {} chunks in {:.2} s; meshed {} ({with_faces} with faces, {quads} quads) in {took:?}",
            generated.chunks,
            generated.seconds,
            chunks.len()
        );
    }
}
