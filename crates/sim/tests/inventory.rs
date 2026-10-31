//! What the player carries: placing uses it up, picking up fills it as far as it has room, and
//! the drop key throws it.

use api::{
    ApiError, BlockBox, BlockDef, BoxelPos, I64Vec3, InteractionDef, InventoryDef, ItemDropDef, Mod, Orientation,
    PlayerDef, Registrar, Vec3, WorldEdit, WorldPos,
};
use sim::{BlockId, FULL_VOLUME, MaterialId, PlayerInput, SimError, Simulation};

/// A stone floor with a stone slab two boxels east of spawn, item drops, and an inventory if
/// one is given.
struct Yard {
    inventory: Option<InventoryDef>,
}

impl Mod for Yard {
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
        if let Some(def) = &self.inventory {
            r.register_inventory(def.clone())?;
        }
        Ok(())
    }

    fn build_world(&self, w: &mut dyn WorldEdit) -> Result<(), ApiError> {
        w.fill(BoxelPos::new(-8, -8, -1), BoxelPos::new(10, 8, -1), "test:stone")?;
        w.add_block(BoxelPos::new(2, 0, 0), "test:slab", Orientation::IDENTITY)?;
        w.set_spawn(WorldPos::new(I64Vec3::ZERO, Vec3::new(0.5, 0.5, 0.0)));
        Ok(())
    }
}

/// Room for `capacity` boxels, starting with `stone` boxels of stone.
fn inventory(capacity: f32, stone: f32) -> InventoryDef {
    InventoryDef { capacity, throw_speed: 6.0, pickup_delay: 1.0, start: vec![("test:stone".into(), stone)] }
}

fn yard(inventory: Option<InventoryDef>) -> Simulation {
    let mut sim = Simulation::with_mods(&[&Yard { inventory }]).unwrap();
    sim.spawn_player().unwrap();
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
    sim
}

fn stone(sim: &Simulation) -> MaterialId {
    sim.blocks().material("test:stone").unwrap()
}

fn block(sim: &Simulation, name: &str) -> BlockId {
    sim.blocks().id(name).unwrap()
}

/// The yaw and pitch that look from the player's eye at `point` in the boxel at `cell`.
fn aim(sim: &Simulation, cell: [i64; 3], point: [f32; 3]) -> (f32, f32) {
    let to = sim.player().unwrap().eye().delta_to(WorldPos::new(I64Vec3::from_array(cell), Vec3::from_array(point)));
    (to.y.atan2(to.x), to.z.atan2(to.truncate().length()))
}

/// Places `block` on top of the floor boxel at `x`, `y`. Returns whether it went down.
fn place_on_floor(sim: &mut Simulation, x: i64, y: i64, block: BlockId) -> bool {
    let (yaw, pitch) = aim(sim, [x, y, -1], [0.5, 0.5, 1.0]);
    let hit = sim.target(yaw, pitch).expect("the floor is in reach");
    let ghost = sim.placement(&hit, block, yaw, pitch, false).expect("somewhere to place");
    sim.step(&PlayerInput { yaw, pitch, alt_pressed: true, alt_held: true, place_block: block, ..Default::default() });
    let placed = sim.universe().main().cell(BoxelPos::new(x, y, 0)).iter().any(|o| o.block == block);
    assert_eq!(placed, ghost.allowed, "the ghost said what would happen");
    placed
}

#[test]
fn placing_uses_up_the_shapes_volume_and_is_refused_when_short() {
    let mut sim = yard(Some(inventory(8.0, 1.0)));
    let (stone, slab, full) = (stone(&sim), block(&sim, "test:slab"), block(&sim, "test:stone"));
    assert_eq!(sim.inventory().amount(stone), FULL_VOLUME);

    assert!(place_on_floor(&mut sim, -1, 0, slab));
    assert_eq!(sim.inventory().amount(stone), FULL_VOLUME / 2, "a slab costs half");
    assert!(!place_on_floor(&mut sim, 0, -1, full), "a full block costs more than is left");
    assert_eq!(sim.inventory().amount(stone), FULL_VOLUME / 2, "a refused placement costs nothing");
    assert!(place_on_floor(&mut sim, 0, -1, slab));
    assert_eq!(sim.inventory().amount(stone), 0);
    assert!(!place_on_floor(&mut sim, 0, 1, slab), "nothing left");
}

#[test]
fn without_an_inventory_placing_is_free() {
    let mut sim = yard(None);
    assert!(sim.capacity().is_none());
    assert_eq!(sim.inventory().total(), 0);
    let full = block(&sim, "test:stone");
    assert!(place_on_floor(&mut sim, -1, 0, full));
    assert!(place_on_floor(&mut sim, 0, -1, full));
}

/// Breaks the slab two boxels east, then walks over until what it dropped is picked up.
fn break_slab_and_collect(sim: &mut Simulation) {
    let (yaw, pitch) = aim(sim, [2, 0, 0], [0.5, 0.5, 0.5]);
    sim.step(&PlayerInput { yaw, pitch, use_pressed: true, use_held: true, ..Default::default() });
    assert!(sim.universe().main().block(BoxelPos::new(2, 0, 0)).is_empty(), "the slab broke");
    for tick in 0..120 {
        let movement = if tick < 15 { api::Vec2::Y } else { api::Vec2::ZERO };
        sim.step(&PlayerInput { movement, ..Default::default() });
    }
}

#[test]
fn breaking_gives_back_what_placing_used_up() {
    let mut sim = yard(Some(inventory(8.0, 1.0)));
    let stone = stone(&sim);
    break_slab_and_collect(&mut sim);
    assert!(sim.items().is_empty(), "picked up");
    assert_eq!(sim.inventory().amount(stone), FULL_VOLUME * 3 / 2);

    // Place a slab where the old one was and break it again: the total comes back.
    let slab = block(&sim, "test:slab");
    sim.teleport_player(WorldPos::new(I64Vec3::ZERO, Vec3::new(0.5, 0.5, 0.0)));
    sim.step(&PlayerInput::default());
    assert!(place_on_floor(&mut sim, 2, 0, slab));
    assert_eq!(sim.inventory().amount(stone), FULL_VOLUME);
    break_slab_and_collect(&mut sim);
    assert_eq!(sim.inventory().amount(stone), FULL_VOLUME * 3 / 2);
}

#[test]
fn when_full_the_rest_stays_on_the_ground_and_is_not_pulled() {
    let mut sim = yard(Some(inventory(2.0, 1.0)));
    let stone = stone(&sim);
    // Two boxels' worth at the player's feet, with room for one.
    let feet = sim.player().unwrap().pos();
    sim.spawn_item(stone, 2 * FULL_VOLUME, feet.translate(Vec3::new(0.0, 0.0, 0.125)));
    sim.step(&PlayerInput::default());
    assert_eq!(sim.inventory().amount(stone), 2 * FULL_VOLUME, "took what fit");
    assert_eq!(sim.inventory().room(sim.capacity()), 0);
    let left = sim.items().iter().next().expect("the rest stays").clone();
    assert_eq!(left.drop.amount, FULL_VOLUME);

    // Another item within pull range isn't pulled toward a full player.
    let near = feet.translate(Vec3::new(1.5, 0.0, 0.125));
    sim.spawn_item(stone, FULL_VOLUME / 2, near);
    for _ in 0..60 {
        sim.step(&PlayerInput::default());
    }
    assert_eq!(sim.items().len(), 2);
    assert!(sim.items().iter().all(|i| !i.pulled));

    // Placing makes room, and the pull starts again.
    let slab = block(&sim, "test:slab");
    assert!(place_on_floor(&mut sim, -1, 0, slab));
    for _ in 0..60 {
        sim.step(&PlayerInput::default());
    }
    assert_eq!(sim.inventory().total(), 2 * FULL_VOLUME, "full again");
}

#[test]
fn the_drop_key_throws_a_block_that_comes_back_after_the_delay() {
    let mut sim = yard(Some(inventory(8.0, 1.5)));
    let stone = stone(&sim);
    // Straight up, so it falls back onto the player.
    let up = 89f32.to_radians();
    let throw = PlayerInput { pitch: up, drop_pressed: true, drop_held: true, drop_material: Some(stone), ..Default::default() };
    sim.step(&throw);
    assert_eq!(sim.inventory().amount(stone), FULL_VOLUME / 2, "a full block's worth left");
    let item = sim.items().iter().next().expect("thrown").clone();
    assert_eq!(item.drop.amount, FULL_VOLUME);
    assert!(item.velocity.z > 5.0, "thrown along the look");

    // It lands at the player's feet but isn't picked up until the delay is over.
    let delay = sim.inventory_def().unwrap().pickup_delay;
    let ticks = (delay * Simulation::TICK_RATE as f32) as usize;
    for _ in 1..ticks - 1 {
        sim.step(&PlayerInput::default());
    }
    assert_eq!(sim.items().len(), 1, "not yet");
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
    assert!(sim.items().is_empty(), "picked up again");
    assert_eq!(sim.inventory().amount(stone), FULL_VOLUME * 3 / 2);
}

#[test]
fn holding_drop_keeps_throwing_until_none_is_left() {
    let mut sim = yard(Some(inventory(8.0, 1.5)));
    let stone = stone(&sim);
    let ahead = PlayerInput { drop_held: true, drop_material: Some(stone), ..Default::default() };
    sim.step(&PlayerInput { drop_pressed: true, ..ahead });
    for _ in 0..15 {
        sim.step(&ahead);
    }
    assert_eq!(sim.inventory().amount(stone), 0, "a full block, then the half left");
    let thrown: u32 = sim.items().iter().map(|i| i.drop.amount).sum();
    assert_eq!(thrown, FULL_VOLUME * 3 / 2);
    // Nothing held, nothing thrown.
    sim.step(&PlayerInput { drop_pressed: true, drop_held: true, ..Default::default() });
}

#[test]
fn the_start_names_a_material_some_block_is_made_of() {
    let def = InventoryDef { start: vec![("test:clay".into(), 1.0)], ..inventory(8.0, 0.0) };
    let err = Simulation::with_mods(&[&Yard { inventory: Some(def) }]).err().unwrap();
    assert_eq!(err, SimError::Api(ApiError::UnknownMaterial("test:clay".into())));
    let over = Simulation::with_mods(&[&Yard { inventory: Some(inventory(1.0, 2.0)) }]).err().unwrap();
    assert!(matches!(over, SimError::Api(ApiError::InvalidDef(_))), "{over}");
}

#[test]
fn the_base_game_starts_with_brick_and_wood_and_charges_for_shapes() {
    let data = data_mods::load(std::path::Path::new(base::DATA_DIR)).unwrap();
    let mods: [&dyn Mod; 3] = [&data, &base::BaseMod, &base_movement::BaseMovementMod];
    let mut sim = Simulation::with_mods(&mods).unwrap();
    sim.spawn_player().unwrap();
    let blocks = sim.blocks();
    let (brick, wood) = (blocks.material("base:brick").unwrap(), blocks.material("base:wood").unwrap());
    assert_eq!(sim.inventory().amount(brick), 16 * FULL_VOLUME);
    assert_eq!(sim.inventory().amount(wood), 16 * FULL_VOLUME);
    assert_eq!(sim.capacity(), Some(64 * FULL_VOLUME));

    // A brick slab on the floor beside the spawn costs half a brick.
    let slab = block(&sim, base::BRICK_SLAB);
    for _ in 0..30 {
        sim.step(&PlayerInput::default());
    }
    assert!(place_on_floor(&mut sim, 2, 2, slab));
    assert_eq!(sim.inventory().amount(brick), 16 * FULL_VOLUME - FULL_VOLUME / 2);
}
