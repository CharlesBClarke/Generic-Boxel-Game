//! Drawing loose items as debris: a small copy of the block each came from, blended between
//! ticks. In the air it tumbles about the axis it was thrown along; on landing it turns the
//! short way onto a face so it lies flat, and squashes for a moment, as it does on each bounce.
//! At rest it lies still. Purely a view; the simulation moves items and owns their boxes.

use std::collections::HashMap;

use api::{BlockBox, Orientation, Vec3 as ZVec3};
use bevy::prelude::*;
use sim::{BlockId, BlockRegistry, Item, Occupant};

use crate::editing::ChunkMeshes;
use crate::{RenderOrigin, Sim, coords, mesher};

/// Tumble speed, radians a second for each meter a second the item is thrown at.
const TUMBLE_PER_SPEED: f32 = 2.5;
/// Share of its tumble an item keeps through a bounce.
const BOUNCE_TUMBLE: f32 = 0.6;
/// How quickly a landed item turns onto its face, per second.
const SETTLE_RATE: f32 = 25.0;
/// How long a squash lasts, in seconds, and how far it squashes for each meter a second of
/// impact, up to the most it ever squashes.
const SQUASH_TIME: f32 = 0.12;
const SQUASH_PER_SPEED: f32 = 0.05;
const MAX_SQUASH: f32 = 0.3;
/// Falling slower than this, in meters a second, when hitting the ground makes no squash.
const MIN_IMPACT: f32 = 0.5;

/// How each item is drawn, by the item's ID, and one mesh per block.
#[derive(Resource, Default)]
pub(crate) struct ItemViews {
    views: HashMap<u64, View>,
    meshes: HashMap<BlockId, Handle<Mesh>>,
}

/// One item's entities and motion. The outer entity holds the position and the squash, along
/// the world's up; the inner one holds the tumble and the item's scale.
struct View {
    outer: Entity,
    inner: Entity,
    block: BlockId,
    rotation: Quat,
    /// Render-space axis times radians a second.
    spin: Vec3,
    /// The turns that leave the shape's box the same size along each axis, so lying on one
    /// matches the box the simulation collides with.
    rests: Vec<Quat>,
    /// Where it's turning to on the ground.
    target: Option<Quat>,
    was_on_ground: bool,
    last_vz: f32,
    /// Seconds since the last impact, and how hard it squashed.
    squash: (f32, f32),
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
    mut state: ResMut<ItemViews>,
    mut transforms: Query<&mut Transform, With<ItemView>>,
) {
    let Some(def) = sim.0.item_drop_def() else {
        return;
    };
    let alpha = fixed.overstep_fraction();
    let dt = time.delta_secs();
    let blocks = sim.0.blocks();
    let ItemViews { views, meshes: block_meshes } = &mut *state;
    let mut live = Vec::with_capacity(sim.0.items().len());
    for item in sim.0.items().iter() {
        live.push(item.id);
        let mesh = block_meshes.entry(item.block).or_insert_with(|| meshes.add(item_mesh(item, blocks, &chunk_meshes))).clone();
        // A new item's entities are spawned below, once its transforms are known: spawned ones
        // only exist after this system, so setting them through the query would miss the
        // first frame and draw it at the origin.
        let new = !views.contains_key(&item.id);
        let view = views.entry(item.id).or_insert_with(|| View::new(item, Entity::PLACEHOLDER, Entity::PLACEHOLDER, blocks));
        if view.block != item.block {
            // Merged into its material's own block: show that shape instead.
            view.block = item.block;
            view.rests = rest_turns(item.block, blocks);
            view.target = None;
            commands.entity(view.inner).insert(Mesh3d(mesh.clone()));
        }
        view.update(item, dt);

        let center = item.prev_pos.translate(item.prev_pos.delta_to(item.pos) * alpha);
        let (since, amount) = view.squash;
        let squash = if since < SQUASH_TIME { amount * (1.0 - since / SQUASH_TIME) } else { 0.0 };
        // Drawn bigger than its box when merged, growing up from the box's bottom; squashed
        // about its middle, then lowered so its bottom stays on the ground.
        let box_height = item.extent(def).z;
        let height = box_height * item.growth();
        let rise = (height - box_height) * 0.5 - squash * height * 0.5;
        let outer = Transform::from_translation(coords::render_translation(center.translate(ZVec3::Z * rise), origin.cell))
            .with_scale(coords::to_render(ZVec3::new(1.0 + squash * 0.5, 1.0 + squash * 0.5, 1.0 - squash)).abs());
        let inner = Transform::from_rotation(view.rotation).with_scale(Vec3::splat(item.scale(def) * item.growth()));
        if new {
            view.inner = commands.spawn((Mesh3d(mesh), MeshMaterial3d(chunk_meshes.material.clone()), inner, ItemView)).id();
            view.outer = commands.spawn((outer, Visibility::default(), ItemView)).add_child(view.inner).id();
            continue;
        }
        if let Ok(mut t) = transforms.get_mut(view.outer) {
            *t = outer;
        }
        if let Ok(mut t) = transforms.get_mut(view.inner) {
            *t = inner;
        }
    }
    views.retain(|id, view| {
        let keep = live.contains(id);
        if !keep {
            commands.entity(view.outer).despawn();
        }
        keep
    });
}

impl View {
    fn new(item: &Item, outer: Entity, inner: Entity, blocks: &BlockRegistry) -> Self {
        // Thrown along its velocity, it tumbles forward: about the level axis across it.
        let v = item.velocity;
        let across = ZVec3::Z.cross(ZVec3::new(v.x, v.y, 0.0)).normalize_or(ZVec3::X);
        Self {
            outer,
            inner,
            block: item.block,
            rotation: Quat::IDENTITY,
            spin: coords::to_render(across) * v.length() * TUMBLE_PER_SPEED,
            rests: rest_turns(item.block, blocks),
            target: None,
            was_on_ground: item.on_ground,
            last_vz: v.z,
            squash: (SQUASH_TIME, 0.0),
        }
    }

    /// Advances the tumble, settling and squash by `dt` seconds of frame time.
    fn update(&mut self, item: &Item, dt: f32) {
        self.squash.0 += dt;
        let vz = item.velocity.z;
        let bounced = self.last_vz < -MIN_IMPACT && vz > 0.0 && !item.pulled;
        let landed = item.on_ground && !self.was_on_ground;
        if bounced || landed {
            let impact = -self.last_vz.min(0.0);
            if impact >= MIN_IMPACT {
                self.squash = (0.0, (impact * SQUASH_PER_SPEED).min(MAX_SQUASH));
            }
            self.spin *= BOUNCE_TUMBLE;
        }
        if item.on_ground {
            self.spin = Vec3::ZERO;
            let rotation = self.rotation;
            let target = *self.target.get_or_insert_with(|| nearest(&self.rests, rotation));
            self.rotation = self.rotation.slerp(target, 1.0 - (-SETTLE_RATE * dt).exp());
        } else {
            self.target = None;
            self.rotation = (Quat::from_scaled_axis(self.spin * dt) * self.rotation).normalize();
        }
        self.was_on_ground = item.on_ground;
        self.last_vz = vz;
    }
}

/// A mesh of `item`'s block, unturned and centered on the middle of its shape, so it turns and
/// scales about the middle of the box the simulation collides with.
fn item_mesh(item: &Item, blocks: &BlockRegistry, chunk_meshes: &ChunkMeshes) -> Mesh {
    let mut data = mesher::MeshData::default();
    let middle = ((item.bounds.min + item.bounds.max) * -0.5).to_array();
    mesher::mesh_occupant(&mut data, Occupant::new(item.block), middle, blocks, &chunk_meshes.atlas, |_| false, &mut Vec::new());
    crate::to_bevy_mesh(data)
}

/// Every turn of `block` that leaves its shape's box the same size along each axis, as render
/// rotations. Items turn about the middle of that box, so these are the ways it can lie.
fn rest_turns(block: BlockId, blocks: &BlockRegistry) -> Vec<Quat> {
    let shape = blocks.get(block).map_or(&[][..], |def| &def.shape[..]);
    let min = |i: usize| shape.iter().map(|b| b.min[i] as i32).min().unwrap_or(0);
    let max = |i: usize| shape.iter().map(|b| b.max[i] as i32).max().unwrap_or(BlockBox::SIZE as i32);
    let size = [0, 1, 2].map(|i| max(i) - min(i));
    Orientation::all().into_iter().filter(|o| o.apply(size).map(i32::abs) == size).map(to_quat).collect()
}

/// The render rotation for a grid turn.
fn to_quat(o: Orientation) -> Quat {
    // Each render axis, turned in Z-up space and brought back.
    let turn = |axis: Vec3| {
        let z_up = coords::from_render(axis).round().as_ivec3().to_array();
        coords::to_render(Vec3::from_array(o.apply(z_up).map(|c| c as f32)))
    };
    Quat::from_mat3(&Mat3::from_cols(turn(Vec3::X), turn(Vec3::Y), turn(Vec3::Z)))
}

/// The turn in `options` closest to `rotation`, or no turn if there are none.
fn nearest(options: &[Quat], rotation: Quat) -> Quat {
    options.iter().copied().max_by(|a, b| a.dot(rotation).abs().total_cmp(&b.dot(rotation).abs())).unwrap_or(Quat::IDENTITY)
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
    use api::BlockDef;

    #[test]
    fn a_new_item_is_drawn_where_it_is_from_its_first_frame() {
        let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
        let mut sim = sim::Simulation::with_mods(&[&data, &base::BaseMod, &base_movement::BaseMovementMod]).unwrap();
        sim.spawn_player().unwrap();
        sim.teleport_player(api::WorldPos::new(api::I64Vec3::new(10, 10, 0), ZVec3::new(0.5, 0.5, 0.0)));
        let down = -89f32.to_radians();
        sim.step(&sim::PlayerInput { pitch: down, break_pressed: true, break_held: true, ..Default::default() });
        let item_pos = sim.items().iter().next().expect("the broken floor dropped an item").pos;

        let atlas = crate::atlas::Atlas::build(sim.textures(), sim.blocks());
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(Sim(sim))
            .init_resource::<RenderOrigin>()
            .init_resource::<ItemViews>()
            .insert_resource(ChunkMeshes { material: Handle::default(), atlas, entities: HashMap::new() })
            .add_systems(Update, sync_items);
        app.update();

        let world = app.world_mut();
        let outer: Vec<_> = world.query_filtered::<&Transform, (With<ItemView>, Without<Mesh3d>)>().iter(world).copied().collect();
        let [outer] = outer[..] else { panic!("one item drawn, got {}", outer.len()) };
        let want = coords::render_translation(item_pos, api::I64Vec3::ZERO);
        assert!((outer.translation - want).length() < 0.2, "drawn at {} on its first frame, not at {want}", outer.translation);
    }

    #[test]
    fn amounts_read_as_boxels() {
        let full = sim::FULL_VOLUME;
        assert_eq!(amount_label(full), "1");
        assert_eq!(amount_label(full * 5 / 2), "2.5");
        assert_eq!(amount_label(full * 3 / 4), "0.75");
        assert_eq!(amount_label(full * 10), "10");
    }

    #[test]
    fn grid_turns_become_the_same_render_rotations() {
        for o in Orientation::all() {
            let q = to_quat(o);
            for v in [[1, 0, 0], [0, 1, 0], [0, 0, 1]] {
                let z_up = Vec3::from_array(v.map(|c| c as f32));
                let want = coords::to_render(Vec3::from_array(o.apply(v).map(|c| c as f32)));
                assert!((q * coords::to_render(z_up) - want).length() < 1e-5, "{o:?} on {v:?}");
            }
        }
    }

    #[test]
    fn a_slab_rests_only_flat_and_a_cube_on_any_face() {
        let mut blocks = BlockRegistry::default();
        let cube = blocks.register(BlockDef::new("test:cube", true, [0; 3])).unwrap();
        let slab = blocks.register(BlockDef::new("test:slab", true, [0; 3]).with_shape(vec![BlockBox::new([0, 0, 0], [16, 16, 8])])).unwrap();
        assert_eq!(rest_turns(cube, &blocks).len(), 24);
        // A slab lies flat, either way up, turned any quarter about the vertical.
        let slab_rests = rest_turns(slab, &blocks);
        assert_eq!(slab_rests.len(), 8);
        let up = coords::to_render(Vec3::Z);
        assert!(slab_rests.iter().all(|q| (*q * up).dot(up).abs() > 1.0 - 1e-5), "every rest keeps it flat");
        // Tipped most of the way onto its side, it turns the short way: on over, upside down.
        let tipped = Quat::from_axis_angle(coords::to_render(Vec3::X), 2.0);
        assert!((nearest(&slab_rests, tipped) * up + up).length() < 1e-5);
    }
}
