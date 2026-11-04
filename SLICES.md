# Roadmap

Work happens in slices. Each slice goes through the stages in `CLAUDE.md`: **Not started → Plan → Implement → Test → Verify → Done.**

There are three phases. The middle one is the longest and the point of the early project: get something playable, then improve game feel one toy feature at a time. The mod API grows alongside the toys rather than coming at the end.

---

## Phase 1: Something playable

One slice: just enough engine to walk around in and start playing.

### Slice 1: Minimum playable engine

**Stage:** Done (2026-10-25)

**Goal:** You can walk around a small boxel test area.

**Scope**
- Cargo workspace with simulation and rendering in separate crates.
- An `api` crate (plain Rust traits and registries) and a `base` crate that registers all content only through it. No WebAssembly, no file loading, no stability promises yet.
- Storage and addressing shaped for later features without building them: the world is one grid among many, and chunks hold separate layers.
- Floating origin: positions are an integer cell plus a small offset, and rendering is relative to an origin that recenters near the camera.
- 32×32×32 chunks, a world made of 3D chunks, Z-up coordinates.
- Block registry by name.
- A hand-built test area: a flat floor plus a few fixed shapes (steps, walls, a pit). No world generation.
- Bevy window, culled meshing.
- A basic walking player with collision, jumping and gravity, plus a fly mode for getting around.
- Fixed 60 Hz simulation tick, rendering interpolated between ticks.

**Out of scope:** Anything the first toy doesn't need, including world generation, saving, greedy meshing, lighting, LOD, sub-grids, mods and multiplayer.

**Done when**
- You can walk and fly around a test area that spans several chunks.
- Chunk storage, coordinate conversion and meshing have tests.
- The Y-up conversion happens only at the render boundary.
- Walking at coordinates in the millions looks and feels the same as near zero (a test, plus a teleport to check it in game).
- The engine crates contain no block definitions; they all come from `base` through `api`.
- Movement doesn't jitter at frame rates above or below 60.

**Approved plan** (2026-10-25)

*Decisions:* Bevy 0.18.1 (the version best known; upgrade later). `sim` is plain Rust with no Bevy. Blocks use flat colors this slice; Minetest textures come with the data-mods seam.

*Crates*
- `crates/api`: plain types and traits mods see. Depends only on glam (pinned to Bevy's version).
- `crates/sim`: world storage, registry, 60 Hz tick, player physics. No Bevy.
- `crates/client`: Bevy window, input, meshing, render origin, camera, interpolation.
- `crates/base`: content mod, depends only on `api`.
- `crates/game`: binary that wires `base` into `sim` and `client`. Engine crates never depend on `base`.

*Key types*
- Positions (`api`): `BoxelPos(I64Vec3)`, `ChunkPos(I64Vec3)`, `LocalPos` (0..32), `WorldPos { cell: I64Vec3, offset: Vec3 }` with the offset kept in [0,1). All movement goes through `WorldPos::translate`, which carries whole units into the cell.
- Addressing: `GridId`, `Universe { grids }`, `Grid { chunks: HashMap<ChunkPos, Chunk> }`, `Chunk { blocks: Layer<BlockId> }`, where `Layer<T>` is `Uniform(T)` or `Dense(Box<[T; 32768]>)`.
- Registry by name (`"base:stone"`). `BlockDef { name, solid, color }`. Runtime `BlockId(u16)` is assigned at startup and isn't identity. `BlockId::EMPTY` is reserved and isn't a block definition.
- Mod API: `trait Mod { fn register(&self, r: &mut Registrar); fn build_world(&self, w: &mut dyn WorldEdit); }`. `base` registers stone, dirt, grass and a marker, then builds the test area via `WorldEdit::set_block(BoxelPos, &str)`.
- Player parameters (added 2026-10-25, before implementation): `api` defines `PlayerDef { size, eye_height, walk_speed, jump_height, gravity, fly_speed }` in real units (meters, seconds), and `base` registers it through `Registrar`. The movement code stays in `sim`, but every number comes from the mod. `sim` refuses to start a player if no `PlayerDef` is registered. Swappable movement behavior is deferred to the WebAssembly seam.
- Test area: about 128×128 floor across several chunks (including vertical chunk boundaries), with steps, walls and a pit. A second copy at about (3,000,000, −2,000,000, 1,000) is the teleport target.

*Simulation:* `Simulation::step(&PlayerInput)` at a fixed 60 Hz, driven from Bevy's `FixedUpdate`. The player uses axis-separated swept collision, gravity, jumping and a fly toggle (collision stays on). `base`'s `PlayerDef` starts at a 0.6×0.6×1.8 box, eyes at 1.62 and a jump of about 1.25 boxels, placeholders for the movement toy to tune.

*Client*
- `client::coords` is the only place Z-up converts to Y-up: `(x, y, z) → (x, z, −y)`.
- `RenderOrigin { cell }` per world view recenters when the camera is more than about 256 boxels away. Meshes are chunk-local, so recentering only updates transforms.
- Culled meshing is a pure function of a chunk plus its 6 neighbors, producing plain vertex arrays (with per-face shading) that then become a Bevy mesh.
- Interpolation renders `prev + (curr − prev) · overstep_fraction`, computed as a small delta. Mouse look is applied per frame.
- Debug keys: fly toggle, teleport between the test areas, and a frame-cap cycle (30/60/144/uncapped).

*Tests:* unit tests for coordinates (negatives, boundaries, carry, millions), layer promotion, the registry, the mesher (6 and 10 faces, cross-chunk culling, empty chunk), physics (landing, walls, jump height, pit) and Y-up conversion. A precision test checks that the same inputs at the origin and at the far copy give bit-identical relative positions. Integration tests check that `base` in a headless `Simulation` makes the player settle on the floor, and that `Simulation` has zero block definitions and no `PlayerDef` with no mod loaded.

*Implementation notes* (small additions within the plan, no change of course)
- `Mod` also has `name()`. `WorldEdit` also has `clear_block`, `fill`/`clear` helpers, `set_spawn` and `add_waypoint`; the spawn point and teleport targets are content, so `base` supplies them. The debug teleport (T) cycles through the waypoints.
- `base` registers a fifth block, `base:brick`, for the steps and walls. Chunk borders show as stone stripes in the floor.
- Jumps integrate gravity with the average of the old and new vertical speed, so the peak matches `jump_height` exactly.
- Chunk material is unlit, with per-face shading baked into the vertex colors until lighting exists.
- Debug keys: F fly, T teleport, F3 frame cap (vsync/30/60/144/none), Esc releases the mouse, click grabs it.
- Added during Verify, outside the plan (small dev convenience): key bindings can be overridden per player in `~/.config/boxel/controls.txt` (or `$BOXEL_CONTROLS`), outside the repo. Defaults stay WASD. See `crates/client/src/controls.rs`.

*Verification* (2026-10-25)
- Walk and fly across several chunks: met (played).
- Tests for chunk storage, coordinates and meshing: met (49 tests across the workspace).
- Y-up conversion only at the render boundary: met. The axis swap is only in `client::coords`; `api` and `sim` don't depend on Bevy.
- Same feel in the millions: met. A bit-identical scripted walk test, and the in-game teleport played fine.
- No block definitions in engine crates: met. A test checks the engine alone has no blocks or `PlayerDef`; `base` depends only on `api`.
- No jitter above or below 60: met. Note: 30 fps felt worse than usual to the player. A code review found no interpolation or pacing bug (the camera blends the last two ticks by overstep fraction, and `prev_pos` updates every tick even with two ticks per frame), so it's most likely just the lower frame rate plus the one tick of display delay interpolation always adds. Revisit if it still feels off during the movement toy.

*Deferred:* step-up, noclip, remeshing after block edits (blocks don't change yet), streaming and unloading chunks (everything is meshed at startup), a lighting system (shading is baked into vertex colors).

*API debt:* none. Spawn point, waypoints and player parameters all go through `api`.

---

## Phase 2: Play with game feel

A run of small toy slices. Each one takes a single game-feel feature that doesn't need much else, makes it playable, and tunes it until it feels right. We pick the next toy together when the current one is done. The list below is a backlog, not a fixed order.

**Rules for this phase**
- **Infrastructure arrives when a toy needs it.** World generation comes when a toy needs more space or variety than the test area. Once world generation exists, we start thinking about LOD. Saving comes when losing your work between sessions gets in the way. Each of these becomes its own slice at that point.
- **Toys go through the API.** When a toy needs something the API can't express, extend the API. A shortcut around it is allowed to keep a toy moving, but it goes in the API debt list below.
- **Keep the API WebAssembly-friendly.** The API is plain Rust for now, but it has to become a WebAssembly boundary. Calls exchange plain data: no references into engine or mod memory, no closures, no generics in the API's traits.
- **Seam slices along the way.** Every few toys, or when the API debt list gets long, run a short slice that pays down debt or adds one real mod feature. The seam backlog below lists the candidates.
- **Tuning beats polish.** Values should be adjustable while the game runs where that helps.
- **Tests cover the logic, playing covers the feel.** Verification for a toy is mainly the user playing it and deciding whether it feels right.

### Current toy

**Waiting for a play check** (remind the user at the start of every session, and at good moments during one, until they confirm; then delete the line): none.

**Low-priority play checks** (don't remind every session; check only when a toy makes it convenient, such as one that puts the player among moving cows; delete the line once seen):
- Cow herd (1): a cow shoved well away from the herd walks back to it on its own.
- Cow herd (3): a moving cow that runs into the player pushes them back, with no jitter or sticking.

#### Fighting cow (branch `toy-fighting-cow`)

**Stage:** Verify. Plan approved 2026-11-03; the user engaged with the recommended answers to the open questions.

**Goal:** a cow you hit fights back. Close in it kicks (head down, then flung up) for low damage, tossing the player up and away to make space; from far off it charges for high damage and big knockback. The player has health, the guard does something, and getting out of reach is a way out.

*Plan*
- **Pathfinding: not needed** (asked first). The fight happens in the flat pasture, where steering straight at the player works. A player on a block or over the fence is out of reach on purpose: the cow gives up after a while. Real pathfinding waits for terrain to get around (world generation, caves) and would be an engine service, its own infrastructure slice.
- **API (`api::brain`).** `BrainOutput.attack: Option<Attack>`: a box in the creature's own space (turned with its facing, like hit boxes), damage, knockback (level away, and up), and what gets through a guard (shares of damage and knockback). Live while the brain keeps sending it; the engine hits the player at most once until it stops. `BrainContext.attack_landed`.
- **API (`api::health`).** *Changed from the plan's "`PlayerDef` gains fields":* a `HealthDef` the mod registers, like `CrowdDef`: health, regen per second, regen delay and the guard's angle. Without one the player can't be hurt.
- **Engine (`sim`).** Live attacks tested against the player's body after targets move. A hit takes health and knocks the player back through the impulse path (speed toward the attacker, or falling, doesn't eat into it). Guarding and facing the attacker (within the guard angle) lets through only the attack's guarded shares. Out of health, the player goes back to spawn, whole; a panel switch makes them unkillable. Health comes back after a delay without hits. `PlayerHurt` events for the client.
- **Base (`base::cow`).** A hit cow fights instead of fleeing (a `fight_share` tunable, by ID; default all), shows `"alarmed"` for a moment so the herd still stampedes, then `"angry"`. Modes: angry (close in, or catch its breath), kick (within about 2.4 m: head down with the `punch` clip, then the hit as the head flings up), charge-up (head down, the start of `eat`), charge (fast, bends only a little toward the player; ends on a hit, overshooting or timing out) and stunned (charged into a wall or the fence). Gives up when it hasn't got closer for a few seconds; anger fades a while after the last hit. Everything tunable.
- **Client.** Health bar, a red flash, view jolt and shake, a low moo and a readout when hit; player health sliders, "can't be knocked out" and Heal in the panel.
- **Tests.** Brain: hit gives angry, close gives kick, far gives charge, a charge into a wall stuns, unreachable gives up, anger fades. Sim: an attack hits once, misses outside its box, knocks back, the guard softens it, knocked out sends you to spawn, regen waits. Base world: a standing player gets charged and thrown; kicks land up close; a player on a 2-high block makes the cow give up; a dodged charge into the fence stuns.
- **Decisions (open questions, at the recommendations):** every hit cow fights, the rest stampede; knocked out respawns at spawn, plus the unkillable switch; the guard blocks most of a kick (75% damage, 50% knockback) but only halves a charge; a missed charge into the fence stuns; the kick uses the `punch` clip (checked in the file: the neck lowers over 0.5 s, then snaps up over 0.25 s).
- **Out of scope:** pathfinding, line of sight, hit boxes following the animated head (cow attacks use their own boxes), the herd charging together, fall damage, the heal-on-kill loop, the rest of Combat depth.

*Built*
- As planned, with `HealthDef` in place of new `PlayerDef` fields (noted above). `api::Attack`, `BrainOutput.attack`, `BrainContext.attack_landed`, `api::HealthDef`, `Registrar::register_health`, `ApiError::DuplicateHealthDef`.
- `sim`: targets keep their live attack; after they move, each one not yet spent is tested against the player's body (a level circle and a height) in the attacker's turned space, and a hit becomes a `Blow`. The simulation applies blows last in the tick: the guard, damage, knockback through `apply_impulse`, knock-out back to spawn, regen. `Simulation::{health_def, health_def_mut, health, set_health, unkillable, set_unkillable, take_player_hurts}`, `PlayerHurt` events.
- `base::cow`: five new modes (angry, kick, charge-up, charge, stunned) and three memory slots (anger, stuck, best). 25 new tunables, all in the Brains panel. Base values: health 20, regen 1 per second after 4 s, guard within 60°; kick 2 damage, push 6, lift 9, within 2.4 m; charge from 6 m, 0.9 s wind-up, 7 m/s for up to 2.5 s, turning 0.8 rad/s, 7 damage, push 16, lift 6; stunned 2 s; gives up after 4 s with no progress; anger lasts 20 s from the last hit. A charge from about 20 m away can run out of time a metre short of the fence; the cow then gives up there.
- Client: `client::hurt` (health bar at the top of the screen, red flash, readout "HURT -7", "GUARDED -2" or "KNOCKED OUT", the attacker's sound low), hurt juice values in the combat feel, and a "Player health" panel section with "Can't be knocked out" and Heal.
- Tests: 443 pass across the workspace (1 ignored, as before). Clippy is clean natively and for `wasm32`. New: health def (2); the cow brain (10: squaring up alarmed then angry, who fights, the kick's timing and single strike, the charge's wind-up and run, a charge's limited turn, ending on a hit, stunning when blocked, overshooting, giving up when unreachable, anger fading, being hit while fighting); `sim/tests/fighting.rs` with a scripted attacker (10: one hit per live attack, again after a gap, knockback away and up, running in doesn't eat into it, misses outside the box, guard facing and facing away, knock-out and unkillable, regen delay, no health def, duplicate def); base world (3: a kick up close tosses up and away, a charge from 10 m hits hard, a cow lured to the fence charges into it, is stunned and gives up). Two old tests now use non-fighting cows (`fight_share` 0), since they're about fleeing. The brain tests caught a bug in the give-up timer (it counted slow progress as none), fixed before closing the step. The game starts and runs cleanly.
- *API debt (new):* health bar in the client. Mods can't change how health is shown.

*Plan change at Verify* (2026-11-03, the user's requests before playing: "can you add fall damage", "ok rename", "can cows have some kind of behavior where they can chose to fight for a injured riveal or flee", then "engage" without answering which of these belong in this toy; all three were taken into it, and the real hind-leg kick went to the backlog).
- **Kick renamed toss.** The model has no leg kick (its clips are `stand`, `walk`, `run`, `eat`, `punch`, `die`); the move is the head flung up, so the mode and its sliders are `toss_*` now. A real hind-leg kick is in the backlog.
- **Fall damage.** `HealthDef::{fall_speed, fall_damage}`: landing faster than `fall_speed` costs `fall_damage` per m/s over. Base: from 16.5 m/s (about 4 blocks with the controller's fall gravity of 35), 1 per m/s, so 10 blocks costs about half your health. No guard; it can knock you out. With fall damage, the camera's hard landing bottoms out at `fall_speed` instead of its own `hard_landing_speed` (20), so the robot's shock absorber says when a fall hurt (the design doc's robotic player). `PlayerHurt.attacker` is `None` for falls; the readout says "FALL -5".
- **Fight or flee for a herd member in trouble.** `Neighbor.health` (a share of full health) in the API. Each cow has a boldness fixed by its ID (0 to 1). A cow that sees another in trouble within `defend_range` (8 m), meaning fighting (`"angry"`) or hurt and alarmed, chooses once: stand up for it (angry, toss and charge like a hit cow) or run. The chance: `defend_chance` (0.25) × 2 × boldness, plus `courage` (0.15) per cow it sees fighting, plus `injury_pull` (0.5) × how badly the one in trouble is hurt, minus how badly it's hurt itself. A cow that came to help shows `"angry"` from the start, with no alarm, so it doesn't scatter the herd. A fighting cow under `flee_health` (30%) with nobody fighting beside it breaks off and runs, and one hit while that hurt runs instead of turning. `fight_share` now picks the boldest cows. New `chase_range` (20 m): further off, a fighting cow loses interest, so a helper doesn't chase a player who's left.
- *Note:* every cow near a hit one decides on the same tick, before any helper shows, so courage only sways later choices (a cow whose calm ran out, or one coming near a fight). If herds feel too timid or too keen, `defend_chance` is the slider; a short hesitation before choosing would let courage spread from the start.
- *Tests:* 452 pass (1 ignored, as before), clippy clean natively and for `wasm32`. New: fall damage (3: a short drop doesn't hurt, a long one hurts by how far past the limit, a fall can knock you out unless unkillable); neighbors' health (in the existing neighbors test); the brain (5: bold cows help and timid ones run, courage, a badly hurt one and its own health sway the choice, a helper is angry from the start, badly hurt and alone it breaks off but not with others beside it, losing interest far off); base world (1: hitting one cow brings some of the herd to its side and scatters the rest, and they lose interest once the player leaves). The timid test herd also turns defending off. The game starts and runs cleanly.

*Second round at Verify* (2026-11-03, engaged): the user's tuned values become the defaults ("add my values as the defualts we will tune it"); killed cows only come back when the player is knocked out ("I want killing a herd to feel challenging for me"); the jump-lift fix.
- **Defaults** from the user's printout: cow health 130 (in `targets.toml`); brain walk 1.5, run 8, notice 7.5, spook 5.3, courage 0.35, injury pull 0.85, anger 29, give up 6, chase 27, approach 2.6; toss push 11, lift 13.5, guarded 0.3 and 0.82; charge speed 15, damage 13, push 12, lift 14.5, guarded push 0.9. Everything else in the printout already matched. With these, a full charge plus its landing does about 13.7 (13, plus about 0.7 from landing at 17.2 m/s); guarded, 6.5 and no fall damage; a toss does 2 and lands just under the fall line.
- **Killed cows stay dead.** `TargetDef.respawns` (`respawns = false` in `targets.toml`, true if left out): without it a broken creature stays gone. `HealthDef.knock_out_resets`: a knock-out, from a blow or a fall, puts every creature back at its spot, whole and calm (base: on). Unkillable means no knock-out, so no reset; F6 still resets by hand.
- **Jump-lift fix.** A blow's lift brings the player's upward speed up to it instead of adding to it, so a blow mid-jump throws no higher than one standing. Before, a charge clipping a rising jump threw the player about 10 m up, about 10 fall damage on top, a knock-out.
- **Found while testing, not changed:** with run speed 8 and spook time 5.3, a stampede carries cows about 40 m, so the herd splits into groups further apart than they see each other (the crowd's `sight`, 12 m), and each group keeps to itself: in a trace, 3 cows and 5 cows stayed in opposite corners of the pasture for two minutes. Options if it matters in play: raise `sight` (the pasture is 30 m), shorten `spook_time`, or let strays look further for the herd. The straying-cow test uses the old stampede values (run 3, spook 2) so it still checks rejoining.
- *Tests:* 454 pass (1 ignored, as before), clippy clean natively and for `wasm32`. New: a blow mid-jump throws no higher than one standing; a knock-out puts every creature back only when the health def says so; a broken cow stays gone past its respawn time and a fall knock-out brings the whole herd back whole (replacing the old breaks-and-comes-back test). The data-files test edits the new health. The game starts and runs cleanly.

*Third round at Verify* (2026-11-03, engaged): the user's idea, "can hitting on the way down significantly reduce fall damage, maybe a damage scaling push back". Recoil opposite the look (the recommendation; the alternative was always straight up).
- `ToolDef::{recoil, recoil_bounce}`: a hit while the player is in the air (not flying) pushes them back opposite their look by `recoil` m/s per point of damage dealt, but never past leaving them rising at `recoil_bounce`. Hits on the ground are unchanged. Head hits and speed carried into the swing add damage, so they add recoil. `Strike.recoil` reports it. Base club: 0.8 per damage, bounce 3 m/s: a 20-damage head hit carried in at 20 m/s takes 16 m/s off a fall, enough to land a 10-block drop without damage.
- *Tests:* 458 pass (1 ignored, as before), clippy clean natively and for `wasm32`. New (in `fighting.rs`): a hit on the way down pushes back from the look by the damage; recoil never leaves the player rising faster than the bounce; a hit on the ground gives none; swinging down onto something on the way down cuts the fall damage by more than half, and without recoil it doesn't help. The game starts and runs cleanly.

*Done when:* (1) hitting a cow makes it fight back while the rest of the herd stampedes; (2) up close it tosses, head down then up, for a little damage, tossing the player up and away; (3) from far off it lowers its head and charges, for big damage and knockback, and a sidestep can dodge it; (4) a dodged charge into the fence stuns it; (5) getting out of reach (on a block, over the fence) makes it give up, and its anger fades; (6) the guard softens tosses more than charges; (7) health shows, comes back, and running out sends you to spawn; (8) the wind-ups read clearly in play; (9) tests pass and clippy is clean; (10) falls from about 4 blocks hurt, more the further, and the camera bottoms out when they do; (11) other cows near a hurt one sometimes come to its side and sometimes run, and a badly hurt cow alone breaks off; (12) killed cows stay dead until a knock-out, which brings the whole herd back; (13) swinging down onto a cow on the way down breaks a fall.

### Toy backlog

- ~~**Block shapes and placement.**~~ Now the current toy, as Block shapes and shared cells. Suggested order after it: inventory and items, then interaction controls and block interactions, then combat feel.
- ~~**Inventory and items.**~~ Done (2026-11-01), see Done toys. Added 2026-10-28, from the user's request that led to the block shapes toy. Real items and counts, breaking gives you the block, and the design doc's "inventory discourages variety" gripe: try limiting by weight or volume instead of slots, or carrying a material and choosing its shape when placing. Suggested next after block shapes, before combat feel. *Updated 2026-10-30:* Loose items was split out of this and is done: breaking drops materials by volume and the player collects them into a counter. What's left: placing uses up material, a shared capacity limit (volume first, maybe weight later), picking a material and then a shape, a drop key, and items staying on the ground when you're full. The plan proposed on 2026-10-29 had these; reuse it.
- **Rethink the inventory.** Added 2026-11-02 at the user's request, during Emergent shapes: "the last toy really was horrible… I was planning on just removing it but got talked out and now every problem with this one is caused by it". The Inventory and items toy's capacity limit and paying for things by volume caused each problem found while playing Emergent shapes: patterns made from parts refused once plain wood ran out, crafting needed room to carry, and dirt couldn't be used. Crafting was made free and an unlimited carry was proposed in that toy. Decide whether a carrying limit or costs belong in the game at all before building on them again. If something like them comes back, it should be because a toy needs it.
- **Covered blocks.** Added 2026-10-31, deferred from Solid materials. Should blocks be roughly one material, with grass and snow a cover on a block rather than part of it? What should a cut through grass show: honest (a bottom slab is all dirt, since the look spans the boxel) or anchored (the look's top follows each box's top, so a grass slab has grass on top, at the cost of seams between a stair's steps)? Settle it once something needs to cut grass, like world generation or crafted shapes.
- **Lower contrast in the other looks.** Added 2026-11-02 at the user's request, from Wood and brick looks: the lower contrast given to bricks and wood "should eventually be added elsewhere" (stone, dirt, gravel, ice and so on). Mostly recipe numbers in `blocks.toml`; `noise` may want a speckle-like setting.
- ~~**Ambient occlusion.**~~ Done (2026-11-02), see Done toys. Flood-filled light levels remain an idea for later. Added 2026-11-02 at the user's request, after Wood and brick looks. Darken inside corners and edges so shapes read at a glance (the brick and wood house's windows, steps and wall bases). Baked per vertex in the mesher, like Minecraft's Smooth Lighting: nearly free each frame and fine on WebGL2, unlike Bevy's SSAO (several ms on integrated GPUs, no WebGL2). The extra work is asking about neighbors in sixteenths, since shapes aren't always full blocks; faces with different AO merge less. Later, maybe flood-fill light levels (sky and torches, baked the same way) for dark caves; shadow-mapped lights and global illumination cost far more.
- **Plank board ends.** *2026-11-02:* Wood and brick looks' woven planks may settle this, since east and west faces now show boards too; check once played. Added 2026-10-31, from Solid materials. Planks stay lined up with the world, so a wood panel or stair facing east or west shows a grid of board ends instead of boards. The user dislikes it but accepted it as a compromise for now. Options: a pattern that reads as boards along both level axes, letting a material turn only about the up axis, or picking the board direction per block when placed.
- **Items inside placed blocks.** Added 2026-10-30, from Loose items. A block placed over a resting item hides it inside the block; it can still be picked up. Push items out the shortest way, or refuse the placement, once it gets in the way.
- **Place on left click, break on right click.** Added 2026-11-01 at the user's request, from Inventory and items: "to keep controls consistent, placing should be left click and breaking should be right", so a block's main use is placing, as a tool's is swinging. Retire the old `break`/`place` names in the controls file (they'd bind the wrong action) and move the user's lines to `use`/`alt_use`. Revises the Skyrim-style controls decision. Fits with Generative crafting or Interaction controls.
- **Crouch to collect.** Added 2026-11-01 at the user's request: "your a little robot and crouching sucks the items in". Items are only pulled and picked up while crouching; walking over them does nothing. Proposed: an `ItemDropDef` flag the base mod turns on, a somewhat wider pull (about 3 m) and a shorter throw delay (about 0.5 s) since throwing and crouching rarely overlap, and maybe a visible suck-in later. Open: whether flying with the shared Shift key vacuums too.
- **Interaction controls.** Split out of Block editing (2026-10-25). Fix Minecraft's ambiguous right click once there are things to use, like crystals or devices. Principles: one input, one action (no catch-all "use" button that changes meaning with what you hold or look at), and what you see is what you get (a ghost of the result, red when not allowed).
  - *Added 2026-10-28:* decide how players reach shared boxels. Placing into an occupied boxel makes placement harder to read (every click has two possible boxels), so it may become a deliberate trick (a key, a tool, or a property of blocks like water) instead of happening on any click. Decide once combat and usable items show how crowded the controls get; the engine keeps full support either way. Also from play: two slabs at right angles make a stair, so a separate stairs block may not be needed.
- **Block interactions.** Unstable crystals: two block types, a touch trigger, a directional blast, and a way to push blocks so devices are possible.
- ~~**Emergent shapes from crafting.**~~ Done (2026-11-02), see Done toys.
- **Crafting follow-ups.** Added 2026-11-02 at the user's request, from closing Emergent shapes: "I think these features kind of need interplay to really pin down". Four changes the user agreed to in discussion, not yet built. Plan them together, since each changes how the others feel:
  1. **Several materials in one shape, as a bundle** (the user chose this over one block with its own look, which would use a look slot per design). The table stops refusing mixed materials. One material still makes an ordinary part; several make a bundle, one carried item holding one single-material piece per material. Placing checks room with the whole shape, then puts each piece in the boxel as its own block, all turned alike, using the shared boxels from the block shapes toy. Breaking takes off the piece aimed at and drops it as an ordinary part. Menu, preview and hotbar pictures draw each piece in its color. Touches crafting, carrying, placing and drawing.
  2. **Building on the side of the table.** Clicking the top builds on the top; clicking a side builds on the side (a toggle in the menu switches). On the side, the grid is the wall as you face it, stacks grow out of the wall toward you, and nothing floats is measured from the wall, so overhangs (shelves, brackets, ledges, wall lamps) become possible. The wall becomes the shape's far side, which basic placement already points the way you look, so looking at a wall mounts it upright; basic placement on a wall face should point the far side straight into the wall so it doesn't depend on looking head-on. Known limit: precise modes put the bottom against faces, so wall parts go on sideways there.
  3. **A third placement mode, "edge", for comparing.** The user: "if you try something new, add it as a option with the middle click so I can compare back and forth". Middle click cycles basic, precise, edge. Edge: the middle square puts the bottom against the face; an edge triangle puts the bottom toward that edge (a slab stands upright in that half of a floor block, a post lies flat pointing that way); the front always comes from where you look. The user's reasoning: the current precise middle square is nearly redundant, and "its more often easyer to posision yourself for placement, then to add another block". Brings back standing a slab upright in half of a floor block, lost to the stacking rule.
  4. **Drop the upside-down flip on the upper half of a wall** in basic placement; only ceilings flip. The table's flip box covers upside-down shapes now. The user found it annoying when making a stair that rises half a block: aiming high flipped it into something useless. A later idea, not agreed: aiming high could raise a shape that fits in the bottom half into the top half, upright, so half-height stairs can continue from a slab.
- **Generative crafting.** A few materials, a few property ingredients and a capacity limit.
  - *Notes for planning* (2026-11-01, from closing Inventory and items): the user expected an inventory with drag and drop and wants it designed in this toy's plan, not as a separate inventory toy. What exists to build on: the player carries amounts of each material in 1/4096ths of a boxel with one shared capacity (`sim::Inventory`, `api::InventoryDef`), placing costs a block's volume of its material, and blocks name their material. Settle in the plan what gets dragged (an amount of material or a finished part) and where to (a pattern, a block). The backlog's place on left click and break on right click item may fit in here.
  - *Notes for planning* (2026-11-02, brainstormed with the user, starting from "how should doors work?"). The user is aiming for 100% emergent crafting: no item is special-cased, and even doors and fences come out of the system. Framing: *emergent shapes, authored verbs*. Properties are written by hand (by the base mod or other mods); what emerges is which shapes carry them and how they combine.
    - **Properties come from materials.** A material brings its properties into any block it's part of, and they act where that part is (only the knob is clickable). First ideas: a *trigger* (sends a signal when the player presses interact, E, on that part), a *toggle* (flips the block between two shapes on a signal), a *listener* (picks up signals from neighbors) and a *connector* (for fences: makes a connecting piece toward a neighbor of the same material). A door is a toggle-material panel with a trigger-material knob; a lever beside it plus a listening door is a remote door; a material that passes signals on gives wiring.
    - **Open design points.** Where a toggle's second shape comes from: crafted as a "state 2" on the table, or (leaning) worked out by the material, such as a 90° turn about the edge touching its neighbor, so the hinge is wherever it's attached. A toggle into an occupied space is refused, like placement's red ghost. Signals move one block per tick, never loop within a tick, and need defined behavior at the loaded world's edge. Connector pieces are derived from neighbors when they change, never saved as real blocks. Blocks with complex behavior (furnaces, chests, tools) each need their own property design, such as a chest holding as much as the space inside it; that's where "100%" will be tested. Players need some way to see which materials have which properties. Blocks made of several materials (a panel with a knob) are currently deferred in Emergent shapes and would be needed.
    - **A simple crafter and an advanced one (the user's idea).** Early on there's a plain crafting menu where recipes need seemingly random items. Unlocking the advanced crafter shows why: the list isn't arbitrary, the items are the materials with the properties the design needs. Early game isn't annoying, but you sense you could design your own door. To make the reveal hold: every simple recipe is a saved design, its ingredient list worked out from the design's parts and never typed by hand (the base game ships designs as data); the player's own designs join the simple menu next to the stock ones; the same property always comes from the same materials, so attentive players spot patterns before unlocking; properties should make intuitive sense (copper carries signals, springy wood hinges); slots can take any material with the property ("any hinge wood"). Watch: the simple menu must be good on its own; what counts as basic and when the advanced crafter unlocks is an open progression question; stock designs should open in the advanced crafter as starting points.
    - **Cost.** A design costs once, not per placement: the world stores 4 bytes per boxel whatever's there, and each crafted shape is one registered block kind. Properties belong to the kind, so they're free per placement. A toggle can be two kinds swapped in place, so a door needs no per-block state; state that can't be a kind would need a sparse layer. Signals cost only when something happens. *Compile designs when crafted* (resolve the materials' properties into a flat behavior list on the block kind), so an emergent door runs like a hand-written one: the generic dispatch is nanoseconds per click, WebAssembly property code microseconds per event, and a trigger and toggle in the same block act directly with no signal hop (signals between blocks may take a tick, about 17 ms). Detail costs more than emergence: a detailed shape has more faces to mesh, emergent or not. The real risk is big player-built circuits (redstone lag), managed with limits like signals per tick.
    - **Limits to plan for.** Block IDs are 16 bits (about 65,000 kinds, counting every material × shape and each toggle's second shape); widen before it's painful. The block and cell tables never shrink while running, so unused shapes need a cleanup pass once saving exists. Saves must carry the shape data of every design used, since names are hashes. Meshing cost of worst-case crafted shapes is unmeasured; a small benchmark (a chunk of them, timed and face-counted) would settle it.
    - **A proof toy.** Trigger, toggle (with the derived 90° turn) and listener. Done when you can build a working door, and a lever that opens it from beside it, with no door code in the engine. Connectors (fences) and wiring would be a second toy. Doors open only with interact, never with left or right click, which answers the design doc's ambiguous-interactions gripe.
- ~~**Combat feel.**~~ Done (2026-10-31), see Done toys.
- ~~**Cow brain.**~~ Done (2026-11-03), see Done toys. Added 2026-11-03, the second of three steps toward a fighting cow (Cow animation is done). Suggested next. A behavior hook in the API shaped like the movement controller: the engine keeps each cow's state as plain data, passes it in each tick and gets back what it wants (walk direction, facing, which clip to play). The base mod's cow wanders, grazes, turns to look at the player and runs off when hit. Cows walk for real: steering, stepping up blocks, not walking off ledges. Native Rust first; WebAssembly with the behavior seam. Pays the Cow animation toy's clip-choice debt. Keeps the pen's respawning cows (the user's call).
- ~~**Cow herd.**~~ Done (2026-11-03), see Done toys. Added 2026-11-03 at the user's request, after Cow brain: "I would want cows to have more herd behaviors". Suggested before Fighting cow, since spooking together is half of what makes a fight feel alive and the angry cow can build on the same view of other cows. Brains only see themselves and the player today, so the API needs a short list of nearby creatures in `BrainContext` (offset, velocity, kind, maybe mode), capped to the nearest few. Behaviors to try: staying near the herd and drifting back when one strays, keeping a little space, grazing roughly the same way with a leader the others follow, and spooking together when one is hit. The 11×11 pen with four cows is cramped for this: a bigger pasture in the test area with more cows (no world generation needed). Could also feed Fighting cow, such as the herd charging when one is hit.
  - *Looking at the player* (2026-11-03, from the user asking whether cows look at people correctly). Cows' eyes are on the sides; only a narrow strip in front is seen by both eyes. Alert or curious, a cow lifts its head, points it straight at the thing with both ears forward and often freezes: Cow brain's watch mode does this. Relaxed, it keeps grazing and tracks with one eye and an ear; uneasy, it keeps the thing in side view and turns or moves away. Two changes to try: turn the head and neck first (bone override in the client, or a look target from the brain), with the body following only if interest lasts; and graded reactions by distance: notice but carry on (a head turn or ear flick), then head up and face, then back off a few steps, sooner if the player moves fast. Real herds bunch and face a newcomer together.
  - *Creature collision* (2026-11-03). Not planned anywhere before (deferred in Combat feel as "targets colliding with the player and each other"); cows today collide only with blocks, so they walk through each other and the player. Proposed: a soft push in the engine, not solid boxes: overlapping bodies are nudged apart over a few ticks, harder the deeper the overlap, the lighter one moving more (by `weight`). The player can shove a cow and a cow the player, nothing gets stuck or jitters, it's cheap (only nearby bodies), and it works the same for every mod's creatures. Built here because herds need spacing; Fighting cow adds the hard hit on top.
- **Fighting cow.** Now the current toy. Added 2026-11-03, the third step. Player health, the cow turning angry, a charge or headbutt (the `punch` clip) with a wind-up telegraphed longer than lag, knockback on the player, and something for the guard to do. Combat depth becomes tunable here; the `die` clip may play before the burst.
  - *Notes for planning* (2026-11-03, from closing Cow brain): the cow's brain is `base::cow::CowBrain`, a state machine in memory slots (mode, time left, heading); an angry mode slots in beside flee. The brain already gets `Hurt` (direction, damage) and the player's offset and velocity. It has no way yet to act on the player: a charge needs something like an attack in `BrainOutput` (a hit box and timing the engine resolves, so lag rules stay in the engine), plus player health in the API. Cows can't leave the pen (the sill), so a charge has to happen inside it or the sill goes.
  - *Head hit boxes* (2026-11-03, deferred from Cow herd, the user's call). Hit boxes are fixed to the body and don't follow the animated head, so a grazing cow's head box stays where the head was. A headbutt needs the head box where the head is: move boxes with their bones, or have the brain pick box poses per clip.
- **A real hind-leg kick for the cow.** Added 2026-11-03 at the user's question, from Fighting cow ("do you have the animations for the kick?"). The cow model has no leg kick; its close attack is a head toss (the `punch` clip). A kick needs a new clip, the rear legs hand-keyframed in the model file, and an attack for when the player is behind the cow.
- **Combat depth.** Added 2026-10-31, deferred from Combat feel. Ideas for a higher skill ceiling, to try once enemies, tools and movement give combat something to be tuned against: swing direction from the mouse during the wind-up (sweep sideways, slam down, launch up), air combos with growing knockback, a sweet spot at the club's full reach, cancelling the recovery into a jump or dash, a charged heavy swing with a perfect-release window (the robot's spring), and what the guard does once something hits back. Also the Doom loop: close kills restoring health once the player has some.
- **Liquid rule sets.** The rule sets from the design doc.
- **Advanced movement revisited.** Added 2026-10-26 at the user's request. Comes after toys that add tools, items and enemies. Rework push-out (block-jumping, and later pistons). The user doesn't like that the body is moved out of the block instantly, like a teleport, or that the launch is mostly upward. Prompted by spam-placing blocks at the dead end of a 1-high tunnel while crouched: the body crawls backward through position shifts while the speed readout stays at 0. Things to settle:
  - How the body gets out of the block with real motion instead of a shift, and how much of the launch should go up.
  - What placing a block while crouched should do. The crouched body's center is at or below the block's center, so today it gives a sideways shove off the side of the block that was never designed.
  - Whether a jump should keep wiping out the push-out kick. Today, placing a moment before jumping loses the kick, which makes block-jumping feel inconsistent.
  - **Known flaw, deferred to this toy (2026-10-26):** the kick can point opposite to the shift. The kick's direction comes from the body's position before the shift, so in the tunnel case the body is shifted backward but kicked forward, into the block it just left, and the collision zeroes it. That's why the speed readout stays at 0 (the readout itself is correct). Found by reading the code and confirmed by the user's play; not yet covered by a test. The redesign must make the launch go the way the body actually leaves the block.
  - **Direction proposed by the user (2026-10-30, after the block-jump trainer),** recorded as candidates to try:
    - *Kick straight up only,* keeping the body's own horizontal speed. Forward speed comes from running, which the player controls. The standing-position tilt is noise players can't learn, and dropping it also removes the opposite-direction flaw above. Pistons can bring their own direction later. Note: the 2026-10-26 entry says the user disliked the launch being *mostly* upward, so settle which one holds when playing it.
    - *Analog kick, scaled by how far the block has to push the body:* full with the feet at the bottom of the boxel, fading to nothing as they near the top. That turns the cliff between "kick" and "too late" into a smooth slope ("sooner is stronger") and reads like a spring, which fits the robotic player.
    - *Order-independent jumping:* a jump adds to rising speed instead of resetting it, so placing a tick before the jump no longer loses the kick. The controller's comment says the jump replaces *downward* speed, but the code zeroes upward speed too. Changing it affects all jumps, so it belongs in this toy, not in a fix.
    - *Later:* push the body out over a few ticks like a spring instead of the instant shift. Collision has to ignore the block while the body is still inside it.
    - The block-jump trainer is the measuring tool: once this toy is done, same-tick, early and late attempts should land close together.

Ideas that come up while playing get added here.

### Infrastructure (added when a toy needs it)

- Saving and loading, with block names in the save
- World generation
  - *Creatures on terrain* (2026-11-03, from Fighting cow, at the user's request). The cow fight was built for the flat pasture. The user expects terrain with more detail than Minecraft's (slabs, stairs and crafted shapes, down to sixteenths), which cuts both ways: half-height steps make more ground walkable with the cow's 0.6 step height, but there are more small obstacles to catch on, and any pathfinding has to understand shaped cells, not just full blocks. When world generation arrives, check:
    - No pathfinding, and full blocks can't be stepped: a ledge between player and cow ends the fight, since the cow gives up. Fixes, by value: let creatures hop up a block, like Minecraft mobs; make an angry cow wait below or circle instead of leaving, so standing on a block isn't a free escape (the user wants herd fights to be challenging); then short-range pathfinding as an engine service.
    - No line of sight: cows notice, defend and chase through hills and walls.
    - Charges are straight lines: trees and rocks stun them often. Probably good, terrain as a tool against the cow.
    - The edge guard is off while fighting, so a charging cow runs off cliffs, and creatures take no fall damage.
    - Knockback near drops: a charge throws the player about 4 m up and 12 m/s out, which off a ledge becomes heavy fall damage. Good danger, worth keeping.
- LOD, once world generation exists

### Seam backlog (added when the game is ready for them)

- ~~**Data mods.**~~ Done (2026-10-27), see Done toys.
- **WebAssembly behavior.** Sandboxed mod code for block behavior, once a toy has behavior worth sandboxing, such as crystals or liquids. Sleeping blocks and read-old, write-new automata. The first step is done: the WebAssembly movement seam (2026-10-26) runs `base`'s movement controller as a component, so block behavior can follow the same pattern (WIT interface in `api`, guest export macro, `wasm_host` wrapper). *To settle (2026-10-27):* keep entity and block state in the engine, not in the mod, before any behavior keeps state in a mod (see the design doc's "Mod memory"). The movement controller still keeps its timers and charge inside the mod and should move to that pattern. Test whether wasmtime lets the engine drop a mod's freed pages (`madvise` on Linux, `DiscardVirtualMemory` on Windows), driven by an allocator in `api`.
- ~~**Sandbox limits.**~~ Done (2026-10-28), see Done toys.
- **Seed a modder.** Once a few seam slices are done, have an outside modder build a small toy on the API and fix what blocks them.

### API debt

Shortcuts taken around the API, each with the toy that needed it. Paid down in seam slices.

- **Fixed input actions** (movement toy; break, place and palette slots added by the block editing toy, crouch by the advanced movement toy, placement mode by the block shapes toy, drop and shape by the inventory toy; shape replaced by interact, and turn_part added, by the emergent shapes toy). Move, jump, sprint, crouch, use, second use, drop, shape, the palette slots and the debug keys are a fixed engine set in `sim::PlayerInput` and `client::controls`. Mods can't define their own actions. API version: mods register named actions with default bindings, and controllers read actions by name.
- **Chase drill in the client** (advanced movement toy, temporary; ghost hits added by the combat feel toy). The ghost and objectives in `client::drill` are game logic living in the client, and it finds landmarks by the `landmark:` prefix on waypoint names. Remove the drill rather than moving it into the API, unless a later toy wants real enemies, which would need entities and a game-rules hook in the API.
- **Block-jump trainer in the client** (block-jump trainer toy, temporary). `client::trainer` judges attempts from read-only diagnostics, `Player::last_step` and `Simulation::last_place`. Remove it with the chase drill, or once Advanced movement revisited makes block-jumping consistent.
- **Tool look in the client** (combat feel toy). Every tool is drawn as the same club, posed by `client::combat`; mods can't give a tool its own model or swing poses. API version: a tool names a model and its poses.
- **Crafting menu in the client** (emergent shapes toy). The table's menu, its strokes and the turn and flip controls live in `client::crafting`; mods can't change how crafting is presented. The rule itself is in `api::crafting`. API version: a client-side mod API for menus, or a data description of the table's grid.
- **Health bar in the client** (fighting cow toy). `client::hurt` draws the player's health and the hurt flash; mods can't change how health is shown. API version: a client-side mod API for the HUD.
- **Camera feel in the client** (movement toy; hit-stop, view kick and shake added by the combat feel toy in `client::combat`). FOV widening, head bob, landing dip and step smoothing live in `client::feel`, so mods can't change them. API version: a client-side mod API for camera effects, presumably driven by the same tunables.

### Phase 2 decisions

- **A robotic player** (tentative, 2026-10-25): the user wants movement and camera feel to read as a robot. Landings are a shock absorber: soft landings compress smoothly, damaging ones bottom out hard. Later feel work (combat, damage) should keep to it.
- **Skyrim-style controls** (2026-10-31, from Combat feel): whatever is held has a main use on left click and a second use on right click. Blocks break and place; a tool swings and guards. Part of the answer to the ambiguous-interactions gripe; the Interaction controls toy builds on it.
- **Signature feature:** not decided yet. Choose it once the block interaction and crafting toys have been played, and record the choice and why in the design doc.

### Fixes

Bugs found in already-merged code, each fixed on its own `fix-` branch (see the workflow in `CLAUDE.md`). One line each: date, branch, what was wrong.

- 2026-10-28, `fix-step-smoothing`: a step-up moved the body but not its previous position, so the renderer blended the rise between ticks while the camera feel also eased it, making the view dip and snap when climbing. Latent until the block shapes toy gave the player a step height.

### Done toys

#### Cow herd (branch `toy-cow-herd`)

**Stage:** Done (2026-11-03). Verified: the user played it in the pasture and called it good. Plan approved 2026-11-03.

**Goal:** cows act as a herd: they keep together and drift back when one strays, keep a little space, loosely follow a leader when wandering and grazing, and spook together when one is hit. Bodies push each other apart, the player included.

*Plan*
- **API (`api::brain`).** `BrainContext.others: Vec<Neighbor>`, the nearest few creatures within sight, nearest first, capped at 8: `offset`, `velocity`, `kind` (name), `id` (stable, so a brain can pick a leader) and `signal`. `BrainOutput.signal: String`, a short label others see (`"alarmed"`), so a spook spreads without brains reading each other's memory.
- **API (`api::crowd`).** A `CrowdDef` the mod registers, like `PushOutDef`: how far creatures see each other, how hard overlapping bodies are pushed apart, the push's top speed and the player's weight. Without one, no neighbors and no pushing.
- **Engine (`sim::combat`).** One pass over all creatures per tick builds the neighbor lists before brains think (every pair; a spatial grid can wait for many more creatures). Then a soft push: each overlapping pair, cow–cow and cow–player, is moved apart this tick, harder the deeper the overlap, split by weight so the lighter moves more. The push is a displacement through the normal sweep, not velocity, so nothing is shoved into a wall, nothing slams and nothing keeps momentum. Bodies are level capsules: a creature's runs along its facing over its hit boxes' length (the cow is long, its collision box is not), the player's is a circle.
- **Base (`base::cow`).** Nudges on the existing state machine: cohesion (beyond `stray_range` from the herd's center, wander back toward it), spacing (inside `personal_space`, turn away from the nearest cow), leader (the lowest id it sees; wander roughly the leader's heading, lean toward grazing when it grazes), spooking (a hit cow shows `"alarmed"`; a cow seeing an alarmed one within `alarm_range` flees too, shorter, away from it, and shows `"alarmed"` itself). Each a tunable.
- **Test area.** A pasture about 30×30, fenced one block high (cows can't step it, the player jumps it), with all the cows: free range, 8 of them. The small pen stays, empty.
- **Tests.** Push (overlap separates, heavier moves less, a pile settles without jitter, never into walls); neighbor list capped and nearest first; each herd rule in the brain; integration: the herd stays within a spread over 2 minutes, hitting one sets others fleeing within a couple of seconds, the player can shove a cow and walk through a herd without getting stuck.
- **Out of scope:** looking at the player (head-only looking, graded reactions, facing a newcomer together: deferred, the user's call), pathfinding, line of sight, anything from Fighting cow.

*Built* (as planned, with small additions)
- `BrainContext.id`: a brain's own ID, so it can tell whether it's older than everyone it sees (the leader).
- The push moves bodies as a displacement through the sweep (and the edge guard), so it gives no speed, no slams and nothing goes into walls; the player gets `Player::shove`, applied on its next step. `max_push` caps how fast. Base values: sight 12 m, push 25 per second, max 8 m/s, player weight 1.5 against the cow's 2.5. With a lower cap the player could walk straight through a cow.
- The cow's memory gains a slot, `Calm`: seconds it ignores alarms after a scare, so a spook spreads then dies out instead of bouncing between cows forever. New tunables: `stray_range`, `personal_space`, `follow`, `follow_spread`, `alarm_range`, `spook_time`, `calm_time`. Signals are `"alarmed"` (fleeing) and `"grazing"`.
- Pasture: a wood fence one high from (-62, -62) to (-32, -32), 8 cows, waypoint `pasture` just east of it (the 5th teleport press). The far test area gets the fence but no cows, as before. The pen stays, empty; its platform and slab steps no longer have cows on them (stepping is still covered by the `sim` tests).
- Crowd sliders in the tuning panel.
- Tests: 416 pass across the workspace (1 ignored, as before). Clippy is clean natively and for `wasm32`. New: crowd def (2), pushing and neighbors in `sim` (8), herd rules in the cow brain (6), and in the base world: the herd keeps together for 2 minutes without cows inside each other, a strayed cow comes back, hitting one spooks at least 5 of 8 and it dies out, the player shoves a cow and can't walk through it, and the player crosses the herd without getting stuck. The game starts and runs cleanly.

*Done when:* (1) cows keep together as a loose herd and a straying cow drifts back; (2) they keep a little space and don't walk through each other or the player; (3) the player can shove a cow, a little lighter than one, and is shoved back, with no jitter or sticking; (4) hitting one cow spooks the ones near it; (5) cows still stay in the pasture and everything from Cow brain works; (6) tests pass and clippy is clean.

*Result*
- Met in play: (2) no cow overlaps another or the player; (4) hitting one spooks the herd; (5) they stay in the pasture (the fence makes this a given), and watching and fleeing from Cow brain still work; (6) tests and clippy.
- Accepted on tests plus play, not fully seen: (1) the herd stays together (the user: "they are at least not leaving", and random wandering starts and ends with them together); (3) the player shoves a cow, but a cow pushing the player back wasn't seen, because cows near the player stop to watch instead of walking into them. Both are on the low-priority play check list.
- *Decisions:* the stampede stays. A cow that sees a scared one runs mostly the way that cow runs, with a smaller push away from it, so the herd bolts together; only the hit cow runs straight away from the player. Cows that watch the player never walk into them on their own; that's Cow brain's watch mode, not a collision gap.
- *API debt:* nothing new.
- *Deferred:* hit boxes don't follow the head when it moves (grazing), added under Fighting cow. Looking at the player (head-only looking, graded reactions, facing a newcomer together) stays in the backlog as before. A spatial grid for neighbors waits for many more creatures.

#### Cow brain (branch `toy-cow-brain`)

**Stage:** Done (2026-11-03). Verified: the user played it: "yes that all works as intended". Plan approved 2026-11-03; the user engaged with the recommended answers to the open questions.

**Goal:** the pen's cows wander, graze, turn to face the player when near and run off when hit. They walk for real: step up slabs, not full blocks, and don't walk off ledges. The brain picks the clip, paying the "Clip choice in the client" debt.

*Plan*
- **API (`api::brain`).** `Brain: Tunables + Send + Sync` with `fn think(&self, ctx: &BrainContext) -> BrainOutput`. It takes `&self`: no per-creature state in the mod, the first behavior to follow the design doc's "state lives in the engine". `BrainContext`: `dt`, velocity, yaw, on_ground, health and full health, `blocked` (walked into something last tick), the player (offset from its feet to the player's, and velocity), `hurt` (direction the hit came from, damage) since the last think, `random` (a u64 the engine derives from the tick and the creature's ID), and `memory`. `BrainOutput`: `walk` (wanted world-space horizontal velocity), `yaw` (new facing; the brain limits its own turn rate), `edge_guard: Option<f32>`, `clip` (a name) and `clip_speed`, and `memory`. Memory is a `Vec<f32>` the engine keeps per creature, zeroed at spawn.
- **Wiring.** `TargetDef` gains `brain: Option<String>`, `step_height` and `walk_accel`. `Registrar::register_brain(name, Box<dyn Brain>)`, like movement controllers. `targets.toml` gets `brain = "base:cow"`. A target without a brain is unchanged.
- **Sim.** `Targets::step` asks each brain before moving, with the player's position. On the ground, horizontal velocity moves toward `walk` at `walk_accel`; in the air knockback rules. The player's step-up sweep and `guard_edges` become shared helpers. Strikes and slams leave `hurt` for the next think. A "brains off" switch in the panel turns cows back into dummies.
- **Base (`base/src/cow.rs`, native).** A state machine in memory: wander (random direction, new one when blocked), graze (`eat`), watch (player in range: stop, turn to face, `stand`), flee (when hurt: run away from the hit for a while, `run`). Tunables: walk and run speed, turn rate, notice range, flee time, wander and graze timings, edge-guard drop. `libm` trig, ready for WebAssembly.
- **Client.** `Animator` plays the brain's clip at its clip speed; play-once clips play when first asked for, then hold. The preview still overrides. The graze timings move into the brain.
- **Test pen.** A one-block sill in the doorway (the player jumps it, cows can't), and inside, a slab step up to a two-high platform.
- **Tests.** Brain: wander turns from walls, notice in range only, flee away from the hit for the flee time, same random gives the same choices. Sim: steps up a slab not a block, stops at a ledge, memory persists and resets on respawn, hurt reaches the next think, knockback mid-walk, brains off, brainless targets unchanged. Client: the animator follows the brain's clip, holds play-once ends, preview overrides.
- **Decisions (open questions, taken at the recommendations):** the doorway gets a sill; looking at the player turns the whole body (head-only looking later); memory is a list of floats.
- **Out of scope:** WebAssembly brains, pathfinding, herding, head-only looking, line of sight, anything from Fighting cow.

*Built*
- As planned, with these small changes: no `walk_accel`. Drag on the ground now pulls toward the brain's walking speed instead of toward a stop, so one number covers both. Slams count only speed beyond what the creature was walking at, so a cow running into a wall isn't hurt. `Target` keeps `prev_yaw` so the client blends turning between ticks. A creature without a brain (or with brains off) plays its model's first looping clip.
- `api::brain` (`Brain`, `BrainContext` with `random_unit`, `BrainOutput`, `Seen`, `Hurt`), `Registrar::register_brain`, `ApiError::UnknownBrain`, `TargetDef::{brain, step_height}`. `targets.toml` takes `brain` and `step_height` (both optional).
- `sim`: the player's step-up sweep and edge guard are now shared (`sweep_stepping`, `guard_edges`). `Targets::step` takes a `StepContext`. `Simulation::{brain_names, brain_mut, brains_on, set_brains_on}`.
- `base::cow::CowBrain` registered as `base:cow`, with 14 tunables. The pen's doorway has a one-block sill (the lintel is gone, so the gap above it is 2 high), and in the north-west corner there's a platform 2 high with half-block steps. One cow moved off the steps.
- Client: the animator follows the brain (the graze sliders are gone from Animation), facing turns every frame, and the panel has a "Brains" section with a think switch and each brain's sliders (covered by Reset and Print).
- Tests: 395 pass across the workspace. Clippy is clean natively and for `wasm32`. New: the cow brain (8), walking targets in `sim` (7), cows wandering for 90 s without leaving the pen, watching, fleeing, brains off, unknown brain names in code and in data, and the animator following the brain. The game starts and runs cleanly; how it looks and feels rests on the user's play.

*Done when:* (1) cows wander, graze and face the player when near, out of step; (2) hit cows run away, then calm down; (3) they step up slabs, not full blocks, don't walk off the platform and stay in the pen; (4) walk and run match their speed without obvious foot sliding; (5) hits, slams, tumbling and respawning work as before; (6) tests pass and clippy is clean.

*Result*
- All six "Done when" criteria met.
- *API debt:* "Clip choice in the client" is paid: the brain picks the clip. Nothing new. Brains follow the design doc's "state lives in the engine"; the movement controller still keeps its state in the mod (noted under the WebAssembly behavior seam).
- *Deferred:* WebAssembly brains (a `brain.wit` like `movement.wit`, for the behavior seam), head-only looking, line of sight, pathfinding and herding.

#### Cow animation (branch `toy-cow-animation`)

**Stage:** Done (2026-11-03). Verified: the user played it: "the cows animate and everything works". Asked for by the user: "add some basic ai, I want to eventurly get a fighting cow but maybe that should be multi steps". Split into three toys: Cow animation, then Cow brain, then Fighting cow (see the backlog). The user chose animation first and to keep the respawning cows.

**Goal:** the pen's cows look alive: they breathe, shift, flick their tails and graze. Every clip the model has can be previewed, so walk, run and the headbutt are checked before an AI needs them.

*Plan*
- **Source.** The original `mobs_cow.b3d` has 26 bones (spine, legs, neck, head, jaw, ears, horns, tail), 186 frames at 60 fps, and clips named in the Luanti mod's `cow.lua`: eat 0–30, stand 35–75, walk 85–114, run 120–140, punch 145–160 (a headbutt), die 165–185. Assimp refused it because its keys run 1–186 against an animation of 0–185. Most of its 412 vertices follow one bone; a few spread small weights over up to 23, so keep each vertex's 4 strongest and renormalize.
- **Converter.** `tools/b3d_to_glb.py` writes one `.glb` with the mesh, skeleton and the six clips cut at those frame ranges and at `cow.lua`'s speeds, handling the frame offset. The Y-up to Z-up turn stays in the converter. glTF so a modder can open it in Blender. OBJ stays for static models.
- **API (`api::media`).** `ModelDef` gains optional skinning: per vertex up to 4 bone indices and weights; bones with a parent and rest pose; named clips with a length, whether they loop, and per-bone position and rotation keys. Plain data; `validate()` checks indices, weights, parents before children and key times in order. The simulation still never looks inside.
- **Loader (`data_mods`).** `targets.toml` takes `mesh = "models/cow.glb"` beside `.obj`, read with the `gltf` crate, with file-and-line errors.
- **Client.** A Bevy skinned mesh, bones as child entities under the existing tumble and squash entities, so hit feel doesn't change. Our own small sampler sets the bone transforms each frame (not Bevy's `AnimationPlayer`, since the data arrives as API data). Each cow loops `stand` and now and then plays `eat`, at random times so they aren't in step.
- **Preview** in the tuning panel: force every cow into one clip, a playback speed, and the idle and eat timings. No new keys.
- **API debt:** the client picks which clip plays (idle, eat). The Cow brain toy pays it by letting the brain choose.
- **Tests:** the sampler (blending, looping, a clip that holds its last frame, speed); `ModelDef` validation; the glTF loader (the real cow has 26 bones and 6 named clips; broken files name the file); the OBJ path still works. Screenshots of a few clips for tearing.
- **Out of scope:** AI and walking about; hit boxes following the animation (they stay on the standing pose); shading following moving parts (baked shading stays as is); new sounds.
- **Open question, taken at its default** (the user said engage without answering): a breaking cow still bursts at once, and `die` is only previewed until the Fighting cow toy. A tumbling cow keeps its current clip; there's no flinch clip.

*Built*
- As planned. `tools/b3d_to_glb.py` replaces `b3d_to_obj.py`; `mods/base/models/cow.glb` replaces `cow.obj` (412 vertices, 26 bones, 6 clips). Blitz3D's quaternions turn the opposite way to the usual convention, so b3d (w, x, y, z) becomes (w, x, z, y) in Z-up; checked by posing every clip on the CPU and drawing side views (scratch only): legs swing in walk and run, the head reaches the ground in eat, the cow folds down in die, punch is a head-down butt, stand opens the jaw.
- `api::{Skin, Bone, BonePose, Clip, Track}`, `ModelDef::skin`, `Skin::pose` (sampling) and `BonePose::blend` in `api::media`. `targets.toml` models take `.glb` and `play_once` (the cow: eat, punch, die). `data_mods::glb` reads them with the `gltf` crate (no default features).
- `client::animation`: `Animator` (idle, graze, preview, blending), `AnimationTuning` (speed, blend, graze_min, graze_spread) and `AnimationPreview`. The rig hangs under the existing tumble and squash entities and turns Z-up to render space once, so bones and keys stay Z-up. Skinned meshes skip frustum culling. Clips freeze during hit-stop like everything else.
- Panel: an "Animation" section with an "every creature plays" clip picker and the four sliders; Reset and Print cover them.
- Tests: 372 pass across the workspace, clippy clean. New: skin validation, sampling (blending, wrapping, holding), pose blending, the cow loading with its bones and clips, glb mistakes naming the file, the animator (out of step, grazing and back, speed, preview repeat, blending from the last pose), animation tunables. The game starts cleanly; the in-game look wasn't screenshotted (capturing the window failed here), so it rests on the user's play.

*Done when:* (1) the pen's cows play the standing idle with legs, ears and tail moving and no tearing or stretching; (2) they graze now and then, out of step; (3) every clip plays correctly from the panel and the speed works; (4) hits, flash, squash, tumbling, slams and bursts work as before while animating; (5) tests pass and clippy is clean.

#### Ambient occlusion (branch `toy-ambient-occlusion`)

**Stage:** Done (2026-11-02). From the backlog entry of the same name. Verified: the user played it and tested all the main things ("they work"); "hard to notice but makes a big diffrece when comparing on vs off ... things look better". Defaults kept (strength 0.6, reach 16); remesh time accepted.

*Built*
- As planned. The four cubes are measured together as one box (`occlusion` in `mesher.rs`); a boxel's 27 neighbors are only gathered once one of its faces is drawn. Defaults: strength 0.6, reach 16.
- The diagonal rule, made exact: each quad splits along the diagonal whose two ends are lit most alike, so a lone dark or lit corner stays in its own triangle.
- **Remesh time** (release, all 59 chunks of the test area, 82,114 quads, best of 5; the ignored test `time_a_full_remesh`): 85 ms before, 197 ms with AO, so about 1.4 to 3.3 ms a chunk. An edit remeshes a chunk or a few; dragging an AO slider remeshes all 59 each change, so expect a hitch while dragging.
- Tests: `a_lone_block_and_open_floor_are_not_darkened`, `a_floor_corner_darkens_by_how_many_walls_meet_there`, `a_slab_beside_a_wall_darkens_less_than_a_full_block`, `quads_split_along_the_diagonal_that_keeps_a_lone_dark_corner_to_itself`, `darkening_reads_across_chunk_borders`, `reach_narrows_how_far_darkening_spreads`.

*Plan*
- **How it works.** For every corner of every quad, the mesher measures the space in front of the face as four boxes meeting at that corner (over the face, past each of the two edges that meet there, and past the corner), each `reach` sixteenths on a side. The corner darkens by how much of that space block shapes fill, measured in sixteenths, so slabs, stairs and crafted shapes count by their real volume. For full blocks this matches Minecraft's smooth lighting: a floor corner against one wall gets half the darkening, a corner where two walls meet three quarters, open floor none.
- **Where it goes.** Multiplied into the vertex color beside the fake face shading; the shader doesn't change and it costs nothing per frame. Each quad's diagonal flips toward its brighter corners so the blend has no crease. Previews (ghost, hotbar pictures, items) have no neighbors and get none.
- **Code.** Client only: `client/src/mesher.rs` gets the neighborhood lookup; `mesh_occupant` takes a closure answering "how much of this box is filled". An `AoTuning` resource (`strength`, 0 is off; `reach` in sixteenths, starting at a full block) gets sliders in the tuning panel like `CameraFeel`, and changing it remeshes the loaded chunks. No new key, so `controls.txt` doesn't change. Nothing goes around the API.
- **Tests** in `mesher.rs`: a lone block on open ground isn't darkened; a floor corner against one wall gets half, against two three quarters; a slab beside a wall darkens less than a full block; the diagonal flips toward the brighter corners; strength 0 gives today's output; darkening reads across chunk borders. Time a full remesh of the test area before and after.
- **Open questions, taken at their defaults** (the user said engage without answering): reach starts at a full block; darkening is worked out only at quad corners, so a small shape against the middle of a big face's edge smears across the face. Revisit in play.

*Done when:* (1) inside corners and edges are visibly darker (wall bases, steps, the house's windows); (2) slabs and stairs darken by how much they fill; (3) no crease along quad diagonals; (4) strength and reach tune live, 0 turns it off; (5) remesh time is measured and still acceptable; checked by the user in play.

#### Wood and brick looks (branch `toy-wood-brick-looks`)

**Stage:** Done (2026-11-02). Verified: the user played it. Bricks: "brich look very good", and their stronger contrast stays ("grew on me"), so criterion 3 now covers the wood only. Wood at shade 0.15: "that was good". Asked for by the user (2026-11-02): "make the planks bigger and in diffrent orientatins, change ethe bricks to bigger and zero gourt, same with the wood can you make it not have a outline inbetween but use slight diffrent colors", then "lower contast all around". A small look change, so the plan was given in chat and not written up here first.

- **Bricks:** 16 long by 8 tall (two rows a block), no mortar, colors closer together (141 to 151 red, was 134 to 158), and the new `speckle` setting turned down from 0.08 to 0.04.
- **Planks:** `seam` is now optional, with new `thickness`, `shade` (each board's own slight tint) and `weave` (a checkerboard of cubes, boards along X in one and Y in the next, no joints). Wood and the crafting table use a weave of 8 with 4-wide boards, no seam, shade 0.03, and tighter grain colors.
- **Plan change during the build:** the plan said 8-wide boards, but in an 8-texel weave cube they fill the cube and read as square tiles. The user picked 4-wide parquet instead. A look repeats every block, so a woven board is at most 8 long.
- **First tuning (2026-11-02):** the user couldn't see the contrast at all ("maybe a little more"). Board shade 0.03 to 0.07, grain and brick colors spread about twice as far (bricks 137 to 155 red), brick speckle 0.04 to 0.06.
- **Second round (2026-11-02):** "bricks arnt working, maybe smaller gorutless, and maybe wood can be block long x, 1/4 block y, and 2 texles z, and evey layer they switch dir". Bricks back to 8 by 4, still with no mortar. The planks' checkerboard `weave` is replaced by `cross`: boards a whole block long, 4 wide and 2 thick, each layer a quarter turn from the one below. Wood and the crafting table use it.
- **Third round (2026-11-02):** "can you make sure adjacent square have more diffrent contrast? maybe drop the contrast for now so I can tell that you are impliemnting what I nead then we will walk it back". Colors and shades were picked at random per brick or board, so neighbors could match. Now a coloring (`distinct` in `looks.rs`) makes sure bricks or boards that touch, across block edges too, never share one: bricks need at least 4 colors (each touches four in the row above, which touch each other), and a board's `shade` is one of four steps. *Temporary:* contrast is exaggerated (4 far-apart brick colors, wood shade 0.25) so the user can see it working; walk it back once confirmed. *Later (2026-11-02):* the bricks keep it ("the contrast of the brick grew on me"); only the wood is walked back.
- **Fourth round (2026-11-02):** "can you make the brick rectangels, maybe explore more intricite patters where they are not just all layed out in the sma orientation in a layer". Offered basket weave, herringbone or both; the user picked herringbone. Bricks gain `pattern = "herringbone"`: 8 by 4 by 4 bricks zigzagging along X and Y in each course, each course shifted a brick's width along X. It repeats every 16 texels; 4 colors still keep touching bricks apart. No mortar for it yet. Running bond stays the default.
- **Fifth round (2026-11-02):** "is there a pattern that exits into the 3rd dimnsion". Offered rotating courses or a 3D weave, and explained that a true 3D herringbone can't repeat every 16 texels (it needs a multiple of 3). The user picked the weave: `pattern = "weave"`, 8-texel cubes of four 8 by 4 by 4 bricks, upright cubes checkered with cubes along X in the bottom half and along Y in the top, so every face shows all three. The brick look uses it; herringbone stays available.
- **Sixth round (2026-11-02):** "something where the pattern isnt a simple cube, like when it wraps it fitts into a cube, but parfial bricks should be on the brick face". Offered shifting the weave by an offset or staggering its cubes; the user picked the stagger: `pattern = "staggered"`, the weave's two layers of cubes in a running bond (the top shifted half a cube along X and Y), shifted a quarter cube by default so block edges cut through bricks. One layer checkers X and Y cubes, the other upright and X; some stacked cubes share a direction (three directions can't keep them all apart), but their bricks still differ in color. The brick look uses it; weave and herringbone stay available.
- **Seventh round (2026-11-02):** "brich look very good, wood is still a bit off can you try adding some dark sprinkles avg 8 per side". Planks gain `sprinkles` (about how many single dark texels a 16 by 16 face shows on average; each texel is one by a chance of `sprinkles / 256`, so any face or cut gets them) and `sprinkle_color`. Wood uses 8 in `[112, 82, 50]`; the crafting table has none. The bricks stay as they are. Wood's exaggerated shade (0.25) is unchanged for now.
- **Eighth round (2026-11-02):** "thinner grain, maybe no sparkes". Sprinkles removed again, before they were committed. Plank grain is now thin lines instead of noise: each row of texels along a board is one grain color, switching once at a random point along it, and rows beside and above each other in a board never match (rows take turns between the colors at even and odd places in `colors`). This applies to every plank look, so the crafting table changed too. Wood and the table each gained a fourth grain color, so both kinds of row switch.
- **Ninth round (2026-11-02):** "lets check it with less contrast", then "try lowering it again". Wood's board shade 0.25 to 0.04, so neighboring boards differ by a few percent. The grain lines and the bricks are unchanged.
- **Tenth round (2026-11-02):** 0.04 was "not enough maybe the middle bettween what it is now and the last": wood's board shade is now 0.15, halfway to the 0.25 last seen in play.
- Tests: `grain_runs_in_thin_lines_along_each_board`, `crossed_boards_turn_every_layer_and_are_told_apart_by_shade` (replacing the weave test), `neighboring_bricks_never_share_a_color`, `bricks_without_enough_colors_to_keep_neighbors_apart_are_an_error`, `herringbone_bricks_zigzag_both_ways_in_a_course`, `herringbone_needs_a_length_that_repeats`, `woven_bricks_run_three_ways_and_every_face_shows_them`, `woven_bricks_need_sizes_that_fit`, `staggered_cubes_shift_between_layers_and_the_edges_cut_bricks`, `brick_speckle_can_be_turned_down`, and new bad-recipe errors. The game's broken-recipe test now edits the new brick length.

*Done when:* (1) bricks are 2:1 rectangles in a staggered 3D weave, cut by block edges, with no mortar lines; (2) wood shows boards a block long, crossing each layer, with no outlines, told apart by slight shade; (3) contrast is lower on both; all checked by the user in play.

#### Remove the charged jump (branch `toy-remove-charged-jump`)

**Stage:** Done (2026-11-02). Verified: the user played it later the same day ("I validated the lack of a crouch jump"). The play check was first deferred at the user's request, so as not to hold up progress. Asked for by the user (2026-11-02): "I think crouch jump was a bad idea. can you just remove that?" Only the charged jump goes; the user chose to keep the mid-air tuck. A small removal, so it skipped the plan stage write-up.

- `base_movement`: the charge, its sinking of the body, and the tunables `charge_time`, `charged_jump_height`, `charge_sink` and `charge_grace` are gone. Crouching asks for `crouch_height`, nothing lower; a jump from a crouch is a plain jump. The tuck, crouch-walk speed and edge guard are unchanged.
- The movement course loses the 3-high ledge and its landmark, which only a charged jump could reach. The 2-high tuck ledge stays.
- Tests: the four charge tests and `a_charged_tuck_clears_a_higher_ledge` are removed; `crouching_does_not_change_the_jump` is added. `spawned_controllers_keep_their_own_state` now checks separate state with a buffered jump instead of a charge.
- Undoes part of Advanced movement (below), whose charged jump is kept there as history.

#### Emergent shapes from crafting (branch `toy-emergent-shapes`)

**Stage:** Done (2026-11-02). Chosen by the user (2026-11-01) ahead of Generative crafting. The drag and drop from Generative crafting's notes is designed here.

**Result:** the user played the stacking rule and liked it ("very interesting, I really like it"). The push-through rule it replaced was the toy's main lesson: emergent shapes need a rule that starts from what players want to make, and the shape's frame has to come from how it was built for placing to be predictable. Closed at the user's request before the follow-ups were built, because they "kind of need interplay to really pin down"; they're in the backlog as Crafting follow-ups. The criteria weren't checked one by one in play; the shapes in criterion 1 are covered by tests.

**Goal:** shapes come from crafting instead of each being its own block. A 4×4 grid on a crafting table, filled with materials and parts, makes a part; the face of the table used decides which way it's pushed through; parts go back into the grid, so shapes build on shapes, including non-prisms. Every shape in the base game becomes emergent.

**Approved plan** (2026-11-01)
- **Playing:** aim at a face of the crafting table (top, front, back or a side; not the bottom) and press **interact (E)**. A menu opens with the cursor free (E or Escape closes it): a 4×4 grid, your materials and parts with amounts, and a 3D preview of the result as it will sit relative to you. Drag a material onto cells to fill them; drag a part onto a cell to put a quarter-size copy of it there; right-click clears a cell. One material per pattern. A part in a cell is a pattern and isn't used up. **Craft** makes one part, paying its volume of material; shift-click crafts as many as you can afford. Parts are carried next to materials and share the volume capacity. Drag parts onto the palette; placing one uses it, breaking a shaped block gives the part back whole.
- **The rule:** each cell holds a 3D copy of its content at a quarter size (4×4×4 sixteenths): a material is a full small cube, a part is the part scaled down. Pushing through repeats each cell's copy 4 times along the face's axis. A part keeps its orientation in the table's frame, so a pipe made at the front, used at the side, gives 4 small pipes per cell with holes facing across; used at the front again it gives a finer prism. Parts that contain sixteenth detail (made from parts) are refused in a cell, shown red, so nesting is two levels deep.
- **Frames:** everything is in the table's frame (the table has a front, like stairs). Each face's grid is the view from that side as you'd see it standing there, so the back mirrors the front and one side mirrors the other. Placing points the table's front the way you look, then basic or precise placement turns it as usual; shaped blocks turn freely.
- **All shapes emergent:** base drops its slab, stairs, fine stairs, panel, window, fence post and log slab blocks and the `[shapes]` table; it keeps full blocks, one per material, plus the crafting table. The window can't be made (a full-depth push can't make a thin panel with a hole) and goes; the post can be made through a part in a cell. The test area builds its shapes through a new `WorldEdit::add_shaped`, so the world uses the same shapes players make. The shape key (G) is retired. `BlockDef.shape` stays in the engine for other mods; engine tests keep their own test blocks, and tests that named base's shaped blocks move to crafted ones.
- **Data:** a shape is a 16×16×16 grid of filled sixteenths, merged into boxes for collision and drawing. A shaped block is registered the first time it's needed, copying its look and properties from the material's full block, named from a hash of the shape (`base:wood_s<hash>`). A save will need each name's shape; noted for the saving slice.
- **API:** `api::crafting`: `CraftingDef { table, grid: 4 }` (from `crafting.toml`), the shape grid, `TableFace`, and the rule as plain functions. `WorldEdit::add_shaped`. A new `interact` action.
- **`sim`:** `BlockRegistry` registers shaped blocks at runtime; parts in `Inventory`; a `craft` call checking the table is in reach, one material, cells fit, enough to pay and room to carry; breaking a shaped block returns the part.
- **`client`:** `crafting.rs` (the egui menu, drag and drop, the preview); `Volumes` and the palette pick up blocks registered while playing; palette slots for parts. `interact = KeyE` goes into the user's controls file and `shape` comes out.
- **Risk:** a two-level shape can break into many boxes, so many faces and collision boxes. Measure a worst-case shape; if it's a problem, stop and say so rather than cutting the feature.
- **Tests:** a full grid gives a full block; slab, stairs and fine stairs patterns give the right boxes; material-only cells match the plain prism; a pipe on its own axis gives a finer prism and on another face gives 4 holes facing across; nested parts are refused; each face gives its frame and the back mirrors the front; box merging covers every sixteenth exactly once; one shape gives one name and ID, another material another; crafting pays the volume and makes one part, refuses past capacity or when short, and a part in a cell isn't used up; placing uses a part and breaking returns it; `crafting.toml` loads and a bad one reports the file and line; the test area builds from crafted shapes; the client's look volumes cover blocks registered later.

**Re-plan: the stacking rule** (approved 2026-11-01, replacing the push-through rule above; a deviation from the approved plan at the user's request)
- **Why:** in play, the push-through rule made many shapes nobody wanted and couldn't make ordinary ones. Every cell copied its content 4 times front to back, so nothing could change shape front to back at the block's scale: no thin stairs (a stair only part of the way through), no corner stairs, no block with one corner cut. Placing was unpredictable too, because a shape's frame came from where it was painted on the table: the same thin panel landed flat at the top of the block, on a side or at the front depending on the row and face used. The user: "it seems very good at making useless shapes, I strugle making anything normal".
- **Playing:** every face of the table opens the same menu, the table's top seen from its front (the per-face rule and `TableFace` are gone). Each cell holds a stack up to 4 layers high. Pick a brush (a material or a made part) and click a cell to add a layer on top; holding and dragging adds a layer only to cells as high as the first, so a sweep levels a row; the right button takes the top layer off the same way. The **turn** key (`turn_part`, default R) or button turns a part brush a quarter turn. A **flip** box turns the result upside down, for things easier built top first. Crafting stays free, one material per craft.
- **The rule:** a layer is a quarter of the block each way: a material is solid, a part is shrunk to a quarter, turned as painted. Every vertical line of sixteenths must be filled from the bottom with no gaps; anything floating is refused and its cell marked. Parts with detail finer than quarters are still refused, so two levels.
- **As built** (changed during implementation, with the user's agreement): the planned turn so the tallest side points away was dropped, since it guessed wrong on off-center shapes (a thin stair would face sideways). A shape keeps the frame it was built in: the table side is its bottom, and its far side points the way the player looks when placed. The same shape built turned is a separate part.
- **Placing:** basic placement is unchanged. Precise placement puts the block's bottom against the face aimed at: the middle square points its front along the look, a triangle toward that edge. So a slab lies flat against a wall and a post sticks out of it. Blocks that only turn their axis, like logs, keep the old triangle rule (lying toward the edge). Standing a slab upright in half of a floor block is lost (see the deferred note).
- **Base:** slab and stairs are heights; the post is a corner column made first, then stacked four high in the middle cells, turned so the four meet.
- **Tests:** one craft each makes a slab, a quarter slab, stairs, a thin stair, outer and inner corner stairs, a bucket and a vertical pipe; a gap floats and is refused at its cell; a part layer shrinks, must rest on what's below, and too-fine parts are refused; flipping keeps the far side far; a turned part goes in turned; the base post is the old post; precise slabs lie flat against floors and walls and a precise post sticks out of a wall; every face of the table is the table; the menu's strokes add and take off layers level with where they started; the turn key loads from the controls file.

- **Built** (2026-11-02): `api::crafting` now has `Layer`, `Stack`, `Stacks`, `stack_shape`, `floating`, `flipped`, `craft_shape(stacks, flip)` and a `heights` helper; `CraftError` is `Empty`, `TooFine { row, col, layer }` or `Floating { row, col }`. `sim::Craft` has `cells: Cells` (stacks of `Piece { thing, turns }`) and `flip` in place of `face`; `Simulation::is_table` replaces `table_face`. The client menu draws stacks with their heights and top layer, marks the refused cell red, and shows the brush and its turn. `turn_part = KeyR` was added to the user's controls file. 349 tests pass, clippy is clean, and the game starts with no errors. The implementation notes below describe the first rule, kept for the record.

**Done when** (re-planned)
1. Slab, stairs, thin stairs, a corner stair and a bucket can each be built in the menu, and the preview shows them.
2. A part placed faces the way it was built: its far side the way you look.
3. Placing feels predictable: in precise mode slabs and panels lie flat against what you click and posts stick out of it; stairs face you.
4. `cargo test` and `cargo clippy` are clean.

**Deferred:** paying for crafting (free since 2026-11-02, at the user's request); saving part names (session only); nesting deeper than two levels; mixed materials in one pattern; saving parts between sessions; Generative crafting's properties; swapping left and right click; aiming at odd shapes always placing against the outer block face (deferred to see whether the stacking rule needs it; it would also stop slab-on-slab filling the top half); *a later plan for precise placement* (2026-11-01, at the user's request, since the stacking rule loses standing a slab upright in half of a floor block): "advanced placing mode might shoot past sub blocks, or maybe 1 boxel of sub blocks", meaning precise aiming could pass through partial blocks to what's behind them, or only through thin ones (about 1 boxel).

**API debt added:** the crafting menu in the client (`client::crafting`): mods can't change the menu. `interact` and `turn_part` join the fixed input actions.

*Implementation notes* (2026-11-02)
- **Changed from the plan:**
  - `CraftingDef` only names the table; the grid is fixed at 4 by the engine, since a 4×4 grid of quarters shrinks exactly into sixteenths.
  - Parts aren't dragged onto the palette: every part carried gets a slot after the materials, as materials do. The menu has a "hold" button per part, and crafting puts the new part in hand.
  - The preview is a picture in the menu, seen from in front of the table's front, a little to the left and above. It isn't a 3D view in the world.
  - The table has no visible front. The hint under the crosshair names the face aimed at ("crafting table, left side: E to craft").
  - The crafted fence post turns freely like every shaped block, where the old one only turned its axis; it fills the same space.
  - *From play (2026-11-02):* drag-only filling couldn't reach scattered cells, like the middle four. Clicking something carried was meant to pick it as a brush. *Fixed the same day:* the drag-and-drop handler on each list entry swallowed that click, so the brush could only be set by dragging. Drag-and-drop is gone. Pressing an entry in the list picks it as the brush, and holding the button on the way to the grid fills every cell passed over, so picking and painting are one gesture. Clicking or sweeping cells fills them with the brush, the right button empties them, and right-clicking the list puts the brush down.
  - *From play (2026-11-02):* paying with material blocked patterns made from parts once the plain wood ran out. The user chose to make crafting **free**, with payment deferred. A craft takes no material; parts still take their volume of the capacity, so "Craft N" makes as many as fit, and with no room it's refused ("no room to carry it"). Placing a part still uses it, and breaking gives it back.
  - *From play (2026-11-02):* picked-up dirt couldn't be used. Every material's full block is now placeable (stone, dirt, ice, log and gravel join brick, wood and the table); grass, which breaks into dirt, and the marker stay unplaceable. The palette lists every material, carried or not, so brick is now slot 4 and wood slot 6, and parts come after slot 9.
  - *Deviation (2026-11-02, at the user's request):* **infinite carry.** `InventoryDef.capacity` is now optional (`Option<f32>`), and leaving `capacity` out of `inventory.toml` means no limit. The base game leaves it out. Placing still uses up material and parts, and the drop key still throws. The HUD says "carrying 32, no limit", the capacity slider leaves the tuning panel, and "Craft N" makes 64 at a time. This changes the Inventory and items toy's code as a design change, not a fix. The limit stays in the engine for mods, or for when a toy needs it again (see "Rethink the inventory" in the backlog).
  - *From play (2026-11-02):* parts you'd made couldn't be got rid of or told apart, and the hotbar gave slots to materials not carried.
    - Each part in the menu's Carried list now has **rename** and **discard** buttons. Discard throws all of that part away (`PlayerInput::discard`) and drops nothing. Names live in the client for the session (`editing::PartNames`, keyed by the part's block name); a blank name goes back to the automatic `wood ½ #3fa2`.
    - The hotbar lists tools, then only the materials and parts carried. If the slot in hand runs out, the hand stays on the last slot there is.
    - The hotbar is drawn with egui (`editing::hotbar`), with a picture for each slot (the material's color or the part's shape), its number, name and amount, and the selected slot outlined. It replaces the text line.
    - While the crafting menu is open, movement and debug keys are ignored, so typing a name doesn't walk or teleport, and the interact key doesn't close the menu while a name is typed.
- **API (`api`):**
  - `shape::ShapeGrid`: 16×16×16 bits, with `from_boxes`, greedy `to_boxes`, `turned`, `quarters` (`None` when there's detail finer than quarters) and `hash64`.
  - `crafting`: `TableFace` (from a normal in the table's frame, plus the grid's right and up for each face), `Cell` (`Empty`, `Solid`, `Shape`), `craft_shape`, `CraftError` (`Empty`, `TooFine`), `CraftingDef`, and a `pattern` helper for text rows.
  - `Registrar::register_crafting`, `WorldEdit::add_shaped`, and `ApiError::DuplicateCraftingDef`.
- **Engine (`sim`):**
  - `BlockRegistry::shaped(material, shape)` registers a shaped block the first time it's needed. It copies the material's full block, takes `Turning::Any`, and is named `<material>_s<16 hex>`. `find_shaped` looks one up without registering.
  - A shaped block names itself as its material, so breaking it drops one part. A part is a carried kind in the same ID space as materials (`MaterialId`): `part_of` gives what it's made of, and `unit` is its volume, so it's carried, picked up, pulled and thrown only whole.
  - The two shapes of one grid turned differently, such as stairs crafted at the front and at the side, are different parts, because they place differently relative to you.
  - `sim::crafting`: `Craft`, `plan` (one material, cells fit, pays the volume, as many as affordable) and `make`. `Simulation::table_face`, `craft_plan` (adds the table and reach checks), `last_craft` and `shaped_block`.
  - Crafting is sent with the tick (`PlayerInput::craft`) so it stays deterministic like the other inputs. Crafting turns material into the same volume of parts, so it never needs more room.
- **Data:** `mods/base/crafting.toml` (`table`), read by `data_mods::crafting` with errors pointing at the file and line. `blocks.toml` keeps only full blocks plus the crafting table (a planks look with a light top, `turning = "any"`). The `[shapes]` support stays for other mods.
- **Base:** `base::shapes` crafts the slab, stairs and post with the players' rule; the post takes two crafts, four corner parts shrunk into the middle cells. The test area builds them through `add_shaped`, and the table stands at `TABLE_SPOT` (-2, 3), its front facing south toward spawn.
- **Client:**
  - `client::crafting`: E at a table face opens the menu, and E, Escape, or walking out of reach closes it. Drag carried things onto cells; dragging across cells fills each, and right-click empties one. "Craft", shift-click or "Craft N" make parts.
  - The palette lists tools, then materials (their full block), then parts carried. The HUD shows parts as `wood ½ #3fa2 ×2`.
  - `Volumes::sync` picks up blocks registered while playing. The shape key is gone, `interact` (E) is new, and the user's controls file was updated to match.
- **Measured risk:** typical crafted shapes come to 2 to 32 boxes. The worst found, checkerboard parts in a checkerboard of cells, is 256 boxes and 1,536 faces for one block, against 6 for a cube. That's fine at toy scale. If walls of them get heavy, the fix is meshing only exposed faces from the shape grid.
- **Tests:** 342 pass across the workspace and clippy is clean.
  - Shapes: box round trips, merged boxes covering every sixteenth once, turning matching turned boxes, quarters, and hashes.
  - The crafting rule: a full grid, slab and stairs at every face, fine stairs, a pipe, material-only cells matching the plain prism, a pipe on its own axis and across (4 holes), too-fine parts refused, and the face frames.
  - The registry: one block per shape and material, parts made of their material, and only materials with a full block taking shapes.
  - Crafting costs, counts, mixed materials, and a part in a cell not being used up.
  - `crafting.toml` and its errors.
  - The base end to end (`sim/tests/crafting.rs`): which face is aimed at, including a turned table; craft, place, break and pick up; reach and shortage refusals; and a part in a cell.
  - Base shapes matching the old ones, the palette and HUD, look volumes for later blocks, the menu's pictures, and the interact key.
  - The game starts with no errors.

#### Inventory and items (branch `toy-inventory`)

**Stage:** Done (2026-11-01). Verified: the user played it and confirmed it works ("I verified the toy works"); the criteria weren't called out one by one in play and are covered by tests.

**Goal:** you carry materials, not blocks. Placing uses them up, everything you carry shares one volume limit, and you pick the shape when placing, so carrying many kinds of block costs nothing extra (the design doc's "inventory discourages variety" gripe). Builds on Loose items. The plan proposed on 2026-10-29 was never written down, so this one was rebuilt from the backlog's list of what's left.

**Approved plan** (2026-10-31, "engage, its ok": the suggested answers to the open questions stand)
- **`sim::inventory`:** an `Inventory` holding an amount per material in 1/4096ths of a boxel, the unit drops use, with one shared capacity. It replaces `Simulation::collected`.
- **Placing costs** the block's volume of its material (a slab ½, stairs ¾); without enough it's refused and the ghost shows red. Breaking gives material back through the drop, as now.
- **Full:** pickup takes only what fits and the rest stays on the ground as a smaller item; items aren't pulled toward a player with no room for them.
- **Drop key:** throws one full block of the held material (or what's left, if less) forward as a loose item, which the thrower can't pick up again for a short delay. Holding it keeps dropping, like holding break.
- **API:** `InventoryDef` (capacity in boxels, throw speed, pickup delay, starting stock by material name), registered by the mod, validated and tunable. Without one there's no inventory and placing is free, as today. `data_mods` reads an `[inventory]` table. `PlayerInput` gets `drop` and the material to drop.
- **Base:** capacity 64 boxels, starting with 32 brick and 32 wood (the test area has brick to break but no wood).
- **Client:** palette slots become tools, then materials (any material with a placeable block). A `shape` key cycles the held material's placeable shapes, remembered per material; scroll and digits still pick slots. The HUD shows each material's amount, the held shape, a fill line ("18½ / 64") and "full" when nothing more fits. New actions `drop` (Q) and `shape` (G), added to the user's controls file.
- **Tests:** placing costs the shape's volume and is refused when short; break, pick up and place again conserves the total; a full pickup takes what fits and leaves the rest; no pull when full; a dropped item waits out the delay before it's picked up; capacity is shared across materials; starting stock read by name, an unknown material reported with file and line; no `InventoryDef` means free placing; the palette lists materials and cycles shapes; end to end with `base`.
- **Out of scope:** weight, crafting, saving, building from storage within some range, pushing items out of placed blocks, an inventory screen.

**Done when**
1. Placing uses up material by shape volume and stops when you run out, with the ghost red.
2. Breaking and picking up gives the material back, and the HUD shows each material's amount and the shared fill.
3. You pick a material, then a shape: every brick and wood shape can be placed from one slot per material.
4. When full, items stay on the ground (partly picked up if some fits) and aren't pulled in.
5. The drop key throws material that lands as a normal loose item, collectable again after the delay.
6. Capacity and the drop values can be tuned while playing.
7. `cargo test` and `cargo clippy` are clean.

*Implementation notes* (2026-10-31)
- **Changed from the plan:** the base mod starts with 16 brick and 16 wood, not 32 of each. 32 + 32 fills the 64 capacity, so you'd start full and nothing could be picked up.
- **API:** `api::InventoryDef` (validated: capacity 0 to 100,000 boxels, the start no more than the capacity; tunable: capacity, throw speed, pickup delay), `Registrar::register_inventory`, `ApiError::{DuplicateInventoryDef, UnknownMaterial}`. Start materials are checked once every mod's blocks are registered.
- **Data:** `mods/base/inventory.toml` (`capacity`, `throw_speed`, `pickup_delay`, `[start]`), read by `data_mods::inventory`. A short material name must be one of the mod's own blocks' materials; mistakes point at the file and line.
- **`sim::inventory`:** `Inventory` keeps amounts by material in 1/4096ths of a boxel; the capacity comes from the definition each time it's needed, so tuning it takes effect at once. Lowering it below what's carried keeps everything but leaves no room. `Simulation::collected` is gone; use `inventory()` and `capacity()`.
- **Placing:** `placement()` refuses (and the ghost shows red) when the block's volume of its material isn't carried; a placed block takes it. Blocks with no material, and every block without an `InventoryDef`, cost nothing. The test-only `AllPlaceable` wrapper registers no inventory, so placement tests stay free.
- **Pickup:** `Collector` has a `room`; a body with none pulls nothing, and an item bigger than the room is picked up in part. `Item::wait` holds off pull and pickup; merging keeps the longer wait.
- **Throwing:** `PlayerInput::{drop_pressed, drop_held, drop_material}`. A full block's worth (or what's left) leaves 0.3 m below the eye along the look at `throw_speed` plus the player's own speed, repeating at the interaction rate while held. Needs item drops and an interaction definition too.
- **Client:** `editing::entries` lists the palette: tools, then each material with a placeable block (its placeable blocks are its shapes, in registration order), then placeable blocks with no material. `Palette` remembers each material's shape. The HUD shows `carrying 32 / 64` (with `FULL`), each material's amount in its slot, and the held shape with its keys. New controls `drop` (Q) and `shape` (G; held, the scroll wheel picks shapes), added to the user's controls file. The tuning panel has an Inventory section.
- **Tests:** 312 pass across the workspace, clippy clean. New: inventory sharing, taking and lowered capacity; partial pickup, no pull when full, and the throw delay in `sim::items`; placing costs and refusals (the ghost agreeing), free placing without an inventory, break and re-place keeping the total, full inventories leaving items, throwing up and catching after the delay, held drop, unknown start material, and the base game end to end (`sim/tests/inventory.rs`); reading `inventory.toml` and its errors; the palette's materials, shapes remembered per material, and the HUD text. The game starts with no errors.

*Ideas from play* (2026-11-01), sent to the backlog rather than built: left click places and right click breaks, and crouching to collect items. The user found the toy didn't give crafting what it needs ("I was thinking it would have drag and drop systems"), so the toy closed as built, and drag-and-drop goes into the Generative crafting plan.

**Closed** (2026-11-01)
- **Decisions:** the player carries materials, not blocks: an amount of each in 1/4096ths of a boxel, all sharing one capacity by volume. Placing costs the block's volume of its material, and a palette slot holds a material whose shape is picked with a key. Without an `InventoryDef`, placing is free. Items a full player can't take stay on the ground, partly picked up if some fits. Thrown items wait out a pickup delay.
- **Deferred:** an inventory screen and drag-and-drop (to the Generative crafting plan); weight; building from storage within some range; the button swap and crouch to collect (backlog); a big shape list makes the palette line long.
- **API debt:** none new; `InventoryDef` and `register_inventory` are in the API. Fixed input actions grew: `drop` and `shape`.


#### Combat feel (branch `toy-combat-feel`)

**Stage:** Done (2026-10-31). Picked by the user as a detour ahead of Inventory and items. Verified: the user played both rounds and closed it: "I think its as good as we can be informed of and need other toys to tune it correlty". Criteria 1 to 4 and 6 were played; 5 (the guard pose, blocks still editing) and 7 (the ghost) weren't called out separately in play and are covered by tests.

**Goal:** beating up a cow feels heavy and fun. Not an AI toy: targets have no behavior ("we just wan't to beat something with juice").

**Approved plan** (2026-10-31)
- **Cow asset:** a Luanti critter, at the user's request ("pull a luanti critter to abuse"), chosen by the user from the CC0 ones in TenPlus1's `mobs_animal`: sirrobzerrone's cow, with a CC0 moo. Converted once to a checked-in model, credited in `mods/base/CREDITS.md`. The Y-up to Z-up turn happens at the import step. This is outside art again after Solid materials dropped it, but only for a creature model; block looks stay procedural.
- **Targets in `sim`:** bodies with a box collider, health, weight, gravity, terrain collision, knockback, tumbling and friction until they settle. No behavior. Breaking is an event; a target comes back at its spot after a delay, or on a key.
- **API:** `TargetDef` by name (model, size, health, weight, sound), placed with `WorldEdit`; `ToolDef` for a melee tool (reach, damage, knockback, swing timing); palette slots hold a block or a tool.
- **Skyrim-style controls** (the user's direction): whatever is held has a main use on left click and a second use on right click. Blocks keep break and place. The tool swings on left click, with short reach that walls stop, and guards on right click ("I thougt it woud be a block"). Nothing attacks yet, so the guard is a visible pose only.
- **Juice in the client, all tunable:** hit-stop scaled by the hit, view and tool recoil (the robot's arm taking the shock), a white flash, squash and stretch, knockback scaled by weight, wind-up and follow-through, a whiff on a miss, a moo, and a burst of chunks when a target breaks.
- **Arena:** cows in the walled pen of the test area, with a waypoint.
- **Chase drill ghost:** kept, and hittable with the same juice: knocked back and dazed, never killed, no effect on the score ("the ghost can have some juice applied to him to").
- **The player can't die** for now ("infitine helth"); the heal-on-kill loop is deferred.
- **Tests:** reach and walls; damage, breaking and respawn; knockback by weight; tumbling and settling against terrain (walls, steps); a slot holding a tool versus a block; the cow loading through `base`; ghost hits.
- **Out of scope:** creature AI, player health, ranged weapons, sound beyond the moo, the cow's walk animation.

**Done when**
1. You can pick the weapon from a slot, and left click whacks a cow that is close enough.
2. Hits feel heavy (hit-stop, recoil, flash, knockback), as judged by the user playing it.
3. Cows tumble and settle properly against terrain and don't end up inside walls.
4. Cows die, burst apart and respawn.
5. Right click raises a guard you can see; blocks still break and place as before.
6. The juice values can be tuned in the Tab panel while the game runs.
7. Hitting the ghost during the drill knocks it back with the same juice, and the drill keeps working.

*Implementation notes* (2026-10-31)
- **Conversion:** assimp refused the cow's b3d file (its animation keys run past the animation's length), so `tools/b3d_to_obj.py` converts the rest pose to a Z-up OBJ in meters, without bones. The plan said `.glb`; OBJ is plain text and needs no glTF loader. Models and sounds reach the engine as plain data through the API (`ModelDef` with mesh and RGBA texture, `SoundDef` with Ogg bytes, `register_model`, `register_sound`), read by `data_mods` from a new `targets.toml`, so the client builds them like any other content and the simulation never looks inside.
- **Controls:** `break`/`place` became `use`/`alt_use` (old names still read, so personal files keep working); `PlayerInput` fields are now `use_*`/`alt_*`, plus `tool`. New key `respawn_targets` (F6), added to the user's controls file.
- **Swing in `sim`:** wind-up, strike, recover; the strike is aimed on the tick it lands, and solid blocks cut its reach. Hits are reported as `Strike` events the client drains. A solid block can't be placed into a target.
- **Hit-stop** pauses simulation ticks in the client (skipped, not made up). Fine for one player; multiplayer will need something else (deferred).
- **The cow's box** is 0.9 wide and 1.3 tall around the body; the head pokes out the front, so hits on the head rely on the tool's radius.
- **Tests:** 283 pass across the workspace, clippy clean. New: target physics (falling, settling, sliding further on ice, walls, breaking and respawn, knockback by weight, nearest target, the tool's radius), swing timing through the simulation with the base club and cows, walls cutting a strike, a tool swinging and guarding instead of editing, no block placed into a cow, models/sounds/targets read from `targets.toml` with file-and-line errors, editing the cow's numbers, the palette's tools before blocks, controls' new names, hit-stop and view kick, a hit tumbling a target away, and the ghost being knocked back and dazed. Checked by screenshot: the cow draws with its texture, flashes white on a hit and tumbles away; the club shows in hand.

*Plan change at Verify* (2026-10-31, "do your recommended set engage"). After first play: "the head has no colision, and the cows feel floaty like MC. so we want a combat system thats exprisive with a higher skill celing."
- **Hit boxes:** a target lists named boxes in its model's space (turned with its facing), each with a damage multiplier; swings test those, terrain collision keeps the simple box. Exact hits beat near misses, so aiming at the head gets the head.
- **Head hits:** the cow's head does double damage, with a bigger hit-stop, a different flash and a higher moo.
- **Less floaty:** air drag on targets, heavier gravity, more push and less lift; a hard landing squashes the target and kicks up dust; a **wall slam** (knocked into a wall fast) deals damage by impact speed with its own juice.
- **Momentum into the hit:** the player's speed toward the target adds to the knockback and the damage, so sprinting, falling or block-jumping into a swing hits harder.
- **Feedback:** the last hit's damage and why (head, carried speed, slam) shows under the crosshair; the Tab panel open draws the hit boxes.
- Deferred to later: swing direction from the mouse, air combos, the sweet spot, recovery cancels, charged swings.
- *Built* (2026-10-31): `api::HitBox` and `TargetDef::{hit_boxes, air_drag, slam_speed, slam_damage}`, `ToolDef::{momentum, momentum_damage}`, `[[target.hit_box]]` in `targets.toml`. Strikes test each box in the target's turned space; hits report the zone and the speed carried. Momentum is the player's speed along the look; moving away adds nothing. Slams are horizontal only and come out as `Slam` events. Hit boxes turn with the facing but not with the drawn tumble. Cow retuned: gravity 40, ground drag 9, air drag 3, slams from 4 m/s at 2 damage per m/s over; club knockback 22, lift 4, momentum 0.8, 0.5 damage per m/s. Landing dust and squash are client-only. 292 tests pass, clippy clean.

**Closed** (2026-10-31)
- **Decisions:** Skyrim-style controls: whatever is held has a main use (left click) and a second use (right click); tools come before blocks in the palette. Targets get named hit boxes with damage multipliers, separate from their collision box. Combat rewards aim (head hits), speed carried into a swing, and knocking targets into walls. Creature models may be borrowed CC0 art, credited in the mod's `CREDITS.md`; block looks stay procedural. Hit-stop pauses the simulation, which only works single-player.
- **Deferred:** final tuning, until toys with enemies, tools and movement give it something to be tuned against (the user's call); swing direction from the mouse, air combos, a sweet spot at full reach, recovery cancels, charged swings (backlog: Combat depth); what the guard does; hit boxes following the drawn tumble; targets colliding with the player and each other; hit-stop for multiplayer; the palette line wrapping into the trainer's label.
- **API debt:** new: tool look in the client. Grown: camera feel (hit-stop, kick, shake) and the chase drill (ghost hits).

#### Solid materials (branch `toy-solid-materials`)

**Stage:** Done (2026-10-31). Verified: the user played it and found it good, apart from plank board ends ("its a comprimise and we will mess with it later"). Criteria 1 and 6 met in play. On closing, the user accepted 3 and 5 without a separate check in play, and 2 and 4 as covered by tests: 2 can't be reached in play since the log slab is hidden, and 4's recipe edit and broken-recipe error are tested through the game's own loader.

**Goal:** blocks are drawn from a small 3D texture volume instead of 2D images on each face, so any cut through a block shows what's inside it, such as the rings of a cut log. Chosen by the user (2026-10-31), ahead of Inventory and items, because procedural and crafted shapes shouldn't need hand-made texture wrapping, and most block textures are just noise.

**Approved plan** (2026-10-31)
- **Every look is a recipe.** The Luanti textures go (`mods/base/textures/`, `CREDITS.md`, `LICENSE.txt`); the user prefers procedural looks to keeping borrowed art and its credits. `blocks.toml` gets `[looks.<name>]` recipes, and blocks name one with `look = "<name>"`. Generators: `noise` (speckle: stone, dirt, ice, gravel), `top` (a cover layer with a ragged edge, for grass), `bricks` (offset rows with mortar), `planks` (grain along boards with seams) and `rings` (a log's rings with a bark shell). Flat-color blocks like the marker stay.
- **Baked, not computed per pixel.** `data_mods` generates each look once at load into a 16×16×16 volume (one constant, to try 32 in play). The volume spans one boxel and repeats every block, and noise wraps at its edges so neighbors meet without seams. Recipes stay plain data, so a look needs no shader code from a mod; a later WebAssembly mod could add generators.
- **API:** `TextureDef` becomes `LookDef` (a name and a volume), `register_texture` becomes `register_look`, `BlockDef.textures`/`FaceTextures` become `look: Option<String>`, and `turn_textures` becomes `turn_look`. A block's color defaults to its look's average.
- **Client:** `volumes.rs` replaces `atlas.rs`: every look packed into one 3D texture (16×16×16·N), volume 0 plain white for flat blocks. The mesher drops UVs for a `solid` vertex attribute: the position inside the volume in sixteenths, moved half a texel inward from the face (done in Rust so it's tested), plus the volume index. Positions are world-aligned in the boxel, unless the block turns its look (logs, the fence post). A new unlit `SolidMaterial` with a small WGSL shader reads texels with `textureLoad`, so nothing bleeds between volumes; chunks, the ghost and loose items all use it. `sim` keeps only the registry; it never draws.
- **New blocks:** `log_slab` (turns its look) and `gravel` (noise).
- **Tests:** each generator repeats exactly from the same settings, uses only its colors and puts its pattern where it belongs (cover on top, bark on the shell, mortar between bricks), and noise wraps; the mesher's sample positions (a full block's top face reads layer 15.5, a bottom slab's top 7.5, two slabs in one boxel agree, turned slabs use world positions and turned logs their own); `data_mods` parses recipes and reports bad ones with file and line; `base_world` registers every look.
- **Deferred at the user's request:** grass when cut. Grass stays a full block only, with no slab or stairs; see Covered blocks in the backlog.

**Done when**
1. Every base block has a procedural look, and none of the Luanti textures are left.
2. A log slab shows rings on its cut face and bark on its sides, in every orientation.
3. Stairs, and two slabs sharing one boxel, look like one piece of material with no seam.
4. Changing a recipe in `blocks.toml` and restarting changes the look, and a broken recipe's error names the file and line.
5. Loose items and the ghost are drawn with the same looks.
6. The user finds the result looks decent.

*Implementation notes* (2026-10-31)
- Built as planned. Looks are generated in `data_mods::looks`; the client's `volumes` and `solid` modules (with `solid.wgsl`, embedded in the binary) replace the 2D atlas. The shader applies tonemapping itself when Bevy asks for it, as `StandardMaterial` did, so colors match the old pipeline's treatment.
- Recipe errors point at the recipe's `[looks.<name>]` line; unknown recipe fields point at the field itself.
- Tests: 246 pass across the workspace (generators, look registry, volume packing, mesher sample positions, loader errors, base world, editing a recipe changes the look). Clippy clean. The game starts and runs with no shader errors; how it looks is for the user to judge.

*Added at the user's request* (2026-10-31, during Verify, after first play): a plan change, approved with "engage".
- **Grout looked broken** where a whole face lined up with a mortar layer (a full block's west, south and bottom faces read layer 0, which was all mortar). Mortar now runs half a row up and a quarter brick in by default, and plank seams half a board in, so no face of a full block, slab or stair lands on a line. Bricks take an optional `offset`, and `mortar` is optional (bricks then differ by color only). Plank joints keep off the block's ends and middle. Tested: no face layer (0, 7, 8, 15) on any axis is mostly lines.
- **More shapes of fewer materials, for building a house:** brick gets `panel` (a 4-thick wall on the north side) and `window` (the panel with an 8×8 hole), joining its full block, slab and stairs. Wood gets a full block, slab, stairs, `fine_stairs` (four quarter-boxel steps, for gentler roofs) and a panel. Every shape is in sixteenths, so box edges fall between texels. Existing blocks stay, since the test area and tests use them.
- **Palette cut to the house set** (2026-10-31, the user's choice of two options, "A engage"): every other block is `placeable = false`, so the palette holds just the 10 brick and wood blocks. The rest stay in the game, building the test area. Tests that check placement with hidden shapes (stone slabs, logs, fence posts) load the base mod through a test-only wrapper that makes every block placeable (`sim/tests/common`); the block-jump trainer's tests place brick. Done criterion 2 (a log slab's rings) can't be checked in play any more; its sample positions are covered by the mesher test.
- **Turning is a rule of the material** (2026-10-31, at the user's request: planks shouldn't turn, logs should). The per-block `turn_look` flag is gone; a look recipe says `turns = true` instead (`LookDef::turns` in the API), and every block of that look follows it. Only the log's look turns. Planks now stay lined up with the world on every shape, so a wood panel or stair facing east or west shows board ends.

**Closed** (2026-10-31)
- **Decisions:** looks are data recipes baked into 16×16×16 solid volumes at load; no outside art (the design doc's "Borrowed assets" bullet is replaced). Whether a look turns with its blocks is a rule of the material. Mortar and seams keep off the layers faces show. The palette holds only the brick and wood house set for now.
- **Deferred:** plank board ends on east- and west-facing shapes (backlog); grass when cut (Covered blocks, backlog); the log slab is hidden, so its rings aren't seen in play; weak GPUs cap 3D textures at 2048 deep, so at most 127 looks in one stack, to split into several textures if that's ever reached.
- **API debt:** none new. Everything went through the API (`LookDef`, `register_look`, `BlockDef::look`).


#### Block-jump trainer (branch `toy-block-jump-trainer`)

**Stage:** Done (2026-10-30). Verified: the user played it and confirmed it works.

**Goal:** a trainer, toggled on with a key, that watches each block-jump attempt (placing a block overlapping your feet while jumping, so the push-out launches you) and tells you what went wrong. A detour at the user's request (2026-10-30): block-jumping feels inconsistent. A dev tool like the chase drill, not a feature; it doesn't change how block-jumping works.

**Approved plan** (2026-10-30)

*Why it's inconsistent (from reading the code):* placing and jumping on the same tick stacks the kick on the jump. Placing and then jumping within coyote time loses the kick, because the jump sets vertical speed to 0. With `hold_to_hop` on, holding jump after a kick from the ground does this by itself. Placing after the jump adds the kick to a jump that has already slowed. Placing once the feet are above the boxel gives no push-out. Standing off-center tilts the kick, and walls or ceilings cancel it.

- `sim`: read-only diagnostics, no behavior change. `Player::last_step()` records the tick's velocity going in, the controller's velocity and impulse, and the external impulses. `Simulation::last_place()` records the last placement attempt (tick, boxel, allowed, the push-out it caused).
- `client::trainer`: a plain `Trainer` fed one observation per tick. It groups ticks into attempts (start on a jump or a placement, end on landing) and gives each attempt one verdict with the timing in ticks and ms, the kick lost, tilt, wall or ceiling hits, and the height reached. The HUD shows recent verdicts, counts and the best height. Toggle key `trainer`, default F5.
- Tests: scripted input on a headless `Simulation` with `base` for each case (same tick, kick lost, held hop, late, too late, refused, missed, kick only).
- Out of scope: changing push-out or the jump. That stays in Advanced movement revisited.

*Test* (2026-10-30): 9 trainer tests pass; the workspace passes and clippy is clean. Heights from the scripted runs, feet above the floor: same tick 3.75 m, placed 8 ticks after the jump 3.19 m, jump 3 ticks after placing (kick lost) 1.48 m, too late 1.18 m, kick only 0.54 m. Losing the kick costs over half the height, from a 50 ms timing slip.

*Result:* the trainer did its job. It showed that the inconsistency comes from how block-jumping is built, not only from the player's timing, and the redesign ideas that came out of it are in the Advanced movement revisited entry. *API debt:* one new entry, Block-jump trainer in the client.

#### Loose items (branch `toy-loose-items`)

**Stage:** Done (2026-10-30). Verified: the user played it and confirmed all four criteria, including tuning in the panel ("I did that to").

**Goal:** breaking a block drops a loose item of what it's made of, which pops out, falls, bounces and settles, and is pulled in and picked up when you come close. Split out of Inventory and items at the user's request (2026-10-29), Minecraft-style: items fall on the ground rather than going straight into an inventory.

**Approved plan** (2026-10-29)

*Decisions*
- **Items are an amount of a material.** A block names its material (`stone_slab` and `stone_stairs` are `stone`, `grass` is `dirt`); breaking it drops its volume of that material, worked out from its shape boxes in 1/4096ths of a boxel (16³), so a slab drops ½ and stairs ¾. Whole numbers, no float drift. In `blocks.toml` a block's material defaults to its own name.
- **Loose items are small bodies** in a plain list in `sim`, not a general entity system: gravity, collision against block boxes with the player's sweep, a bounce, and ground drag. Drawn as a small cube of the material's first full block, spinning, sized by amount.
- **Pickup is a pull** (the user agreed, 2026-10-29): within a radius of the body, items drift toward it, and close enough they're picked up. Picked-up amounts go into a plain counter per material, shown on the HUD. No capacity and no cost yet; the palette stays free.
- **Merging** (the user agreed): loose items of the same material within about a boxel merge into one.
- **No despawning** (the user's call, 2026-10-29): items stay until picked up. When despawning proves necessary, add a despawn time where 0 means never, and leave it at 0.
- **API:** `BlockDef.material`; `ItemDropDef` (pop speed, gravity, bounce, drag, size, pull radius and speed, pickup radius, merge radius), registered by the mod and tunable while running. Without one registered, breaking drops nothing.

*Tests:* volume from shapes (overlapping boxes counted once); materials and each material's drawn block; reading `material` from data; an item falls, bounces less each time and settles; pulled in and picked up; merging; the same at millions from zero; breaking a slab with the base mod drops half a stone that ends up in the counter; nothing drops without an `ItemDropDef`.

*Out of scope:* capacity, placing using up material, picking a shape, a drop key, crafting, saving, despawning. These are Inventory and items (next).

*Done when*
1. Breaking a block drops an item of its material, sized by how much, which pops out, falls, bounces and settles.
2. Walking near items pulls them in and picks them up, and the HUD counts what you've collected per material.
3. Nearby items of the same material merge.
4. The item values can be tuned while playing.
5. `cargo test` and `cargo clippy` are clean.

*Implementation notes* (2026-10-29)
- **API.** `BlockDef.material` with `with_material`, checked as a namespaced name; `api::ItemDropDef` (tunable, validated); `Registrar::register_item_drops`; `ApiError::DuplicateItemDropDef`. `blocks.toml` has `material`, defaulting to the block's own name; in base, grass is `dirt`, slabs and stairs are `stone` or `brick`, the fence post is `wood`.
- **Materials in the registry.** `BlockRegistry` gives each material a `MaterialId` in the order blocks first name it, and keeps what each block drops (`Drop { material, amount }`, from `volume(shape)`, overlaps counted once) and the block each material is drawn as (its first full block). `FULL_VOLUME` is 4096.
- **`sim::items`.** `Items` is a plain list of `Item` (center position, previous position, velocity, on-ground, pulled), with IDs that are never reused. Items sweep with the player's `sweep_box` (made `pub(crate)`), bounce above 1 m/s and stop below it, and slow down sideways on the ground. A `Collector` (the player's box) picks up within `pickup_radius` and pulls within `pull_radius`; pulled items ignore gravity and turn toward the body's middle. Merging is all-pairs each tick, into the item holding more. An item's size follows the cube root of its amount, from half to twice `size`.
- **Simulation.** Breaking an occupant drops its material from the middle of its boxes, popped up with a sideways direction from a hash of tick and position, so it's the same on every machine. Items step after the player each tick; picked-up amounts go into a counter per material (`Simulation::collected`).
- **Client.** `client::items` draws each item as its material's block, meshed around its center, blended between ticks, turning and bobbing upward while it rests. The HUD line above the palette lists what's been picked up. The tuning panel has an Item drops section.
- **Not handled:** a block placed over a resting item leaves it inside the block, still collectable. No new controls, so the controls file is unchanged.

*Plan change at the user's request* (2026-10-29, before playing): "lets not copy MC so much". The slow spin and soft bob were Minecraft's, so items become **debris** instead (option A of three offered; the others, a robot magnet pickup with a beam and crushed pellets, are not built):
- No spin or bob; a resting item lies still.
- In the air it tumbles about the axis it popped along, faster the harder it popped. On landing it turns the short way onto a face so it lies flat, with a quick squash, and bounces squash too.
- It has the shape of what broke: a slab drops a small slab, stairs small stairs, all at one scale. The collision box is that shape's bounds, so a small slab lies low. Merged items become the material's own block.
- The tumble and squash are client-only; the simulation keeps each item's shape and box.
- *Built:* `Item` keeps the block it looks like, that shape's bounds and volume; `Item::scale` replaces `size` and `Item::extent` gives the box. Merging switches the kept item to its material's block. `client::items` gives each item an outer entity (position, and a squash along the world's up that keeps its bottom on the ground) and an inner one (tumble and scale). Landing turns it to the nearest turn that keeps the box's size on every axis, so a slab lands flat either way up and a cube on any face. Tests: a slab's item has a slab's box and rests a quarter of its size up; two slabs merge into a stone cube; grid turns match render rotations; a slab rests only flat.
- *Fixed in this toy* (2026-10-29, found by the user in play): a resting item that merged grew about its middle, so its bottom went into the floor, and since collision ignores boxes an item already overlaps, it sank through. A merged item now grows up from where its bottom was, then is lifted out of anything it still overlaps, by at most a boxel. Test: two resting slab items, and two cubes, merge and still rest on the floor.
- *Changed in this toy at the user's suggestion* (2026-10-30, found in play): a grown item's box could reach into a wall beside it, rest partly on the wall's lower block and hang in the air when its own floor was broken. **Growth is now only drawn:** an item's box is always one block's worth of its shape, and the client draws it up to twice as big, growing up from the box's bottom (`Item::growth`). The lift out of overlaps was removed again; the box still keeps its bottom when merging changes its shape (slab to cube). Test: an item resting against a tall wall merges, its box stays the same size and clear of the wall, and it falls when the block under it breaks. Trade-off accepted: a big merged item can be drawn poking into a wall by up to about 12 cm, and fits through gaps by its box. Still open: a block placed over a resting item hides it inside the block (backlog, unless it bothers the user in play).
- *Fixed in this toy* (2026-10-30, found by the user in play): each new item flashed at the render origin for one frame. Its entities were spawned with default transforms and then set through a query, which can't see entities spawned in the same system. They're now spawned with their transforms. Test: the client's first Bevy app test runs `sync_items` once after breaking a block and checks the item is drawn where it is.

*Verification* (2026-10-30)
1. Breaking drops an item of its material, sized by amount, that pops out, falls, bounces and settles: met (played; tested).
2. Walking near items pulls them in and picks them up, and the HUD counts each material: met (played; tested).
3. Nearby items of the same material merge: met (played, after the visual-growth change; tested).
4. The item values can be tuned while playing: met (played).
5. `cargo test` and `cargo clippy` are clean: met (230 tests).

*Result*
- **Decisions:** blocks name a material, and breaking drops the block's volume of it in 1/4096ths of a boxel; loose items are a plain list of small bodies in `sim`, not an entity system; items are debris shaped like what broke, tumbling in the air and settling flat with a squash, not Minecraft's spin and bob; the collision box is one block's worth of the shape, and merging only changes how big it's drawn; pickup is a pull within 2 m; same-material items within 1 m merge; no despawning.
- **Deferred:** a block placed over a resting item hides it inside the block (backlog); a big merged item can be drawn poking into a wall; the tumble, settle and squash strengths are client constants, not tunables; despawning, as a time where 0 means never, when it proves necessary.
- **API debt:** none new. `ItemDropDef` and `BlockDef.material` are in the API; the base mod uses them like any other mod.

#### Block shapes and shared cells (branch `toy-block-shapes`)

**Stage:** Done (2026-10-28). Verified: the user played it and reported every criterion met ("everthing works all critearia met").

**Goal:** slabs, stairs, fences and logs placed in several ways depending on where you aim, and two things sharing one cell, like a fence and a slab. This is the engine's general form of Minecraft's waterlogging: the design doc's "multiple occupants per boxel".

**Approved plan** (2026-10-28)

*Decisions*
- **A cell holds a short list of occupants,** each a block plus an orientation. The blocks layer stores a small ID per cell that points into a shared table of occupant lists, and identical lists share one entry. Most cells still hold one occupant, so memory and the uniform-chunk shortcut stay about where they are. Water later becomes just another occupant.
- **Orientation is one of the 24 grid-aligned rotations of a cube.** It covers top and bottom slabs (a flipped slab), vertical slabs, stairs facing any way or upside down, and logs along any axis. Slopes and free angles are deferred.
- **Shapes are data.** A block lists boxes in 1/16-cell units; a full cube is the default. `blocks.toml` gets shared named shapes (`slab`, `stairs`, `post`) and which orientations each block allows.
- **Where a placed block goes:** into the cell holding the point just in front of where you aim. Aiming at the top of a bottom slab puts a top slab in the same cell (a double slab); aiming at a fence post puts a slab in the fence's cell; aiming at a full block puts it in the next cell, as now.
- **How it's turned:** each block names one of a few engine placement rules: fixed; axis from the clicked face (logs); facing away from you, upside down when aiming at the cell's upper half (stairs); top or bottom half from where you aim (slabs).
- **Sharing rule** (the user agreed, 2026-10-28; loose for now, tightened after play): up to 4 occupants per cell, a full cube shares with nothing, and the same block can't be in a cell twice with the same orientation. Overlap is allowed, so a fence post pokes through a slab.
- **Ghost preview:** a see-through copy of exactly what will be placed, red when it isn't allowed.
- **Collision, aiming and breaking use the real shapes.** Bodies collide with each occupant's boxes (stand on a slab at half height, step up stairs with the existing step height). Aiming finds the occupant and box you look at, and breaking removes only that occupant. The placement check against the body uses the new block's boxes, not the whole cell.
- **Meshing** draws each occupant's boxes, turned by its orientation. A face is hidden only when the neighboring cell holds a full cube.

*Content (`mods/base`):* stone slab and stairs, brick slab and stairs, a wooden fence post and an oak log, with wood and log textures from Minetest Game (the user approved the download, 2026-10-28) and credits. The test area gets a fence sharing a cell with a slab, a staircase and a sideways log.

*Tests:* occupant lists and the shared table; the 24 orientations (composition, turning boxes); the sharing rule; aiming at partial blocks and picking the right occupant; placement target and turning for each rule; standing on a slab and walking up stairs; breaking one occupant leaves the other; hidden faces next to partial blocks; parsing shapes from data files.

*API:* `BlockDef` gains shape, allowed orientations and placement rule; `api` gains orientation and occupant types. **New API debt:** placement rules are a fixed engine set until mods can run code for block behavior.

*Out of scope:* liquids, fences connecting to neighbors (the user agreed to leave it, 2026-10-28), slopes and free angles, smaller-than-boxel detail, the material-plus-shape palette, items and inventory. The current palette stays and gains the new blocks.

*Done when*
1. You can place and break slabs, stairs, a fence and a log, and each turns as expected from where you aim.
2. A fence and a slab can share a cell, two slabs make a double slab, and breaking one occupant leaves the other.
3. The ghost always matches what gets placed, and turns red when the block isn't allowed.
4. You can stand on slabs and walk up stairs without jumping, and can't walk through partial blocks.
5. No gaps or extra faces where partial blocks meet full ones, including across chunk borders.
6. `cargo test` and `cargo clippy` are clean.

*Implementation notes* (2026-10-28)
- **Storage.** `sim::cell`: `Occupant { block, orientation }`, `CellId`, and `CellTable`, one per grid, storing each sorted list of up to 4 occupants once, in place (no heap per list). `Chunk::cells` replaces `Chunk::blocks`. `Grid::cell`, `set_cell` and `occupants` read and write lists; `block` and `set_block` remain for single blocks. World building got about 10% slower than before (base world tests 3.5 s to 3.9 s) after a first version that allocated per boxel doubled it.
- **Orientation** is `api::Orientation`, a 3×3 matrix of -1, 0 and 1, with `quarter_turns`, `z_along`, `then`, `apply`, `apply_box` and `all`.
- **Shapes.** `api::BlockBox` (sixteenths) and `BlockDef.shape`, checked at registration. In `blocks.toml`: a `[shapes]` table of named shapes, `shape = "name"` or a list of `[x0, y0, z0, x1, y1, z1]` boxes, and `placement`.
- **Collision** works on boxes: `Terrain::boxes` replaces `Terrain::solid` (still there as a helper), and `clamp_axis` and `box_clear` test boxes instead of whole boxels. For full blocks it gives the same numbers as before; all movement and wasm equivalence tests pass unchanged. Push-out uses the box around the placed block's boxes.
- **Aiming.** The raycast tests each box in the boxels it walks and reports the occupant, the face normal and the exact point. Breaking removes only that occupant.
- **Placement.** `Simulation::placement(hit, block, yaw, pitch)` returns a `PlaceTarget` (boxel, turned occupant, allowed), so a refused placement still says where it would go.
- **Content.** Stone and brick slabs and stairs, a wood fence post and a log, with Minetest Game's `default_wood` (BlockMen), `default_tree` and `default_tree_top` (Cisoun) credited in `mods/base/CREDITS.md`. The shapes corner west of spawn has a staircase up to a platform, a brick slab path with posts through two slabs, a lone post, a double slab, and logs lying and standing. The debug teleport has a new `shapes` waypoint at its foot.
- **Client.** The mesher draws each occupant's turned boxes with textures cropped to each box, and hides a face only when it's on the boxel's side against a full block. The outline traces the aimed occupant's boxes. The ghost is a see-through mesh of the placed occupant, slightly shrunk so it doesn't flicker against neighbors, red when refused.
- *API changes:* `Orientation` (with `from_axes`), `BlockBox`, `Turning`, `BlockDef.shape`, `turning`, `turn_textures`, `with_shape`, `with_turning`, `with_turn_textures`, `is_full`, `validate`, `WorldEdit::add_block`, `ApiError::CannotShare`.
- 208 tests pass in the workspace after the third plan change and the flicker fix; clippy is clean.

*Changes from the plan, found while building* (each small; flagged for the user)
- **Step height 0.6 in `base`.** The plan said stairs would use "the existing step height", but the base player's was 0, so stairs couldn't be walked up. 0.6 is Minecraft's: enough for slabs and stairs, not for full blocks.
- **Fallback to the next boxel.** When the block can't share the boxel in front of the aimed point (a full block aimed at the top of a slab), it goes into the boxel on the far side of the face instead, as in Minecraft. Without this you couldn't build on a slab.
- **No API type for reading a boxel's occupants.** Nothing in a mod needs to read the world yet; mods add turned blocks with `WorldEdit::add_block`.

*Plan change at the user's request* (2026-10-28, after a first play)
- **One placement rule for every block,** replacing the four per-block rules (fixed, along face, facing away, half). A block's bottom goes against the face aimed at and its front (+Y) turns the way the player looks, rounded to the nearest axis along that face. Each block says how much of that turn it keeps with `turning`: `none` (plain blocks), `axis` (logs) or `any` (slabs, stairs). On a floor a slab is a bottom slab, on a ceiling a top slab, on a wall a vertical slab; stairs rise away from you on a floor, hang upside down from a ceiling and turn sideways on a wall. Aiming at the upper half of a wall no longer makes a top slab, and a slab aimed at the top of a slab goes into the boxel above; a double slab comes from aiming at a ceiling over a slab.
- **The planned API debt is gone:** there's no fixed set of engine placement rules any more, only the one rule and each block's `turning`.
- **Textures stay lined up with the world by default,** so bricks run level on a vertical slab or upside-down stairs, and a slab's side shows the same strip as the full block beside it. `turn_textures = true` turns them with the block instead; only the log uses it. The user raised the worry that turned textures would look wrong.
- **Duplicates compare the space filled,** not the orientation, so a slab turned about its own axis can't stack onto the same slab.
- The fall back to the boxel on the far side of the face stays.
- *Fixed on its own branch first:* the camera dip and snap when stepping up stairs (`fix-step-smoothing`, see Fixes), merged into this branch.

*Second plan change at the user's request* (2026-10-28, after a second play)
- **A flip key** (`flip`, middle mouse by default, added to the user's controls file). It switches which side of a block goes against the aimed-at face: its bottom (the normal rule) or its back (+Y). Flipped on a wall, a block stands upright, or upside down when aimed at the upper half, so a slab sticks out level from the wall and stairs stand with their tall back to it; flipped on a floor or ceiling, its top points the way the player looks. The HUD shows which side is in use. Crouch was considered and rejected: it already sinks the robot and holds it at edges, and one input should do one thing.
- **A slab aimed at the top of a slab fills the top half,** whatever the material. General form: a face inside a boxel is aimed at from that boxel, so a block placed into that boxel goes against its far side, facing back toward the face. A slab aimed at a fence post's side fills the far half of the post's boxel.
- **No two blocks fill exactly the same space,** whatever they're made of (before, a brick slab could land in a stone slab's space).
- **The fence post can lie on its side** as a beam (`turning = "axis"`, grain turning with it).
- **Why double slabs are here:** mainly so the engine visibly supports several blocks per boxel from the start, which the user sees as a fundamental Minecraft limitation. How players reach the feature in the long run is deferred (see the Interaction controls backlog item).
- **Fixed in this toy:** crouched on the stairs, wiggling could slide the body off the staircase's side. The edge guard allowed the half-boxel step down onto a sliver of support, then switched off during that short fall, so the body drifted off the sliver. A fall the guard allowed is now guarded until landing, with what's left of the allowed drop. Only partial blocks make such small drops, so it belongs to this toy. Test: wiggling crouched on the stairs with 8 random patterns never falls more than the guard allows.
- *API debt:* `flip` joins the fixed input actions (renamed `placement_mode` in the third change).

*Third plan change at the user's request* (2026-10-28)
- **Two placement modes replace the single rule and the flip key.** The same key (middle mouse) switches between them, renamed `placement_mode` in the code and the user's controls file. The HUD shows the mode.
  - **Basic** (default, Minecraft-like): blocks stand upright with their front the way the player looks, upside down against a ceiling or when aimed at the upper half of a wall. Blocks that only turn their axis (log, post) line up with the face.
  - **Precise:** every face is split into five parts: a square in the middle (half the face's width) and four triangles around it, drawn corner to corner. The middle puts the block's bottom against the face, its front the way the player looks along the face (the earlier single rule). A triangle stands the block in the half of the boxel toward that edge, back against the face: on a wall the top and bottom triangles make level top and bottom slabs and the side triangles stand slabs against that side; on a floor the triangles stand slabs against the north, south, east or west side. From one face a slab can reach every half of the boxel.
- **A region guide in precise placement** (asked for after playing): the aimed-at face shows its middle square and diagonals, with the part the crosshair is in picked out in yellow. It uses the simulation's own `face_region`, and a test checks the part drawn always holds the aimed point. No outline on the ghost: the user asked for nothing new beyond the guide.
- **No flicker where faces coincide** (the user's idea, 2026-10-28; the user chose oldest wins). Boxels now keep their occupants in placement order instead of sorted, so the same occupants in another order are another table entry. Where faces of two occupants of a boxel lie on the same plane facing the same way, the mesher cuts the shared part out of the newer one's face (faces are rectangles on the 1/16 grid, so the cut is exact), and the oldest occupant's face shows. Breaking the older one brings the newer one's whole face back. Tests check no overlapping coplanar quads are left in either order, which texture shows, and that the face returns. `api::Orientation` gained `inverse`.
- The rest of the second change stays: a slab on a slab fills the top half (in both modes), no two blocks in the same space, the fence beam, the crouch fix.

*Deferred at the user's request* (2026-10-28): the ghost preview still flickers against faces it touches, despite being drawn slightly smaller. Not fixed in this toy.

*Noticed while testing:* standing on a slab, a fence post is only 0.5 above your feet, so the 0.6 step height walks you onto the post. Minecraft makes fence collision 1.5 high for this; worth deciding when fences join their neighbors.

*Verification* (2026-10-28)
1. Slabs, stairs, the fence post and the log place, break and turn as expected: met (played, in basic and precise placement).
2. A fence and a slab share a boxel, two slabs make a double slab, breaking one occupant leaves the other: met (played; tested).
3. The ghost matches what gets placed and turns red when refused: met (played). Its flicker against touching faces is deferred.
4. Standing on slabs, walking up stairs without jumping, partial blocks block movement: met (played after the step-smoothing fix; tested).
5. No gaps or extra faces where partial and full blocks meet, across chunk borders too: met (played; tested), with coinciding faces in one boxel drawn once.
6. `cargo test` and `cargo clippy` are clean: met (208 tests).

*Result*
- **Decisions:** a boxel holds up to 4 occupants (block plus orientation) in placement order, interned per grid; 24 grid-aligned orientations; shapes are boxes in sixteenths, defined in data; collision, aiming and breaking work per box; two placement modes, basic (Minecraft-like) and precise (five parts of the face), on one key; textures lined up with the world unless a block turns them; the oldest occupant's face shows where faces coincide; step height 0.6.
- **Deferred:** the ghost's flicker against touching faces; fences joining their neighbors, and whether fence collision should be 1.5 high (a slab-standing body steps onto a post); how players reach shared boxels in the long run (in the Interaction controls backlog item); slopes and free angles; an API type for reading a boxel's occupants; whether a stairs block is needed at all (two slabs at right angles make one; see Emergent shapes from crafting).
- **API debt:** placement mode joins the fixed input actions. The planned debt for a fixed set of engine placement rules never happened: the rules are engine-owned modes, and blocks choose only how much they turn.

#### Sandbox limits seam (branch `seam-sandbox-limits`)

**Stage:** Done (2026-10-28). Verified: the user played it and movement feels unchanged ("its good"). *Deviation (2026-10-27):* at the user's request, Data mods was planned and built while this waited at Verify, since the user couldn't play right then.

**Goal:** a WebAssembly mod that loops forever, allocates without end or recurses too deep is stopped, and the game carries on, the same way it does when a mod panics. The point is to stop a broken or hostile mod from hanging the game or using up its memory, without changing how a working mod behaves.

**Approved plan** (2026-10-27)

*Decisions*
- **Fuel, not epoch interruption.** Fuel counts WebAssembly instructions, so a mod runs out at exactly the same point on every machine, which keeps replays and multiplayer deterministic. Epoch interruption uses the wall clock and could stop a mod on one machine but not another. Fuel's cost (roughly 10–30% in tight loops) is negligible for a movement controller.
- **Limits are per loaded mod.** Controllers a mod spawns share its one instance, so they share one memory cap. Fuel is per call: the engine refuels to a fixed budget before every call (`step`, `spawn`, `tunables`, `set-tunable`) and before loading (`name`, `create`), so one call's cost never depends on earlier calls.
- **Choosing the fuel budget.** A test measures the Quake controller's heaviest step over the 10,000-tick equivalence run, and the default is set to about 100 times that. A test checks Quake stays under a tenth of the budget.
- **Memory cap:** wasmtime's `StoreLimits`, with a 256 MiB default linear memory per mod as a runaway guard, plus limits on tables and instances. Hitting it makes the guest's allocation fail, and the guest aborts with a trap.
- **Stack depth:** set wasmtime's `max_wasm_stack` explicitly; overflow already traps.
- **Any limit hit is handled like a panic.** The mod is marked dead, a message naming the limit is printed once, and velocity passes through unchanged. Running out of fuel mid-call can leave the instance half-updated, so it is never called again.
- **No API change.** Everything lives in `wasm_host`: a `Limits` struct (fuel budget, memory cap, stack size). `load_movement(bytes)` keeps the defaults; `load_movement_with(bytes, limits)` lets tests use small budgets.

*Crates*
- `crates/wasm_host`: fuel and memory settings on the `Config` and `Store`, refuelling before each call, `Limits`, clearer stop messages.
- `crates/test_mods/hostile_movement` (new): a fixture controller whose `behavior` tunable picks what goes wrong in `step`: loop, allocate or recurse forever. Built for tests only, like `panicky_movement`.
- `crates/game/build.rs`: add the fixture to the crates compiled to WebAssembly.
- `crates/game/tests/wasm_movement.rs`: the new tests.

*Tests*
- A looping `step` is stopped by fuel; the controller reports itself dead and velocity passes through.
- Endless allocation hits the memory cap and stops the controller.
- Endless recursion hits the stack limit and stops the controller.
- A loop in `spawn` or `tunables` is stopped, and a component that loops while loading fails to load with an error instead of hanging.
- Two controllers spawned from the same mod share one memory cap.
- Fuel is deterministic: the same inputs use exactly the same fuel across two loads.
- Quake's heaviest step stays under a tenth of the budget, and the bit-for-bit equivalence tests still pass.

*Done when*
1. Endless loops, endless allocation and endless recursion in a mod each stop that mod without hanging or crashing the game, with tests for all three.
2. Loading a component that loops forever returns an error.
3. Quake movement uses less than 10% of the fuel budget, and the equivalence tests still match bit for bit.
4. The user runs the game and movement feels unchanged (a quick check; nothing visible changes).
5. `cargo test` and `cargo clippy` are clean.

*Deferred:* a wall-clock backstop (epoch interruption); one fuel budget per mod per tick, which matters once a mod runs many things per tick; a global memory total across all mods; restarting a dead mod from a fresh instance; player-approved permissions for host services.

*Decided:* a stopped mod stays dead, since restarting needs a fresh instance and loses the controller's state.

*Result*
- All five "Done when" criteria met. The user checked 4 in game. After merging `master` (the data mods seam) into the branch, 164 tests pass in the workspace, and clippy is clean.
- *API debt:* none new, and none paid down. The limits live entirely in `wasm_host`.

#### Data mods seam (branch `seam-data-mods`)

**Stage:** Done (2026-10-27). Verified: the user played it, saw the textures, changed `blocks.toml` and restarted, and saw a broken file's error name the file and line.

**Goal:** the base game's blocks and textures come from files in a mod folder, loaded at startup. Editing a block's properties or swapping a texture needs a restart, not a rebuild. Flat colors give way to Minetest Game textures, with credits and license kept.

**Approved plan** (2026-10-27)

*Decisions*
- **TOML,** parsed with `serde` and `toml`. Unknown fields are errors, so typos aren't silently ignored. Errors name the file, line and field.
- **Layout:** `mods/base/` holds `mod.toml` (name, version, description), `blocks.toml` (`[[block]]` entries: name, solid, friction, placeable, textures), `textures/*.png` (16×16), `LICENSE.txt` and `CREDITS.md`.
- **A data mod is a normal `Mod`.** A new engine crate, `data_mods` (no Bevy), reads a folder into a `DataMod` that implements `api::Mod` and registers through the same `Registrar`. The base game is two parts sharing the `base` namespace: data (`mods/base`) and code (the `base` crate, which still builds the test area and registers the player and interaction rules).
- **Textures are plain data in the API:** `TextureDef { name, width, height, rgba }`, `Registrar::register_texture`, and `BlockDef.textures: Option<FaceTextures>` (top, bottom, side). `color` stays for untextured blocks. The loader decodes PNGs (`png` crate), so `sim` only stores bytes.
- **Rendering: one atlas,** nearest filtering, no mipmaps. The mesher adds UVs and picks top, bottom or side per face; vertex colors keep the face shading. Keeps `StandardMaterial`. An array texture (for tiling with greedy meshing) is deferred.
- **Finding mods:** `game` loads `<workspace>/mods/base`, or the folder given with `--mods <dir>`. Loading every folder with dependency order is deferred until a second data mod exists.
- **Textures** from Minetest Game's `default` mod: stone, dirt, grass top and side, brick, ice. The marker keeps its flat color. Credits per file, CC BY-SA 3.0.

*Crates:* `api` (texture types, `register_texture`); `data_mods` (new: parsing, PNG decoding, errors, `DataMod`); `sim` (texture registry, checks that named textures exist); `base` (blocks move to `mods/base/blocks.toml`, name constants stay); `client` (atlas, UVs); `game` (loads `mods/base` before `BaseMod`, `--mods`).

*Tests:* parsing a good file, an unknown field, a missing texture, a bad name and a malformed PNG; the base data mod registers the same six blocks with the same properties as before; atlas textures don't overlap and UVs stay inside their texture; grass gets top, side and bottom textures on the right faces and untextured blocks keep flat colors; existing tests, including wasm equivalence, still pass.

*Done when*
1. The base game's blocks are defined in `mods/base/blocks.toml`; the `base` crate registers no blocks in code.
2. The game shows Minetest textures, with grass top and side distinct.
3. Changing a value in `blocks.toml` (such as ice friction) and restarting changes the game with no rebuild.
4. A broken data file gives a readable error naming the file and line, not a panic deep in the engine.
5. `LICENSE.txt` and `CREDITS.md` credit every texture's authors.
6. `cargo test` and `cargo clippy` are clean.

*Setup:* the user approved downloading the textures, license and credits from the Minetest Game repository.

*Implementation notes* (2026-10-27)
- **Short names in data files.** `blocks.toml` names blocks without a namespace; the loader adds the one from `mod.toml`, so a data mod can't define blocks in another mod's namespace. Texture names may be short (this mod's `textures/`, checked at load with file and line) or full (another mod's, checked by the engine once every mod has registered).
- **Color is optional for textured blocks.** It defaults to the side texture's average color, kept for anywhere a block is drawn as one color (later LOD). Untextured blocks must give one.
- **Grass side is composited ahead of time.** In Minetest Game, `default_grass_side.png` is a see-through overlay drawn over dirt. Instead of adding texture layering to the engine, `mods/base/textures/grass_side.png` is the two combined into one opaque image, and `CREDITS.md` says so, as CC BY-SA requires.
- **Loading.** `game::load_from(dir, movement)` loads the data half from `dir`, then `BaseMod`; `game::load` uses `base::DATA_DIR` (the source tree's `mods/base`). A load error prints and exits instead of panicking. Tests outside `game` load the data half with `data_mods::load(base::DATA_DIR)`.
- **Atlas.** Equal cells sized to the largest texture, each with a one-pixel border repeating the texture's edge, plus a white cell for flat-colored blocks, which tint it with vertex color. Built in `client::atlas`, kept in `ChunkMeshes`.
- **Dependencies:** `serde`, `toml` 1.x, and `image` with only PNG (the version Bevy already uses).
- *API changes:* `TextureDef`, `FaceTextures`, `BlockDef.textures` and `with_textures`, `Registrar::register_texture`, `ApiError::UnknownTexture`. In `sim`: `TextureRegistry`, `Simulation::textures`, `BlockId::index`.

*Result*
- All six "Done when" criteria met. The user checked 2 to 4 in game (textures with grass top and side distinct; ice friction and a grass texture changed by editing `blocks.toml` and restarting; a misspelled field gave an error naming the file and line). 156 tests pass in the workspace, and clippy is clean.
- *Deferred:* an array texture instead of the atlas (needed for tiling once greedy meshing exists); loading every folder in `mods/` with dependency order (once a second data mod exists); texture layering such as Minetest's grass side overlay (composited ahead of time for now); items, recipes and models in data files.
- *API debt:* none new, and none paid down. The base game's blocks now go through the same `Registrar` as before, from data instead of code.

#### WebAssembly movement seam (branch `seam-wasm-movement`)

**Stage:** Done (2026-10-26). Verified: the user played the wasm and native versions ("everything looks ok").

**Goal:** Run `base`'s movement controller as WebAssembly, loaded by the engine, to prove the plain-Rust API really crosses the boundary. The rest of `base` stays native.

*Why now:* the API debt list has three entries, and the movement controller is the richest mod behavior: it runs every tick and keeps state (timers, the charge).

*To settle in the plan:* the runtime (wasmtime or wasmi); the interface (the component model with WIT, or a minimal hand-rolled one); how `Box<dyn MovementController>` and `spawn()` become instance handles; `Tunables` across the boundary so the panel still works; whether float results match inside and outside. Read the design doc's modding sections first.

*Out of scope:* loading player-chosen mods from files, data mods, sandbox permissions, API versioning.

**Approved plan** (2026-10-26)

*Decisions*
- **Runtime: wasmtime**, chosen for the speed later block behavior will need and for component model support. NaN canonicalization and deterministic relaxed SIMD are on.
- **Interface: the component model with WIT**, in `crates/api/wit/movement.wit`. A `controller` resource has `spawn() -> controller`, `step(move-context) -> move-output`, `tunables() -> list<tunable>` and `set-tunable(name, value) -> bool`. `MoveContext`, `MoveOutput` (including `edge-guard`) and `Tunable` are WIT records.
- **Instance handles:** a host-side `WasmController` implements `MovementController` by holding a resource handle. Its `spawn()` calls the guest's `spawn`. Controllers from one module share a `Store` behind `Arc<Mutex<…>>`. `sim` still sees `dyn MovementController`.
- **Tunables** cross as a list plus `set-tunable`. `WasmController` implements `Tunables`, so the panel is unchanged.
- **Floats:** glam's `libm` feature is on workspace-wide, so `sin_cos` and the like match inside and outside wasm. Tests check bit-for-bit equality.
- **`PlayerDef` names its controller** (`controller: "base:quake"`) instead of holding a `Box<dyn MovementController>`, so it is plain data. The engine keeps a controller registry by name (`Registrar::register_movement_controller` for native ones, plus the wasm loader). `spawn_player` returns an error on an unknown name.
- **Traps:** a guest panic is logged once, the controller is marked dead and velocity passes through unchanged. The game doesn't crash.

*Crates*
- `crates/base_movement` (new): `QuakeController` moves here from `base`, depending only on `api`. Builds natively, so its unit tests stay. On `wasm32` it is exported as a component.
- `crates/api`: the WIT file, plus a wasm-only `export_movement_controller!` macro that wraps any `MovementController` impl with wit-bindgen exports. Modders write the plain trait.
- `crates/wasm_host` (new, engine-side, no Bevy): wasmtime setup, component loading, `WasmController`. `sim` doesn't depend on it; `game` wires it in.
- `crates/game`: a `build.rs` compiles `base_movement` for `wasm32-unknown-unknown` into a separate target directory, turns it into a component with the `wit-component` crate and embeds it with `include_bytes!`. `cargo run` needs no extra step. The component imports nothing, not even WASI. A `--native-movement` flag swaps the native controller in for comparison (a flag, not a control action).

*Tests*
- Equivalence: about 10,000 ticks of seeded scripted input (bunny hops, charging, crouching, ice, air strafing) through native and wasm controllers. Every `MoveOutput` field matches bit for bit.
- Whole simulation: the same run as a full simulation, near zero and at `FAR_ORIGIN`, with native and wasm positions matching bit for bit.
- `wasm_host`: spawned controllers keep separate state, tunables round-trip, a trap doesn't crash, an unknown controller name is an error.

*Done when*
1. The game runs `base`'s movement from a wasm component while the rest of `base` stays native.
2. The user plays it and it feels the same as native (bunny hops, charged jumps, edge guard, ice).
3. The equivalence tests pass bit for bit.
4. The tuning panel adjusts and resets the wasm controller.
5. `cargo test` and `cargo clippy` are clean, and `cargo run` needs no extra build step.

*Deferred:* fuel or epoch limits (an infinite loop in a mod still hangs the game; part of sandbox permissions), loading mods from files, versioning.

*Setup:* the user approved installing the `wasm32-unknown-unknown` target with rustup.

*Implementation notes* (2026-10-26)
- **Floats: the `libm` crate, not glam's `libm` feature.** The trig that mattered was std's `f32::sin_cos` in `MoveInput::wish_direction` (and `look_direction`), not glam's, so `api` calls `libm::sincosf` directly. Checked: with std's `sin_cos` the native and wasm results differ in the last bit within 300 ticks; with `libm` they match bit for bit over 10,000. Bevy's glam is untouched.
- **Where the tests live.** The components are built by `game/build.rs`, so the boundary tests are in `crates/game/tests/wasm_movement.rs` rather than in `wasm_host`. A small fixture crate, `crates/test_mods/panicky_movement`, panics on its first step, for the trap test. It is built into `OUT_DIR` for tests only, not embedded in the game.
- **Native registration.** `base_movement::BaseMovementMod` registers the native controller (`--native-movement` and tests). `Registrar::register_movement_controller` takes a `Box<dyn MovementController>`, so it is native-only; a WebAssembly mod exports its controller with `export_movement_controller!` instead.
- **Trapped controllers** keep velocity unchanged and drop gravity too, so the player floats. That's fine for a broken mod, and the message is printed once.
- **Build cost.** Adding wasmtime changed shared dependency features, so Bevy rebuilt once (about 10 minutes). Later builds are normal.

*Result*
- All five "Done when" criteria met. Six new boundary tests; 137 tests pass in the workspace. Clippy is clean natively and for `wasm32`.
- *API changes:* `PlayerDef.controller` is now a name; there's a new `Registrar::register_movement_controller` and `ApiError::UnknownController`. `api` has the WIT file and the wasm-only `export_movement_controller!` macro.
- *API debt:* none new, and none paid down. This seam proved the boundary works rather than clearing debt; the three entries remain.

#### Edge guard (branch `toy-edge-guard`)

**Stage:** Done (2026-10-25). Verified: the user played it ("it works").

**Goal:** Crouching keeps you from walking off edges, like Minecraft's sneaking. Picked up from Advanced movement's deferred list at the user's request (2026-10-25), while the WebAssembly seam was at the Plan stage on its own branch. The user approved the detour.

*What was built*
- `api`: `MoveOutput` gains `edge_guard: Option<f32>`, the biggest drop a body on the ground may walk off. `None` means no guard, so other controllers don't change.
- `sim`: while it's set, the body is on the ground and it isn't jumping, horizontal movement is cut, one axis at a time, if it would leave nothing solid under the footprint within that drop. The body slides along edges instead of stopping dead. It may hang over the edge as long as part of the footprint is supported.
- `base`: the Quake controller turns it on while crouched on the ground and not jumping. `edge_guard_drop` is tunable and defaults to 0.6 (Minecraft's value), which stops you at every boxel edge, steps included. Set it above 1 to crouch-walk down single steps.
- Tests: stops at a cliff, slides along an edge, the drop limit, walking off without crouch, jumping off while crouched, and the controller only guarding while crouched on the ground. 131 tests pass; clippy is clean.

*API debt:* none new. The WebAssembly seam has to carry `edge_guard` across the boundary along with the rest of `MoveOutput`.

#### Advanced movement (branch `toy-advanced-movement`)

**Stage:** Done (2026-10-25)

**Goal:** Crouching, the Half-Life tuck, a charged crouch jump and block-jumping, all feeling robotic.

*Backlog entry:* Split out of the movement toy at the user's request (2026-10-25). Crouching and crouch jumping, both kinds:
- *Half-Life tuck:* crouching in the air lifts the feet, so jump-then-crouch clears a higher ledge.
- *Charged jump:* holding crouch on the ground builds a spring charge, so jumping from a crouch goes higher. Fits the robot feel.
- Sketch: `MoveInput` gains `crouch`; the controller returns the body height it wants. Engine rule: shrinking is always allowed, growing needs headroom; on the ground the feet stay put, in the air the head stays put (that's what makes the tuck work). Crouch-walk speed, charge time and charged height are tunables in `base`.
- *Block-jumping* (moved here from World interaction controls, 2026-10-25; needs block placing, which the Block editing toy adds). Placing a block into your own feet launches you. Build it as a general engine rule, not a special case: a solid appearing inside a body pushes it out with an impulse along block center → body, with tunable strength and upward bias. Standing on the far edge of the cell tilts the push forward, so looking down and timing placements becomes fast-movement tech. The same rule later lets pistons shove players.

**Approved plan** (2026-10-25)

*API (`api`)*
- `MoveInput` gains `crouch` (held). `MoveContext` gains the body's current `height`. `MoveOutput` gains `height: Option<f32>`, the height the controller wants; `None` means standing height, so other controllers don't change.
- `PushOutDef { strength, upward_bias }`, registered by `base` like `InteractionDef` and tunable live.

*Engine (`sim`)*
- Height changes: shrinking is always allowed, growing is clamped to the headroom. On the ground the feet stay put; in the air the head stays put. The eye stays the same distance below the top of the head.
- Push-out rule: a solid appearing inside a body moves the body out the shortest way that has room, then gives it an impulse along block center → body center plus the upward bias, so the far edge of the cell tilts the launch forward.
- Placing into your own body is allowed when there's room to push you out, refused otherwise.

*Base (`base`)*
- Quake controller: crouch height (1.0), crouch-walk speed (40% of walking), charge time (0.6 s) and charged jump height (2.5), all tunable.
- Charge builds while crouched on the ground and is lost on standing up without jumping. Holding crouch through the jump keeps you tucked, so the two stack.

*Client*
- Crouch key, default left Shift (shared with fly-down; flying ignores crouch), rebindable and added to the user's controls file.
- Camera feel: eye height eases between standing and crouching like a hydraulic, and sinks a little further as the charge builds.

*Tests:* crouch shrinks from the top on the ground; can't stand under a low ceiling; the tuck lifts the feet, keeps the head, and clears a ledge a plain jump can't; the charge raises the jump up to the charged height and resets on standing; push-out clears the body with the right impulse direction, including the edge tilt; placing is refused with no room to push out; the "millions" test still matches.

*Out of scope:* edge guard (crouching doesn't stop you walking off edges), slide, wall-jump.

*API debt:* crouch joins the fixed input actions entry.

*Done when*
- Crouching, the tuck and the charged jump work and feel robotic to the user.
- A crouch-jump clears a ledge a plain jump can't.
- Placing a block under your feet launches you, and the far edge of the cell tilts the launch.
- Tests cover height changes, the tuck, the charge and push-out.

*Plan change* (2026-10-25, the user's request during Verify): a **chase drill** added as a temporary tool to feel the movement, not a feature. A ghost floats straight at you through walls (5.5 m/s, +0.25 per point) and resets your score when it touches you; objectives appear at random landmarks (one is the bottom of the pit). `base` supplies the landmarks as waypoints through the existing API; the drill is one client module, `client::drill`, toggled with F4, meant to be deleted when later toys no longer need it. Logged as API debt so it doesn't become permanent.

*Implementation notes*
- `MoveContext` also gains `standing_height`, so a controller can tell it's crouched (or stuck low under a ceiling) without knowing the body's definition.
- The charge sinks the camera through the body itself: the controller asks for `crouch_height − charge × charge_sink` (0.15 m at full charge). The client needed no charge data. Side effect: a charged body is 0.15 m shorter.
- Charged jump and tuck stack: on the jump tick the controller asks for standing height (the spring extends); with crouch still held it tucks on the next tick, mid-air, so the feet lift.
- Height changes move the previous position too, so they aren't interpolated as motion. `Player::crouch_shift_total` counts how far the eyes jumped (down for a ground crouch, zero for a mid-air tuck), and `client::feel` eases it with a new `crouch_recover` tunable, like step-up smoothing.
- Push-out moves the body along the launch's strongest axis if there's room, otherwise the shortest way that has room. Without a `PushOutDef`, or with no room, placing into the body is refused as before. Push-out isn't eased on the camera.
- `base` defaults: push-out strength 6 m/s, upward bias 1.
- Two ledges added to the movement course, west-facing at x 26 to 29: 2 high (y −30 to −27) for a tuck, 3 high (y −36 to −33) for a charged tuck. Neither clears with a plain jump.
- The tuck's margin on the 2-high ledge is thin. The feet peak at 2.12, so you have to reach the edge near the top of the jump. Worth tuning (`crouch_height`) while playing.
- Chase drill: `base` adds 11 landmarks (`base::LANDMARKS`) to both copies of the test area, so the teleport key now cycles through them too. The tower top was left out, since it's 41 high. A beam marks the objective so it can be found from anywhere. Drill values are in the tuning panel. Within the plan change: the ghost follows you if you teleport, and new objectives prefer spots at least 15 m away.
- Added during Verify, outside the plan (comfort, at the user's request): double-tap forward to sprint, Minecraft-style. Sprints until forward is released; the sprint key still works. Client input only (`controls::DoubleTapSprint`), with the window set by `double_tap_window` in the controls file (default 0.3 s, 0 turns it off).
- 125 tests pass across the workspace; clippy is clean.

*Verification* (2026-10-25)
- Crouching and the tuck work and feel robotic: met (played with the chase drill; the user confirmed they feel right).
- The charged jump: not met. In play it gave no extra height. A headless check showed it works only if crouch is still held when jump is pressed (2.59 vs 1.32); releasing crouch to jump, even on the same tick, threw the charge away. Fixed with a `charge_grace` tunable (0.2 s, to cover a thumb sliding from crouch to jump): the charge survives letting go of crouch for that long, including on the jump tick itself. After the fix: met (played; "feels good, it works now").
- A crouch-jump clears a ledge a plain jump can't: met (tests on the 2-high and 3-high ledges; confirmed in play).
- Placing a block under your feet launches you, and the far edge tilts the launch: met (tests; confirmed in play).
- Tests cover height changes, the tuck, the charge and push-out: met.

*Decisions*
- Crouching is the robot's spring: a ground crouch charges it, a jump releases it, and a mid-air crouch tucks the feet. Recorded in the design doc under the robotic player.
- Body height is the controller's call, and the engine enforces the room: shrinking is free, growing needs headroom, feet stay put on the ground and the head stays put in the air.
- A block appearing inside a body pushes it out and launches it, as an engine rule with mod-supplied numbers (`PushOutDef`). Block-jumping is the first use; pistons can use it later.
- The chase drill is a temporary tool for feeling movement, not a feature. Delete it once later toys don't need it.

*Deferred*
- The tuck's margin on the 2-high ledge is thin (feet peak at 2.12). Revisit `crouch_height` during detailed tuning.
- Edge guard, slide, wall-jump.
- Push-out isn't eased on the camera.

*API debt added:* crouch joins the fixed input actions entry; the chase drill in the client (temporary).

#### Block editing (branch `toy-block-editing`)

**Stage:** Done (2026-10-25)

**Goal:** Place and break blocks, Minecraft-style.

*Rescoped 2026-10-25:* this toy started as "World interaction controls", aimed at Minecraft's ambiguous right click. There's nothing to use or open yet, so there's nothing ambiguous to fix. It's now just placing and breaking, copying Minecraft's controls. The ambiguity goal moved to the backlog as "Interaction controls".

**Approved plan** (2026-10-25)

*Controls:* left click breaks, right click places, number keys or the scroll wheel pick the block. All rebindable, and added to the user's controls file (see `CLAUDE.local.md`). Holding a button repeats at about 4 per second. Breaking is instant.

*Feedback:* a Minecraft-style outline on the targeted block. No ghost preview.

*Engine (`sim`)*
- Boxel raycast (DDA) returning the hit cell and face. Gameplay, so it lives in the simulation.
- Edits are intents checked at the tick: in reach, not into a filled cell, not overlapping the player's body. Applied at the tick boundary, the shape a server needs later.
- Changed chunks are tracked for remeshing. `PlayerInput` gains pitch for the raycast.

*API and base*
- `base` registers reach and hold-to-repeat rate, exposed through `Tunables`.
- `BlockDef` gains `placeable`, so the palette knows what to offer.

*Client*
- Block outline from the raycast, and a simple HUD showing the selected block.
- Remesh only changed chunks, plus neighbors for edits on a chunk border.

*Tests:* raycast (hits, faces, reach, negative and far coordinates), edit validation (not inside the player, not into a filled cell, not out of reach), chunk change tracking including border neighbors, remeshing after an edit, and a break-then-place integration test.

*Out of scope:* ghost preview, timed breaking, inventory or items, tools, drops, particles, sound, block-jumping (Advanced movement).

*API debt:* none new expected; break, place and block selection join the fixed input actions entry.

*Done when*
- You can break and place blocks with the mouse, and pick which block to place.
- The outline always marks the block that will break, and placing lands on the face you're looking at.
- Placing into your own body, into a filled cell or out of reach does nothing.
- Edits on chunk borders remesh correctly with no holes or stale faces.
- Raycast, edit validation and change tracking have tests.

*Implementation notes*
- `api` gains `InteractionDef { reach, repeat_rate }`, registered through `Registrar::register_interaction` like `PlayerDef`, and it implements `Tunables` itself, so the panel shows an Interaction section. Without one, the engine does no editing. `base` uses reach 4.5 and 4 per second.
- `look_direction` moved from `client::coords` to `api`, so the simulation and the camera share one definition.
- The simulation raycasts at the start of each tick, from where the player stood at the end of the last tick, using the tick's yaw and pitch. That's the same spot and look the client drew the outline from, so the outline matches what breaks. Edits are applied before movement.
- Hold-to-repeat only continues a press the game saw. The click that captures the mouse doesn't break anything, even if you keep holding it. If both buttons act on the same tick, breaking wins.
- Break and place accept mouse buttons or keys (`MouseLeft`, `MouseRight`, `MouseMiddle`, `MouseBack`, `MouseForward`). The palette slots are actions `slot_1` to `slot_9`. The scroll wheel isn't rebindable.
- Small additions within the plan: a crosshair, and the palette along the bottom of the screen with the selected block in brackets.
- 99 tests pass across the workspace; clippy is clean.

*Verification* (2026-10-25)
- Break, place and pick a block: met (played; "works as expected").
- The outline marks the block that will break, and placing lands on the face you look at: met (played).
- Placing into your own body, a filled cell or out of reach does nothing: met (played the body check; tests cover all three).
- Chunk borders remesh with no holes or stale faces: met (played on the borders; tested).
- Tests for raycast, edit validation and change tracking: met (99 tests pass across the workspace; clippy is clean).

*Decisions*
- Copy Minecraft's controls for now. Fixing ambiguous interactions waits until there are things to use; that's the Interaction controls backlog toy.
- Interaction numbers are content: `base` registers an `InteractionDef`. The engine owns targeting and the rules that keep edits valid.

*Deferred*
- Tuning reach and repeat rate, until later toys give more context. The user found the toy thin on its own but working.
- Ghost preview, timed breaking, inventory, drops, particles and sound.

*API debt added:* break, place and the palette slots join the fixed input actions entry.

#### Movement mechanics (branch `toy-movement`)

**Stage:** Done (2026-10-25)

**Goal:** Quake-style movement that feels good, with the base mod (not the engine) deciding how movement works.

**Approved plan** (2026-10-25)

*Split:* how movement feels is content, so `base` implements the controller. The engine owns what every controller needs and what must stay consistent: the tick, body state, collision, impulses and surface data.

*API (`api`)*
- `MovementController` trait: each tick it gets plain data (body state, input, ground contact and the surface underfoot, `dt`) and returns the new velocity. Gravity is its job.
- `PlayerDef` shrinks to body shape (width, height, eye height), step-up height and the controller.
- `Tunables` trait: `list() -> Vec<Tunable { name, value, min, max }>` plus `set(name, value)`, so the engine's panel can tune any mod's values.
- `BlockDef` gains `friction`.

*Engine (`sim`)*
- Runs the controller, then applies impulses, then collides. Jump goes through the impulse path.
- Optional step-up in collision (off by default).
- Reports ground contact and the friction underfoot to the controller.
- `PlayerInput` gains `sprint` and separate jump-pressed (latched) and jump-held.
- Fly mode stays an engine debug tool that bypasses the controller.

*Base (`base`)*
- Quake controller: ground acceleration and friction (scaled by block friction), air acceleration with strafing, momentum kept, bunny hopping, sprint, jump buffering, coyote time, stronger fall gravity and terminal velocity. Every value exposed through `Tunables`.
- An ice block, and a movement course: jump gaps of 2 to 5, a pillar line, a sprint straight, a drop tower and an ice patch.

*Client (`client`)*
- `bevy_egui` 0.39.1 tuning panel on a key: sliders for every tunable, reset, print values.
- Camera feel (FOV widening with speed, head bob, landing dip), also tunable.
- New rebindable keys for sprint and the panel.

*Tests:* the controller (acceleration time, friction stop, air strafing gains speed, bunny hops keep speed), the coyote and buffer windows at their edges, ice slides farther, step-up limits and headroom, impulses work with any controller, and the bit-identical "millions" test still passes.

*API debt to log:* fixed engine input actions; camera feel in `client`.

*Implementation notes*
- The Quake controller's jump tick skips friction but still applies ground acceleration. Pure Quake (air acceleration only on the jump tick) left you crawling at the air speed cap when holding jump from a standstill. Bunny hop speed is unaffected, since acceleration never removes speed above the wish speed.
- `Player` exposes cumulative landing and step-up counters, so the renderer can react to events on ticks it didn't draw.
- New default keys: sprint is left Ctrl, the tuning panel is Tab.
- After the first play session (2026-10-25): the user is happy with the movement. Detailed tuning is deferred until later toys give more context, so the "tuned values saved back into `base`" criterion is deferred rather than met. The current defaults stay.
- Tuned by playing: `ground_accel` and `air_accel` 10 → 15, because held-jump stair climbs stalled against risers at 10 (a regression test now checks a held-jump climb reaches the top of the steps). `hard_landing_speed` 15 → 20 m/s, so jumping off a 3-boxel ledge stays a soft landing.
- Landing reworked as a robotic shock absorber (camera feel, within the plan): a damped suspension that compresses smoothly on soft landings and bottoms out instantly, with a nod and a slow recovery, at or above `hard_landing_speed`. That speed is a placeholder for "this fall hurts" until fall damage exists, and should then come from the same source.

*Out of scope:* crouch, slide, wall-jump. Push-out and block-jump go to the World interaction controls entry.

*Done when*
- Movement feels good to the user after a tuning session.
- Bunny hopping and air strafing work.
- Ice feels different.
- The tuned values are saved back into `base`.

*Verification* (2026-10-25)
- Movement feels good to the user: met. The user played it and was happy with the movement, the stair climbing after the acceleration change, and the landing suspension.
- Bunny hopping and air strafing work: met by tests (bunny hops keep speed through the landing tick, air strafing gains speed past walk speed). Not specifically checked in play.
- Ice feels different: met by test (sliding on ice goes over 5× as far as on grass). Not specifically checked in play.
- Tuned values saved back into `base`: partly met. The acceleration and hard-landing values found by playing are the new defaults; detailed tuning is deferred until later toys give more context.
- 75 tests pass across the workspace; clippy is clean. The slice 1 note about 30 fps feeling worse didn't come up again.

*Decisions*
- Movement is content. `base` owns the controller; the engine owns the body, collision, impulses and surface data. Recorded in the design doc.
- The API stays WebAssembly-friendly (plain data in and out), now a phase 2 rule.
- Live tuning is generic: anything implementing `Tunables` gets sliders, whichever mod it comes from.
- The player should feel robotic. Landings are a shock absorber whose hard bottom-out is meant to signal fall damage. Recorded in the design doc as a tentative direction.

*Deferred*
- Detailed tuning of the Quake values.
- Crouch and crouch jumping, now the Advanced movement backlog toy. Block-jump and push-out are in the World interaction controls entry.
- `hard_landing_speed` is a camera-only placeholder. When fall damage exists, it should come from the same source as the damage threshold, so the feel and the damage always agree.
- Step-up exists in the engine but `base` has it off; it hasn't been played.

*API debt added:* fixed input actions; camera feel in the client (see the API debt list).


---

## Phase 3: Toward launch (not yet sliced)

Once the toys have found what makes the game fun and the API has grown with them:

- **The mastery loop.** A finite stressor that comes to the player, with at least two ways to solve it, built around the signature feature.
- **API hardening.** Clear remaining API debt, make WebAssembly mods deterministic, and add versioning so the API can be called stable.
- **More early modders,** following the design doc's path to modder critical mass.

After that: server/client split and multiplayer, lighting, sub-grids, world views, field layers, signal networks and the console.

Single-player is meant to run a local server, so every slice keeps simulation and rendering separate, even before networking exists.
