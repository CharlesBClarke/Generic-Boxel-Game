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

None. Pick the next toy from the backlog.

### Toy backlog

- **Advanced movement.** Split out of the movement toy at the user's request (2026-10-25). Crouching and crouch jumping, both kinds:
  - *Half-Life tuck:* crouching in the air lifts the feet, so jump-then-crouch clears a higher ledge.
  - *Charged jump:* holding crouch on the ground builds a spring charge, so jumping from a crouch goes higher. Fits the robot feel.
  - Sketch: `MoveInput` gains `crouch`; the controller returns the body height it wants. Engine rule: shrinking is always allowed, growing needs headroom; on the ground the feet stay put, in the air the head stays put (that's what makes the tuck work). Crouch-walk speed, charge time and charged height are tunables in `base`.
  - *Block-jumping* (moved here from World interaction controls, 2026-10-25; needs block placing, which the Block editing toy adds). Placing a block into your own feet launches you. Build it as a general engine rule, not a special case: a solid appearing inside a body pushes it out with an impulse along block center → body, with tunable strength and upward bias. Standing on the far edge of the cell tilts the push forward, so looking down and timing placements becomes fast-movement tech. The same rule later lets pistons shove players.
- **Interaction controls.** Split out of Block editing (2026-10-25). Fix Minecraft's ambiguous right click once there are things to use, like crystals or devices. Principles: one input, one action (no catch-all "use" button that changes meaning with what you hold or look at), and what you see is what you get (a ghost of the result, red when not allowed).
- **Block interactions.** Unstable crystals: two block types, a touch trigger, a directional blast, and a way to push blocks so devices are possible.
- **Generative crafting.** A few materials, a few property ingredients and a capacity limit.
- **Combat feel.** Weighty hits and aggressive, Doom-style combat.
- **Liquid rule sets.** The rule sets from the design doc.

Ideas that come up while playing get added here.

### Infrastructure (added when a toy needs it)

- Saving and loading, with block names in the save
- World generation
- LOD, once world generation exists

### Seam backlog (added when the game is ready for them)

- **Data mods.** Blocks and textures defined in mod files, loaded at startup. Likely after the first couple of toys. Textures from Minetest Game, with credits and license kept.
- **WebAssembly behavior.** Sandboxed mod code for block behavior, once a toy has behavior worth sandboxing, such as crystals or liquids. Sleeping blocks and read-old, write-new automata. The user wants a real WebAssembly test soon, to check the plain-Rust API really crosses the boundary: a candidate first step is running `base`'s movement controller as WebAssembly.
- **Seed a modder.** Once a few seam slices are done, have an outside modder build a small toy on the API and fix what blocks them.

### API debt

Shortcuts taken around the API, each with the toy that needed it. Paid down in seam slices.

- **Fixed input actions** (movement toy; break, place and palette slots added by the block editing toy). Move, jump, sprint, break, place, the palette slots and the debug keys are a fixed engine set in `sim::PlayerInput` and `client::controls`. Mods can't define their own actions. API version: mods register named actions with default bindings, and controllers read actions by name.
- **Camera feel in the client** (movement toy). FOV widening, head bob, landing dip and step smoothing live in `client::feel`, so mods can't change them. API version: a client-side mod API for camera effects, presumably driven by the same tunables.

### Phase 2 decisions

- **A robotic player** (tentative, 2026-10-25): the user wants movement and camera feel to read as a robot. Landings are a shock absorber: soft landings compress smoothly, damaging ones bottom out hard. Later feel work (combat, damage, crouch charging) should keep to it.
- **Signature feature:** not decided yet. Choose it once the block interaction and crafting toys have been played, and record the choice and why in the design doc.

### Done toys

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
