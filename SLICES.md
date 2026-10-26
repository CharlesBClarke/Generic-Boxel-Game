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

**WebAssembly movement seam** (branch `seam-wasm-movement`)

**Stage:** Plan (not started; agreed as the next step on 2026-10-25)

**Goal:** Run `base`'s movement controller as WebAssembly, loaded by the engine, to prove the plain-Rust API really crosses the boundary. The rest of `base` stays native.

*Why now:* the API debt list has three entries, and the movement controller is the richest mod behavior: it runs every tick and keeps state (timers, the charge).

*To settle in the plan:* the runtime (wasmtime or wasmi); the interface (the component model with WIT, or a minimal hand-rolled one); how `Box<dyn MovementController>` and `spawn()` become instance handles; `Tunables` across the boundary so the panel still works; whether float results match inside and outside. Read the design doc's modding sections first.

*Out of scope:* loading player-chosen mods from files, data mods, sandbox permissions, API versioning.

### Toy backlog

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

- **Fixed input actions** (movement toy; break, place and palette slots added by the block editing toy, crouch by the advanced movement toy). Move, jump, sprint, crouch, break, place, the palette slots and the debug keys are a fixed engine set in `sim::PlayerInput` and `client::controls`. Mods can't define their own actions. API version: mods register named actions with default bindings, and controllers read actions by name.
- **Chase drill in the client** (advanced movement toy, temporary). The ghost and objectives in `client::drill` are game logic living in the client, and it finds landmarks by the `landmark:` prefix on waypoint names. Remove the drill rather than moving it into the API, unless a later toy wants real enemies, which would need entities and a game-rules hook in the API.
- **Camera feel in the client** (movement toy). FOV widening, head bob, landing dip and step smoothing live in `client::feel`, so mods can't change them. API version: a client-side mod API for camera effects, presumably driven by the same tunables.

### Phase 2 decisions

- **A robotic player** (tentative, 2026-10-25): the user wants movement and camera feel to read as a robot. Landings are a shock absorber: soft landings compress smoothly, damaging ones bottom out hard. Later feel work (combat, damage, crouch charging) should keep to it.
- **Signature feature:** not decided yet. Choose it once the block interaction and crafting toys have been played, and record the choice and why in the design doc.

### Done toys

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
