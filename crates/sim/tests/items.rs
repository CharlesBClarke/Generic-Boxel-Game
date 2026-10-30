//! Breaking blocks drops loose items, which the player picks up.

use api::{
    ApiError, BlockBox, BlockDef, BoxelPos, I64Vec3, InteractionDef, ItemDropDef, Mod, Orientation, PlayerDef,
    Registrar, Vec3, WorldEdit, WorldPos,
};
use sim::{FULL_VOLUME, PlayerInput, Simulation};

/// A stone floor with a stone slab two boxels east of spawn, and item drops if `drops`.
struct Quarry {
    drops: bool,
}

impl Mod for Quarry {
    fn name(&self) -> &str {
        "test"
    }

    fn register(&self, r: &mut dyn Registrar) -> Result<(), ApiError> {
        r.register_block(BlockDef::new("test:stone", true, [0, 0, 0]).with_material("test:stone"))?;
        let slab = vec![BlockBox::new([0, 0, 0], [16, 16, 8])];
        r.register_block(BlockDef::new("test:slab", true, [0, 0, 0]).with_shape(slab).with_material("test:stone"))?;
        r.register_movement_controller(base_movement::QUAKE, Box::new(base_movement::QuakeController::default()))?;
        r.register_player(PlayerDef {
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            step_height: 0.6,
            controller: base_movement::QUAKE.into(),
        })?;
        r.register_interaction(InteractionDef { reach: 5.0, repeat_rate: 4.0 })?;
        if self.drops {
            r.register_item_drops(ItemDropDef {
                pop_speed: 3.0,
                gravity: 20.0,
                bounce: 0.3,
                drag: 8.0,
                size: 0.25,
                pull_radius: 2.0,
                pull_speed: 8.0,
                pickup_radius: 0.3,
                merge_radius: 1.0,
            })?;
        }
        Ok(())
    }

    fn build_world(&self, w: &mut dyn WorldEdit) -> Result<(), ApiError> {
        w.fill(BoxelPos::new(-5, -5, -1), BoxelPos::new(10, 5, -1), "test:stone")?;
        w.add_block(BoxelPos::new(2, 0, 0), "test:slab", Orientation::IDENTITY)?;
        w.set_spawn(WorldPos::new(I64Vec3::ZERO, Vec3::new(0.5, 0.5, 0.0)));
        Ok(())
    }
}

fn quarry(drops: bool) -> Simulation {
    let mut sim = Simulation::with_mods(&[&Quarry { drops }]).unwrap();
    sim.spawn_player().unwrap();
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
    sim
}

/// The pitch, looking east, that aims at the middle of the slab's top.
fn slab_pitch(sim: &Simulation) -> f32 {
    let aim = sim.player().unwrap().eye().delta_to(WorldPos::new(I64Vec3::new(2, 0, 0), Vec3::new(0.5, 0.5, 0.5)));
    aim.z.atan2(aim.truncate().length())
}

fn break_slab(sim: &mut Simulation) {
    let pitch = slab_pitch(sim);
    assert_eq!(sim.target(0.0, pitch).map(|h| h.pos), Some(BoxelPos::new(2, 0, 0)));
    sim.step(&PlayerInput { pitch, break_pressed: true, break_held: true, ..Default::default() });
    assert!(sim.universe().main().block(BoxelPos::new(2, 0, 0)).is_empty(), "the slab broke");
}

#[test]
fn breaking_a_slab_drops_half_its_material_which_the_player_picks_up() {
    let mut sim = quarry(true);
    break_slab(&mut sim);
    let stone = sim.blocks().material("test:stone").unwrap();
    let item = sim.items().iter().next().expect("an item dropped").clone();
    assert_eq!((item.drop.material, item.drop.amount), (stone, FULL_VOLUME / 2));
    let start = WorldPos::new(I64Vec3::new(2, 0, 0), Vec3::new(0.5, 0.5, 0.25));
    assert!(start.delta_to(item.pos).length() < 0.2, "drops from where the slab was");
    assert!(item.velocity.z > 0.0, "and pops up");
    assert_eq!(sim.collected().count(), 0);

    // Step toward it until it's pulled in and picked up.
    for tick in 0..120 {
        let movement = if tick < 15 { api::Vec2::Y } else { api::Vec2::ZERO };
        sim.step(&PlayerInput { movement, ..Default::default() });
    }
    assert!(sim.items().is_empty(), "picked up");
    assert_eq!(sim.collected().collect::<Vec<_>>(), [(stone, FULL_VOLUME / 2)]);
}

#[test]
fn a_dropped_item_left_alone_settles_on_the_floor() {
    let mut sim = quarry(true);
    break_slab(&mut sim);
    // Out of the pull's reach.
    sim.teleport_player(WorldPos::new(I64Vec3::new(-4, 0, 0), Vec3::new(0.5, 0.5, 0.0)));
    for _ in 0..240 {
        sim.step(&PlayerInput::default());
    }
    let item = sim.items().iter().next().expect("still there");
    assert!(item.on_ground);
    let floor = WorldPos::new(I64Vec3::ZERO, Vec3::ZERO);
    let height = item.extent(sim.item_drop_def().unwrap()).z;
    assert_eq!(item.block, sim.blocks().id("test:slab").unwrap(), "shaped like the slab it came from");
    assert!((floor.delta_to(item.pos).z - height * 0.5).abs() < 1e-3, "resting on the floor");
}

#[test]
fn without_an_item_drop_definition_breaking_drops_nothing() {
    let mut sim = quarry(false);
    break_slab(&mut sim);
    assert!(sim.items().is_empty());
}

#[test]
fn the_base_game_drops_dirt_from_grass_and_stone_from_its_slab() {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mods: [&dyn Mod; 3] = [&data, &base::BaseMod, &base_movement::BaseMovementMod];
    let sim = Simulation::with_mods(&mods).unwrap();
    let blocks = sim.blocks();
    let drop = |name: &str| blocks.drop(blocks.id(name).unwrap()).map(|d| (blocks.material_name(d.material), d.amount));
    assert_eq!(drop(base::GRASS), Some(("base:dirt", FULL_VOLUME)));
    assert_eq!(drop(base::STONE_SLAB), Some(("base:stone", FULL_VOLUME / 2)));
    assert_eq!(drop(base::STONE_STAIRS), Some(("base:stone", FULL_VOLUME * 3 / 4)));
    assert!(sim.item_drop_def().is_some());
}
