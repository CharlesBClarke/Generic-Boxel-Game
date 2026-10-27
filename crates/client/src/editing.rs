//! Breaking and placing blocks on the client side: picking from the palette, catching button
//! presses between ticks, the outline on the targeted block and remeshing changed chunks.
//! The simulation decides whether an edit happens; this only asks and shows.

use std::collections::HashMap;

use api::{ChunkPos, Vec3 as ZVec3, WorldPos};
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use sim::{BlockId, BlockRegistry};

use crate::controls::Controls;
use crate::{ChunkMesh, Look, PendingToggles, RenderOrigin, Sim, coords, mesher};

/// Scroll distance, in pixels, that moves the palette one slot on touchpads and other
/// pixel-scrolling devices.
const PIXELS_PER_SLOT: f32 = 40.0;

/// Which block gets placed. There's no inventory yet, so every placeable block is on offer.
#[derive(Resource, Default)]
pub struct Palette {
    pub slot: usize,
    scrolled: f32,
}

impl Palette {
    /// The selected block, or empty if nothing is placeable.
    pub fn selected(&self, blocks: &BlockRegistry) -> BlockId {
        placeable_blocks(blocks).get(self.slot).copied().unwrap_or(BlockId::EMPTY)
    }
}

/// Every block a player may place, in registration order.
pub fn placeable_blocks(blocks: &BlockRegistry) -> Vec<BlockId> {
    blocks.iter().filter(|(_, def)| def.placeable).map(|(id, _)| id).collect()
}

/// The mesh entity for each chunk that has geometry, and the material they share.
#[derive(Resource)]
pub(crate) struct ChunkMeshes {
    pub material: Handle<StandardMaterial>,
    pub entities: HashMap<ChunkPos, Entity>,
}

/// Whether the mouse is captured for playing. Clicks only edit while it is, so the click
/// that captures it doesn't also break a block.
pub(crate) fn playing(cursor: &CursorOptions) -> bool {
    cursor.grab_mode != CursorGrabMode::None
}

/// Latches break and place presses for the next tick, and picks the palette slot.
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
    if controls.break_block.just_pressed(&keys, &mouse) {
        toggles.break_pressed = true;
    }
    if controls.place_block.just_pressed(&keys, &mouse) {
        toggles.place_pressed = true;
    }

    let count = placeable_blocks(sim.0.blocks()).len();
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

/// Outlines the block that breaking would remove.
pub(crate) fn draw_outline(sim: Res<Sim>, look: Res<Look>, origin: Res<RenderOrigin>, mut gizmos: Gizmos) {
    let Some(hit) = sim.0.target(look.yaw, look.pitch) else {
        return;
    };
    let center = WorldPos::at_boxel(hit.pos).translate(ZVec3::splat(0.5));
    // Slightly larger than the block, so the lines aren't hidden by its own faces.
    let transform =
        Transform::from_translation(coords::render_translation(center, origin.cell)).with_scale(Vec3::splat(1.005));
    gizmos.cube(transform, Color::srgb(0.05, 0.05, 0.05));
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
    let changed = sim.0.take_changed_chunks();
    let grid = sim.0.universe().main();
    for pos in changed {
        let data = mesher::mesh_chunk(grid, pos, sim.0.blocks());
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

/// The palette along the bottom of the screen, with the selected block in brackets.
pub(crate) fn update_palette_hud(sim: Res<Sim>, palette: Res<Palette>, mut text: Single<&mut Text, With<PaletteHud>>) {
    let blocks = sim.0.blocks();
    let line = placeable_blocks(blocks)
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            let name = blocks.get(id).map_or("?", |def| def.name.split_once(':').map_or(&def.name, |(_, path)| path));
            if i == palette.slot { format!("[{} {name}]", i + 1) } else { format!(" {} {name} ", i + 1) }
        })
        .collect::<Vec<_>>()
        .join("  ");
    if text.0 != line {
        text.0 = line;
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
        assert_eq!(Palette { slot: 1, scrolled: 0.0 }.selected(&blocks), dirt);
        assert_eq!(Palette { slot: 5, scrolled: 0.0 }.selected(&blocks), BlockId::EMPTY);
    }

    #[test]
    fn breaking_a_block_remeshes_its_chunk() {
        let mut sim = Simulation::with_mods(&[&base::BaseMod, &base_movement::BaseMovementMod]).unwrap();
        sim.spawn_player().unwrap();
        sim.teleport_player(WorldPos::new(I64Vec3::new(10, 10, 0), ZVec3::new(0.5, 0.5, 0.0)));
        let below = ChunkPos::new(0, 0, -1);
        let before = mesher::mesh_chunk(sim.universe().main(), below, sim.blocks()).quad_count();

        let down = -89f32.to_radians();
        sim.step(&PlayerInput { pitch: down, break_pressed: true, break_held: true, ..Default::default() });
        let changed = sim.take_changed_chunks();
        assert!(changed.contains(&below));
        // The floor's top layer is the top of this chunk, so the chunk above is told too.
        assert!(changed.contains(&ChunkPos::new(0, 0, 0)));

        // One top face gone; four side faces and the floor of the hole appear.
        let after = mesher::mesh_chunk(sim.universe().main(), below, sim.blocks()).quad_count();
        assert_eq!(after, before + 4);
    }
}
