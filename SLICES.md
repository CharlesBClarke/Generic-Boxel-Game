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

#### Block shapes and shared cells (branch `toy-block-shapes`)

**Stage:** Verify (2026-10-28). Implemented and tested; waiting for the user to play it.

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
- **Placement.** `Simulation::placement(hit, block, yaw)` returns a `PlaceTarget` (boxel, turned occupant, allowed), so a refused placement still says where it would go.
- **Content.** Stone and brick slabs and stairs, a wood fence post and a log, with Minetest Game's `default_wood` (BlockMen), `default_tree` and `default_tree_top` (Cisoun) credited in `mods/base/CREDITS.md`. The shapes corner west of spawn has a staircase up to a platform, a brick slab path with posts through two slabs, a lone post, a double slab, and logs lying and standing. The debug teleport has a new `shapes` waypoint at its foot.
- **Client.** The mesher draws each occupant's turned boxes with textures cropped to each box, and hides a face only when it's on the boxel's side against a full block. The outline traces the aimed occupant's boxes. The ghost is a see-through mesh of the placed occupant, slightly shrunk so it doesn't flicker against neighbors, red when refused.
- *API changes:* `Orientation`, `BlockBox`, `Placement`, `BlockDef.shape`, `placement`, `with_shape`, `with_placement`, `is_full`, `validate`, `WorldEdit::add_block`, `ApiError::CannotShare`.
- 196 tests pass in the workspace; clippy is clean.

*Changes from the plan, found while building* (each small; flagged for the user)
- **Step height 0.6 in `base`.** The plan said stairs would use "the existing step height", but the base player's was 0, so stairs couldn't be walked up. 0.6 is Minecraft's: enough for slabs and stairs, not for full blocks.
- **Fallback to the next boxel.** When the block can't share the boxel in front of the aimed point (a full block aimed at the top of a slab), it goes into the boxel on the far side of the face instead, as in Minecraft. Without this you couldn't build on a slab.
- **No separate list of allowed orientations.** A block's placement rule decides which orientations it can have.
- **No API type for reading a boxel's occupants.** Nothing in a mod needs to read the world yet; mods add turned blocks with `WorldEdit::add_block`.

*Noticed while testing:* standing on a slab, a fence post is only 0.5 above your feet, so the 0.6 step height walks you onto the post. Minecraft makes fence collision 1.5 high for this; worth deciding when fences join their neighbors.

### Toy backlog

- ~~**Block shapes and placement.**~~ Now the current toy, as Block shapes and shared cells. Suggested order after it: inventory and items, then interaction controls and block interactions, then combat feel.
- **Interaction controls.** Split out of Block editing (2026-10-25). Fix Minecraft's ambiguous right click once there are things to use, like crystals or devices. Principles: one input, one action (no catch-all "use" button that changes meaning with what you hold or look at), and what you see is what you get (a ghost of the result, red when not allowed).
- **Block interactions.** Unstable crystals: two block types, a touch trigger, a directional blast, and a way to push blocks so devices are possible.
- **Generative crafting.** A few materials, a few property ingredients and a capacity limit.
- **Combat feel.** Weighty hits and aggressive, Doom-style combat.
- **Liquid rule sets.** The rule sets from the design doc.
- **Advanced movement revisited.** Added 2026-10-26 at the user's request. Comes after toys that add tools, items and enemies. Rework push-out (block-jumping, and later pistons). The user doesn't like that the body is moved out of the block instantly, like a teleport, or that the launch is mostly upward. Prompted by spam-placing blocks at the dead end of a 1-high tunnel while crouched: the body crawls backward through position shifts while the speed readout stays at 0. Things to settle:
  - How the body gets out of the block with real motion instead of a shift, and how much of the launch should go up.
  - What placing a block while crouched should do. The crouched body's center is at or below the block's center, so today it gives a sideways shove off the side of the block that was never designed.
  - Whether a jump should keep wiping out the push-out kick. Today, placing a moment before jumping loses the kick, which makes block-jumping feel inconsistent.
  - **Known flaw, deferred to this toy (2026-10-26):** the kick can point opposite to the shift. The kick's direction comes from the body's position before the shift, so in the tunnel case the body is shifted backward but kicked forward, into the block it just left, and the collision zeroes it. That's why the speed readout stays at 0 (the readout itself is correct). Found by reading the code and confirmed by the user's play; not yet covered by a test. The redesign must make the launch go the way the body actually leaves the block.

Ideas that come up while playing get added here.

### Infrastructure (added when a toy needs it)

- Saving and loading, with block names in the save
- World generation
- LOD, once world generation exists

### Seam backlog (added when the game is ready for them)

- ~~**Data mods.**~~ Done (2026-10-27), see Done toys.
- **WebAssembly behavior.** Sandboxed mod code for block behavior, once a toy has behavior worth sandboxing, such as crystals or liquids. Sleeping blocks and read-old, write-new automata. The first step is done: the WebAssembly movement seam (2026-10-26) runs `base`'s movement controller as a component, so block behavior can follow the same pattern (WIT interface in `api`, guest export macro, `wasm_host` wrapper). *To settle (2026-10-27):* keep entity and block state in the engine, not in the mod, before any behavior keeps state in a mod (see the design doc's "Mod memory"). The movement controller still keeps its timers and charge inside the mod and should move to that pattern. Test whether wasmtime lets the engine drop a mod's freed pages (`madvise` on Linux, `DiscardVirtualMemory` on Windows), driven by an allocator in `api`.
- ~~**Sandbox limits.**~~ Done (2026-10-28), see Done toys.
- **Seed a modder.** Once a few seam slices are done, have an outside modder build a small toy on the API and fix what blocks them.

### API debt

Shortcuts taken around the API, each with the toy that needed it. Paid down in seam slices.

- **Fixed input actions** (movement toy; break, place and palette slots added by the block editing toy, crouch by the advanced movement toy). Move, jump, sprint, crouch, break, place, the palette slots and the debug keys are a fixed engine set in `sim::PlayerInput` and `client::controls`. Mods can't define their own actions. API version: mods register named actions with default bindings, and controllers read actions by name.
- **Chase drill in the client** (advanced movement toy, temporary). The ghost and objectives in `client::drill` are game logic living in the client, and it finds landmarks by the `landmark:` prefix on waypoint names. Remove the drill rather than moving it into the API, unless a later toy wants real enemies, which would need entities and a game-rules hook in the API.
- **Camera feel in the client** (movement toy). FOV widening, head bob, landing dip and step smoothing live in `client::feel`, so mods can't change them. API version: a client-side mod API for camera effects, presumably driven by the same tunables.

### Phase 2 decisions

- **A robotic player** (tentative, 2026-10-25): the user wants movement and camera feel to read as a robot. Landings are a shock absorber: soft landings compress smoothly, damaging ones bottom out hard. Later feel work (combat, damage, crouch charging) should keep to it.
- **Signature feature:** not decided yet. Choose it once the block interaction and crafting toys have been played, and record the choice and why in the design doc.

### Fixes

Bugs found in already-merged code, each fixed on its own `fix-` branch (see the workflow in `CLAUDE.md`). One line each: date, branch, what was wrong.

- 2026-10-28, `fix-step-smoothing`: a step-up moved the body but not its previous position, so the renderer blended the rise between ticks while the camera feel also eased it, making the view dip and snap when climbing. Latent until the block shapes toy gave the player a step height.

### Done toys

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
