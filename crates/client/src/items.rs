//! Drawing loose items: a small cube of each item's material, blended between ticks, turning
//! slowly and bobbing while it rests. Purely a view; the simulation moves them.

use std::collections::HashMap;

use api::Vec3 as ZVec3;
use bevy::prelude::*;
use sim::{MaterialId, Occupant};

use crate::editing::ChunkMeshes;
use crate::{RenderOrigin, Sim, coords, mesher};

/// Radians a second an item turns.
const SPIN_SPEED: f32 = 1.2;
/// How high a resting item bobs, in meters, and how many bobs a second.
const BOB_HEIGHT: f32 = 0.05;
const BOB_RATE: f32 = 0.6;

/// The entity drawing each item, by the item's ID, and one mesh per material.
#[derive(Resource, Default)]
pub(crate) struct ItemViews {
    entities: HashMap<u64, Entity>,
    meshes: HashMap<MaterialId, Handle<Mesh>>,
}

#[derive(Component)]
pub(crate) struct ItemView;

/// Spawns, moves and despawns item entities to match the simulation's items.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sync_items(
    mut commands: Commands,
    sim: Res<Sim>,
    origin: Res<RenderOrigin>,
    fixed: Res<Time<Fixed>>,
    time: Res<Time>,
    chunk_meshes: Res<ChunkMeshes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut views: ResMut<ItemViews>,
    mut transforms: Query<&mut Transform, With<ItemView>>,
) {
    let Some(def) = sim.0.item_drop_def() else {
        return;
    };
    let alpha = fixed.overstep_fraction();
    let seconds = time.elapsed_secs();
    let blocks = sim.0.blocks();
    let mut live = Vec::with_capacity(sim.0.items().len());
    for item in sim.0.items().iter() {
        live.push(item.id);
        let center = item.prev_pos.translate(item.prev_pos.delta_to(item.pos) * alpha);
        // Each item gets its own phase, so a pile doesn't turn in step.
        let phase = item.id as f32 * 2.4;
        // Bobs upward only, so it never sinks into the floor.
        let bob = if item.on_ground { BOB_HEIGHT * 0.5 * (1.0 - (seconds * BOB_RATE * std::f32::consts::TAU + phase).cos()) } else { 0.0 };
        let transform = Transform::from_translation(coords::render_translation(center.translate(ZVec3::Z * bob), origin.cell))
            .with_rotation(coords::turn_about_up(seconds * SPIN_SPEED + phase))
            .with_scale(Vec3::splat(item.size(def)));
        if let Some(mut t) = views.entities.get(&item.id).and_then(|&e| transforms.get_mut(e).ok()) {
            *t = transform;
            continue;
        }
        let material = item.drop.material;
        let mesh = views
            .meshes
            .entry(material)
            .or_insert_with(|| {
                let mut data = mesher::MeshData::default();
                // Built around the boxel's center, so it turns and scales in place.
                let occupant = Occupant::new(blocks.material_block(material));
                mesher::mesh_occupant(&mut data, occupant, [-0.5; 3], blocks, &chunk_meshes.atlas, |_| false, &mut Vec::new());
                meshes.add(crate::to_bevy_mesh(data))
            })
            .clone();
        let entity = commands.spawn((Mesh3d(mesh), MeshMaterial3d(chunk_meshes.material.clone()), transform, ItemView)).id();
        views.entities.insert(item.id, entity);
    }
    views.entities.retain(|id, entity| {
        let keep = live.contains(id);
        if !keep {
            commands.entity(*entity).despawn();
        }
        keep
    });
}

/// An amount of material as boxels' worth, like `2.5`, without trailing zeros.
pub(crate) fn amount_label(amount: u32) -> String {
    let boxels = amount as f64 / sim::FULL_VOLUME as f64;
    let text = format!("{boxels:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// What the player has picked up, as `stone 2.5, dirt 1`, or empty with nothing yet.
pub(crate) fn collected_line(sim: &sim::Simulation) -> String {
    let blocks = sim.blocks();
    sim.collected()
        .map(|(m, amount)| {
            let name = blocks.material_name(m);
            format!("{} {}", name.split_once(':').map_or(name, |(_, path)| path), amount_label(amount))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_read_as_boxels() {
        let full = sim::FULL_VOLUME;
        assert_eq!(amount_label(full), "1");
        assert_eq!(amount_label(full * 5 / 2), "2.5");
        assert_eq!(amount_label(full * 3 / 4), "0.75");
        assert_eq!(amount_label(full * 10), "10");
    }
}
