//! Crystal blasts on the client side: a bright beam along where it went, fading out, and the
//! view shaking more the closer it was.

use api::{Vec3 as ZVec3, WorldPos};
use bevy::prelude::*;

use crate::RenderOrigin;
use crate::combat::{CombatFeel, ViewKick};
use crate::coords;
use crate::world::ClientWorld;

/// A beam being drawn.
#[derive(Component)]
pub(crate) struct BeamView {
    /// Its middle.
    center: WorldPos,
    /// 1 along the beam's axis, 0 across it, in render space.
    along: Vec3,
    age: f32,
    life: f32,
    material: Handle<StandardMaterial>,
}

/// The beam's color at full strength: a pale, hot green.
const BEAM: [f32; 3] = [0.75, 1.0, 0.8];

/// Draws every blast since the last frame and shakes the view.
pub(crate) fn play_blasts(
    mut commands: Commands,
    mut world: ResMut<ClientWorld>,
    feel: Res<CombatFeel>,
    origin: Res<RenderOrigin>,
    mut kick: ResMut<ViewKick>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let eye = world.player().map(|p| p.pos());
    for blast in world.take_blasts() {
        let center = blast.from.translate(blast.dir * blast.length * 0.5);
        if let Some(eye) = eye {
            let near = (1.0 - eye.delta_to(center).length() / feel.blast_reach.max(0.1)).max(0.0_f32);
            if near > 0.0 {
                kick.shake(feel.blast_shake * near, feel.shake_time * 2.0);
            }
        }
        // A box along the beam, `blast_width` across.
        let along = blast.dir.abs();
        let size = along * blast.length.max(0.05) + (ZVec3::ONE - along) * feel.blast_width;
        let size = coords::to_render(size).abs();
        let material = materials.add(StandardMaterial {
            base_color: Color::linear_rgba(BEAM[0], BEAM[1], BEAM[2], 1.0),
            emissive: LinearRgba::rgb(BEAM[0] * 4.0, BEAM[1] * 4.0, BEAM[2] * 4.0),
            unlit: true,
            alpha_mode: AlphaMode::Add,
            ..default()
        });
        let mesh = meshes.add(Cuboid::new(size.x, size.y, size.z));
        let transform = Transform::from_translation(coords::render_translation(center, origin.cell));
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material.clone()),
            transform,
            BeamView { center, along: coords::to_render(along), age: 0.0, life: feel.blast_time.max(0.01), material },
        ));
    }
}

/// Fades beams out, thinning them, and keeps them where they are as the render origin moves.
pub(crate) fn update_beams(
    mut commands: Commands,
    time: Res<Time>,
    origin: Res<RenderOrigin>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut beams: Query<(Entity, &mut BeamView, &mut Transform)>,
) {
    for (entity, mut beam, mut transform) in &mut beams {
        beam.age += time.delta_secs();
        if beam.age >= beam.life {
            materials.remove(&beam.material);
            commands.entity(entity).despawn();
            continue;
        }
        let left = 1.0 - beam.age / beam.life;
        transform.translation = coords::render_translation(beam.center, origin.cell);
        if let Some(m) = materials.get_mut(&beam.material) {
            let fade = left * left;
            m.base_color = Color::linear_rgba(BEAM[0] * fade, BEAM[1] * fade, BEAM[2] * fade, 1.0);
        }
        // Thins toward its middle line as it fades: scaled only across the beam.
        let thin = 0.3 + 0.7 * left;
        transform.scale = beam.along.abs() + (Vec3::ONE - beam.along.abs()) * thin;
    }
}
