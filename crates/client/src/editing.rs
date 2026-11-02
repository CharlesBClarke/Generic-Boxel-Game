//! Breaking and placing blocks on the client side: picking a tool or a material and its shape
//! from the palette, catching button presses between ticks, the outline on the targeted block,
//! the ghost of what placing would do, and remeshing changed chunks. The simulation decides
//! whether an edit happens; this only asks and shows.

use std::collections::HashMap;

use api::{ChunkPos, Vec3 as ZVec3, WorldPos};
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use sim::{BlockId, BlockRegistry, FACE_CENTER, MaterialId, Occupant, Simulation, ToolId, face_region, occupant_boxes};

use crate::controls::Controls;
use crate::solid::SolidMaterial;
use crate::volumes::Volumes;
use crate::{ChunkMesh, Look, PendingToggles, RenderOrigin, Sim, coords, mesher};

/// Scroll distance, in pixels, that moves the palette one slot on touchpads and other
/// pixel-scrolling devices.
const PIXELS_PER_SLOT: f32 = 40.0;

/// What's in hand, and how blocks get placed. Every tool is on offer, then every material with
/// a placeable full block, whether any is carried or not, then every part carried. A material
/// places its full block; shapes come from crafting, as parts. See [`entries`].
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

/// The palette's slots: every tool, then each material with a placeable full block, in the
/// order materials were first named, then each part carried, in the order they were first
/// made, then any other placeable block.
pub fn entries(sim: &Simulation) -> Vec<Entry> {
    let blocks = sim.blocks();
    let mut out: Vec<Entry> = sim.tool_ids().map(Entry::Tool).collect();
    let all = (0..blocks.material_count()).filter_map(|i| blocks.material_by_index(i));
    let (materials, parts): (Vec<_>, Vec<_>) = all.partition(|&m| blocks.part_of(m).is_none());
    let placeable = |b: BlockId| blocks.get(b).is_some_and(|def| def.placeable && def.is_full());
    let main: Vec<(MaterialId, BlockId)> =
        materials.into_iter().map(|m| (m, blocks.material_block(m))).filter(|&(_, b)| placeable(b)).collect();
    out.extend(main.iter().map(|&(m, b)| Entry::Material(m, b)));
    let carried: Vec<_> = parts.into_iter().filter(|&p| sim.inventory().amount(p) >= blocks.unit(p)).collect();
    out.extend(carried.iter().map(|&p| Entry::Part(p, blocks.material_block(p))));
    let listed = |b: BlockId| main.iter().any(|&(_, m)| m == b) || blocks.drop(b).is_some_and(|d| blocks.part_of(d.material).is_some());
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
    pub fn held(&self, sim: &Simulation) -> Held {
        match entries(sim).get(self.slot) {
            Some(Entry::Tool(tool)) => Held::Tool(*tool),
            Some(Entry::Material(_, block) | Entry::Part(_, block) | Entry::Block(block)) => Held::Block(*block),
            None => Held::Nothing,
        }
    }

    /// The material or part in the selected slot, if it holds one: what the drop key throws.
    pub fn material(&self, sim: &Simulation) -> Option<MaterialId> {
        match entries(sim).get(self.slot) {
            Some(Entry::Material(m, _) | Entry::Part(m, _)) => Some(*m),
            _ => None,
        }
    }

    /// Selects the slot holding part `part`, if it's carried.
    pub fn select_part(&mut self, sim: &Simulation, part: MaterialId) {
        if let Some(i) = entries(sim).iter().position(|e| matches!(e, Entry::Part(p, _) if *p == part)) {
            self.slot = i;
        }
    }

    /// The selected block, or empty if a tool or nothing is selected.
    pub fn selected(&self, sim: &Simulation) -> BlockId {
        match self.held(sim) {
            Held::Block(block) => block,
            _ => BlockId::EMPTY,
        }
    }

    /// The selected tool, if one is.
    pub fn tool(&self, sim: &Simulation) -> Option<ToolId> {
        match self.held(sim) {
            Held::Tool(tool) => Some(tool),
            _ => None,
        }
    }
}

/// How many slots the palette has.
pub fn slot_count(sim: &Simulation) -> usize {
    entries(sim).len()
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
}

/// Whether the mouse is captured for playing. Clicks only edit while it is, so the click
/// that captures it doesn't also break a block.
pub(crate) fn playing(cursor: &CursorOptions) -> bool {
    cursor.grab_mode != CursorGrabMode::None
}

/// Latches use and drop presses for the next tick, and picks the palette slot.
#[allow(clippy::too_many_arguments)]
pub(crate) fn edit_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    controls: Res<Controls>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    sim: Res<Sim>,
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
    if controls.placement_mode.just_pressed(&keys, &mouse) {
        palette.precise = !palette.precise;
    }

    let count = slot_count(&sim.0);
    if count == 0 {
        return;
    }
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
pub(crate) fn draw_outline(sim: Res<Sim>, look: Res<Look>, palette: Res<Palette>, origin: Res<RenderOrigin>, mut gizmos: Gizmos) {
    if palette.tool(&sim.0).is_some() {
        return;
    }
    let Some(hit) = sim.0.target(look.yaw, look.pitch) else {
        return;
    };
    let Some(&occupant) = sim.0.universe().main().cell(hit.pos).get(hit.occupant) else {
        return;
    };
    for b in occupant_boxes(occupant, sim.0.blocks()) {
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
    sim: Res<Sim>,
    look: Res<Look>,
    palette: Res<Palette>,
    origin: Res<RenderOrigin>,
    mut gizmos: Gizmos,
) {
    if !palette.precise || palette.tool(&sim.0).is_some() {
        return;
    }
    let Some(hit) = sim.0.target(look.yaw, look.pitch) else {
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
    sim: Res<Sim>,
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
    let blocks = sim.0.blocks();
    let place = sim
        .0
        .target(look.yaw, look.pitch)
        .and_then(|hit| sim.0.placement(&hit, palette.selected(&sim.0), look.yaw, look.pitch, palette.precise));
    let Some(place) = place else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    visibility.set_if_neq(Visibility::Visible);
    if ghost.shown != Some(place.occupant) {
        let mut data = mesher::MeshData::default();
        // Built around the boxel's center, so scaling shrinks it in place.
        mesher::mesh_occupant(&mut data, place.occupant, [-0.5; 3], blocks, &chunk_meshes.volumes, |_| false, &mut Vec::new());
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

/// Rebuilds the meshes of chunks the simulation changed.
pub(crate) fn remesh_changed_chunks(
    mut commands: Commands,
    mut sim: ResMut<Sim>,
    origin: Res<RenderOrigin>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut chunk_meshes: ResMut<ChunkMeshes>,
    mut handles: Query<&mut Mesh3d, With<ChunkMesh>>,
) {
    // Crafting registers blocks while playing; their looks must be known before they're drawn.
    chunk_meshes.volumes.sync(sim.0.blocks());
    let changed = sim.0.take_changed_chunks();
    let grid = sim.0.universe().main();
    for pos in changed {
        let data = mesher::mesh_chunk(grid, pos, sim.0.blocks(), &chunk_meshes.volumes);
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

/// The palette along the bottom of the screen, as three lines: how full the inventory is, the
/// slots (tools, then materials with how much is carried), with the selected one in brackets,
/// and what's held with the keys that change it.
pub(crate) fn update_palette_hud(
    sim: Res<Sim>,
    palette: Res<Palette>,
    controls: Res<Controls>,
    mut text: Single<&mut Text, With<PaletteHud>>,
) {
    let line = palette_text(&sim.0, &palette, &controls);
    if text.0 != line {
        text.0 = line;
    }
}

/// The text of the palette HUD.
pub(crate) fn palette_text(sim: &Simulation, palette: &Palette, controls: &Controls) -> String {
    let blocks = sim.blocks();
    let inventory = sim.inventory();
    let amount = crate::items::amount_label;
    let block_name = |id: BlockId| blocks.get(id).map_or("?".to_owned(), |def| short(&def.name).to_owned());
    let slots = entries(sim)
        .iter()
        .map(|entry| match entry {
            Entry::Tool(tool) => short(&sim.tool(*tool).name).to_owned(),
            Entry::Material(m, _) => format!("{} {}", short(blocks.material_name(*m)), amount(inventory.amount(*m))),
            Entry::Part(p, _) => format!("{} ×{}", part_name(blocks, *p), inventory.amount(*p) / blocks.unit(*p)),
            Entry::Block(block) => block_name(*block),
        })
        .enumerate()
        .map(|(i, name)| if i == palette.slot { format!("[{} {name}]", i + 1) } else { format!(" {} {name} ", i + 1) })
        .collect::<Vec<_>>()
        .join("  ");

    let carried = match sim.capacity() {
        Some(capacity) => {
            let full = if inventory.room(Some(capacity)) == 0 { "  FULL" } else { "" };
            format!("carrying {} / {}{full}", amount(inventory.total()), amount(capacity))
        }
        None => "carrying, no limit".to_owned(),
    };

    let mode = if palette.precise {
        "precise placement: the middle of a face lays it flat, an edge stands it there"
    } else {
        "basic placement"
    };
    let mode = format!("{mode} ({} switches)", controls.placement_mode.label());
    let held = match (palette.material(sim), palette.held(sim)) {
        (Some(m), Held::Block(_)) => {
            let what = match blocks.part_of(m) {
                Some(_) => format!("{}, a crafted shape", part_name(blocks, m)),
                None => format!("{} block, shapes are crafted at a table", short(blocks.material_name(m))),
            };
            let throw = if sim.inventory_def().is_some() {
                format!(", {} throws", crate::controls::label(controls.drop))
            } else {
                String::new()
            };
            format!("{what}{throw}  ·  {mode}")
        }
        (_, Held::Block(_)) => mode,
        _ => String::new(),
    };
    format!("{carried}\n{slots}\n{held}")
}

/// A name without its namespace.
pub(crate) fn short(name: &str) -> &str {
    name.split_once(':').map_or(name, |(_, path)| path)
}

/// A part's name for people: its material, how much of a block it fills, and the first four
/// digits of its shape's hash to tell parts of one size apart, like `wood ½ #3fa2`.
pub(crate) fn part_name(blocks: &BlockRegistry, part: MaterialId) -> String {
    let material = blocks.part_of(part).map_or("?", |m| short(blocks.material_name(m)));
    let tag = blocks.material_name(part).rsplit_once("_s").map_or("", |(_, hash)| &hash[..hash.len().min(4)]);
    format!("{material} {} #{tag}", fraction(blocks.unit(part)))
}

/// A volume as a fraction of a boxel, in the simplest sixteenths, sixty-fourths and so on it
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

#[derive(Component)]
pub(crate) struct PaletteHud;

#[cfg(test)]
mod tests {
    use super::*;
    use api::{BlockDef, I64Vec3};
    use sim::{PlayerInput, Simulation};

    #[test]
    fn the_palette_lists_only_placeable_blocks() {
        let mut blocks = BlockRegistry::default();
        let stone = blocks.register(BlockDef::new("test:stone", true, [0, 0, 0])).unwrap();
        blocks.register(BlockDef::new("test:bedrock", true, [0, 0, 0]).with_placeable(false)).unwrap();
        let dirt = blocks.register(BlockDef::new("test:dirt", true, [0, 0, 0])).unwrap();
        assert_eq!(placeable_blocks(&blocks), [stone, dirt]);
    }

    fn base_sim() -> Simulation {
        let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
        Simulation::with_mods(&[&data, &base::BaseMod, &base_movement::BaseMovementMod]).unwrap()
    }

    #[test]
    fn the_palette_holds_the_tools_then_the_materials_then_parts_carried() {
        let mut sim = base_sim();
        let club = sim.tool_ids().next().expect("base registers a tool");
        assert_eq!(sim.tool(club).name, base::CLUB);
        let blocks = sim.blocks();
        let brick = blocks.material(base::BRICK).unwrap();
        let brick_block = blocks.id(base::BRICK).unwrap();
        // Every material with a placeable block, in the order they were first named, carried
        // or not. Grass is made of dirt and the marker isn't placeable. The test area's crafted
        // shapes aren't carried, so they aren't on offer.
        let order = [
            base::STONE,
            base::DIRT,
            base::BRICK,
            base::ICE,
            base::WOOD,
            base::LOG,
            base::GRAVEL,
            base::CRAFTING_TABLE,
        ];
        let materials = order.map(|name| Entry::Material(blocks.material(name).unwrap(), blocks.id(name).unwrap()));
        assert_eq!(entries(&sim), [&[Entry::Tool(club)][..], &materials[..]].concat());

        let at = |slot| Palette { slot, ..Default::default() };
        assert_eq!(at(0).held(&sim), Held::Tool(club));
        assert_eq!((at(0).tool(&sim), at(0).selected(&sim)), (Some(club), BlockId::EMPTY), "a tool places nothing");
        assert_eq!(at(0).material(&sim), None, "nor throws");
        assert_eq!((at(3).tool(&sim), at(3).selected(&sim), at(3).material(&sim)), (None, brick_block, Some(brick)));
        assert_eq!(at(slot_count(&sim)).held(&sim), Held::Nothing);

        // A part shows up once one is carried, after the materials.
        let wood = sim.blocks().material(base::WOOD).unwrap();
        let slab = sim.shaped_block(wood, &base::shapes::slab()).unwrap();
        let part = sim.blocks().drop(slab).unwrap().material;
        sim.spawn_player().unwrap();
        sim.spawn_item(part, sim::FULL_VOLUME / 2, sim.player().unwrap().pos());
        for _ in 0..5 {
            sim.step(&PlayerInput::default());
        }
        assert_eq!(sim.inventory().amount(part), sim::FULL_VOLUME / 2, "picked up");
        assert_eq!(entries(&sim).last(), Some(&Entry::Part(part, slab)));
        let mut palette = at(0);
        palette.select_part(&sim, part);
        assert_eq!((palette.slot, palette.selected(&sim), palette.material(&sim)), (9, slab, Some(part)));
    }

    #[test]
    fn the_hud_shows_amounts_the_fill_and_what_is_held() {
        let sim = base_sim();
        let palette = Palette { slot: 3, ..Default::default() };
        let text = palette_text(&sim, &palette, &Controls::default());
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "carrying 32 / 64");
        assert!(lines[1].contains("[4 brick 16]") && lines[1].contains("6 wood 16"), "{}", lines[1]);
        assert!(lines[1].contains("3 dirt 0"), "materials not carried are listed too: {}", lines[1]);
        assert!(lines[2].starts_with("brick block, shapes are crafted at a table, Q throws"), "{}", lines[2]);
        let blocks = sim.blocks();
        let stone = blocks.material(base::STONE).unwrap();
        let stairs = blocks.find_shaped(stone, &base::shapes::stairs()).unwrap();
        let name = part_name(blocks, blocks.drop(stairs).unwrap().material);
        assert!(name.starts_with("stone ¾ #") && name.len() == "stone ¾ #".len() + 4, "{name}");
        assert_eq!((fraction(sim::FULL_VOLUME / 2), fraction(sim::FULL_VOLUME * 3 / 16)), ("½".into(), "3/16".into()));
        assert_eq!(fraction(sim::FULL_VOLUME * 2), "2");
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
        let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
        let mut sim = Simulation::with_mods(&[&data, &base::BaseMod, &base_movement::BaseMovementMod]).unwrap();
        sim.spawn_player().unwrap();
        // Stand east of a slab with a fence post through it and aim down at the post's top.
        let blocks = sim.blocks();
        let shaped = |m: &str, shape| blocks.find_shaped(blocks.material(m).unwrap(), &shape).unwrap();
        let (fence, slab) = (shaped(base::WOOD, base::shapes::post()), shaped(base::BRICK, base::shapes::slab()));
        let cell = api::BoxelPos::new(-9, -4, 0);
        assert_eq!(sim.universe().main().cell(cell).len(), 2);
        sim.teleport_player(WorldPos::new(I64Vec3::new(-8, -4, 0), ZVec3::new(0.5, 0.5, 0.0)));
        let eye = sim.player().unwrap().eye();
        let aim = eye.delta_to(WorldPos::new(cell.0, ZVec3::new(0.5, 0.5, 1.0)));
        let (yaw, pitch) = (aim.y.atan2(aim.x), aim.z.atan2(aim.truncate().length()));
        let hit = sim.target(yaw, pitch).unwrap();
        assert_eq!(sim.universe().main().cell(hit.pos)[hit.occupant].block, fence);

        let volumes = Volumes::build(sim.looks(), sim.blocks());
        let chunk = ChunkPos::new(-1, -1, 0);
        let before = mesher::mesh_chunk(sim.universe().main(), chunk, sim.blocks(), &volumes).quad_count();
        sim.step(&PlayerInput { yaw, pitch, use_pressed: true, use_held: true, ..Default::default() });
        let left: Vec<_> = sim.universe().main().cell(cell).iter().map(|o| o.block).collect();
        assert_eq!(left, [slab]);
        assert!(sim.take_changed_chunks().contains(&chunk));
        let after = mesher::mesh_chunk(sim.universe().main(), chunk, sim.blocks(), &volumes).quad_count();
        assert_eq!(after, before - 5, "the post's faces are gone; its bottom was already hidden by the floor");
    }

    #[test]
    fn breaking_a_block_remeshes_its_chunk() {
        let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
        let mut sim = Simulation::with_mods(&[&data, &base::BaseMod, &base_movement::BaseMovementMod]).unwrap();
        sim.spawn_player().unwrap();
        sim.teleport_player(WorldPos::new(I64Vec3::new(10, 10, 0), ZVec3::new(0.5, 0.5, 0.0)));
        let below = ChunkPos::new(0, 0, -1);
        let volumes = Volumes::build(sim.looks(), sim.blocks());
        let before = mesher::mesh_chunk(sim.universe().main(), below, sim.blocks(), &volumes).quad_count();

        let down = -89f32.to_radians();
        sim.step(&PlayerInput { pitch: down, use_pressed: true, use_held: true, ..Default::default() });
        let changed = sim.take_changed_chunks();
        assert!(changed.contains(&below));
        // The floor's top layer is the top of this chunk, so the chunk above is told too.
        assert!(changed.contains(&ChunkPos::new(0, 0, 0)));

        // One top face gone; four side faces and the floor of the hole appear.
        let after = mesher::mesh_chunk(sim.universe().main(), below, sim.blocks(), &volumes).quad_count();
        assert_eq!(after, before + 4);
    }
}
