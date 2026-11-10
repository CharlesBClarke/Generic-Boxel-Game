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

**Waiting for a play check** (remind the user at the start of every session, and at good moments during one, until they confirm; then delete the line):

**Low-priority play checks** (don't remind every session; check only when a toy makes it convenient, such as one that puts the player among moving cows; delete the line once seen):
- Cow herd (1): a cow shoved well away from the herd walks back to it on its own.
- Cow herd (3): a moving cow that runs into the player pushes them back, with no jitter or sticking.
- Sound (5): sounds pan as you turn and fade with distance. Hard to judge with cows mooing rarely; check once something makes a steady or on-demand sound.

- Cow pathfinding (low priority, 2026-11-06): the user called the toy "really good" without going through the five course checks one by one; note any that misbehave: cows through the pen's gap and up its terraces, up the mesa's steps, a charge baited off the mesa's cliff (landing hurt), waiting below the pillar (orange route), climbing hills on the generated terrain.

### Experimental materials (`toy-experimental-materials`)

**Stage:** Verify (batch A). `fix-mesher-cost` is done and merged into this branch (2026-11-08). **Next session:** batch B (sensor, delay, trigger scope).

**Goal:** an experiment kit of small materials, each with a one-sentence rule, to find which set balances trivial use for players who don't care against combining for those who do, and to compare boxel-wide against touching signals. The user (2026-11-08): "the goal here is to disover what set of materials has a good ballence of trival use for Luers, and combantainal complexity for the people who care. we allso need moore systems if we want to figure out if we want triggers to be block global or texel adjasenct, so I just want a verity of little toys to really experiment what works". Names are placeholders ("bs names now").

**Plan** (approved and engaged 2026-11-08). Built in three batches, with a play check after each:
- **A. Light and crystals.**
  - *Glow:* a look field `glow = { strength }` beside `sheen`; a glowing material is drawn self-lit (no sun, shade or AO) and lights nothing else (that's Stored light). No on/off of its own: hide it with phasite or cover it with a phasite shutter. *Lumite*, a plain lamp. Optional bloom behind a slider.
  - *Crystals:* properties `fuse` (a blue crystal) and `lens` (a green crystal). When a fuse piece and a lens piece touch face to face at texel level, both are used up and a blast fires from blue toward green, along the face they touch on; touching on faces in several directions fires one beam per direction. Checked on boxels that just changed (placed, shown by a phase flip, broken next to). A beam runs up to `range` boxels, breaks blocks in its path (they drop as usual) until its `strength` runs out, hurts and knocks back cows and the player (a rocket jump). A beam reaching a lens sets it off, firing onward in the beam's direction at full strength; one reaching a fuse just breaks it. Client: beam flash, a pickable sound, shake. All on sliders (`BlastDef`). Both crystals glow a little.
- **B. Signals.**
  - *Sensor:* a body (player or cow) touching a piece of it signals it, once per touch.
  - *Delay:* passes a signal it gets on after `delay` seconds (slider). Signals still waiting are dropped on save.
  - *Trigger scope switch* (a tuning toggle): boxel-wide (today: a signal reaches a boxel and every piece in it answers; copper carries boxel to boxel) or touching (a signal reaches only the pieces touching the piece that sends it; copper carries only through copper pieces that touch). Crystals always use texel touching.
- **C. Feel.**
  - *Bounce:* landing on it springs you up (jump pads); the movement API sees it like friction.
  - *Falls:* unsupported pieces drop, like sand. (Climb was left out.)
- **Throughout:** the crosshair hint lists the aimed material's properties; a demo spot in the test area per batch; every new material is endless.

**Done when**
1. A lumite lamp glows in the dark-shaded side of a build, and a phasite shutter toggles it.
2. Placing a blue crystal against a green one fires a beam from blue to green that breaks blocks and knocks back cows and the player; a lever wired to a hidden blue crystal fires it; a line of green crystals chains.
3. A sensor fires a trap when a cow or the player steps on it; a loop of delays makes a clock.
4. The trigger scope switch changes what the knob example does (a knob not touching a phasite strip in its boxel flips it boxel-wide, not touching).
5. A bounce pad springs the player; a falls material drops when what holds it up is broken or hidden.
6. Tests for each rule; `cargo test` and clippy clean.

**Batch A notes** (2026-11-08)
- Glow: `LookDef::glow`, packed beside the sheen in the shader's per-look texel; drawn as `lit × (1 − min(glow, 1)) + own colors × glow`, ignoring AO. Bloom left out for now: it needs Bevy's HDR path, which changes the tone mapping of everything; offer after the user has seen the glow.
- Crystals: `sim::blast` (`reactions`, `beam`), run once a tick after edits and signals on the boxels changed (`ChangedChunks` now keeps them). Blast values live in `SignalDef.blast` (`BlastDef`), tuned in the `signals` section. Used-up crystals drop nothing; what a beam breaks drops as usual. At most 256 beams a tick. Events: `Event::Blast`, drawn by `client::blast` (an additive bar that fades and thins, shake by distance in `CombatFeel`). Sound: a new `Kind::Blast` pick list (`blast_` sounds: laser, crunch, boom from Kenney Sci-fi); its own sound is the green crystal's breaking sound, so breaking one by hand zaps too.
- The crosshair hint lists any aimed material's properties in a few words (`crafting::property_words`).
- Lab (`base::LAB_SPOT`, waypoint `lab`): a lumite lamp behind a phasite shutter on a lever; a lever-fired crystal gun that chains through a second green crystal into a brick wall.
- **Played** (2026-11-08), the user: "its fun. I like the gun, the lamp felt pointless wihtouut lighting, but thats just that." The lamp waits for Stored light. Done-when 1 and 2 not yet checked one by one.
- **Beam tuning deferred** ("maybe deffer tuneing it"). Today every beam is alike and one fires per lens piece and direction, so several separately placed green pieces in a boxel stack beams (more damage, a longer tunnel) while one green gadget, however layered, fires once: a hidden rule. Candidates: strength and damage from the green volume used up, and the beam's width from the lens's outline down the beam.
- **Lag found:** a 3D checker gadget made the placement ghost laggy, and interacting next to it hitched. A fault in the mesher from earlier toys, fixed on its own branch (`fix-mesher-cost`, 2026-11-08; the user: "ok it works very well"): see `docs/notes/mesher-cost.md`.
- **Finding from building the gun:** with the grab at 1 texel and the table working in quarters, a player can't make a hidden blue crystal: antiphasite only cuts a quarter-thick blue, and the kept part still touches the green. The lab's trigger is texel-exact, built by the base mod (`shapes::crystal_trigger`). With `grab` at 4 the quarter version works. Worth deciding with the user: how a player sets crystals off from a distance.
- **Glitch reported** (2026-11-09, not yet looked into or sorted as slice bug / earlier-code fix / design change). The user: "the losse block maybe we need vocab for that, only displays one material": the thing lying on the ground after a break seems to show only one of its materials. No Terminology word for it yet; agree one with a concrete example, then add it to the table.

### Toy backlog

- **Rename "part" to "gadget" in the code.** Added 2026-11-07, deferred from Blueprint library at the user's request: the Terminology table says gadget, the code still says part.
- **Hotbar background not quite snug.** Added 2026-11-07 at the user's request, after `fix-hotbar-shrink`: the background now shrinks when slots go, but in play not all the way down to its contents. Minor, maybe not worth fixing. Unchecked guess: the line under the slots (what's held and the placement mode) is wider than the slot row, so it sets the width.
- **Tectonic-like terrain shape.** Added 2026-11-05 at the user's request, from Generated terrain: the generated mountains are pointy, unlike Tectonic's broad ranges with plateaus, flat tops, terraced tiers and valleys. Reshape `base::terrain` toward it (Tectonic's regions and curves are MIT under `data/tectonic/`; its `data/minecraft/` files are Mojang's and stay out). Tuning `jaggedness`, `range_scale` and `mountain_height` softens the current shape meanwhile.
- **Rethink the inventory.** Added 2026-11-02 at the user's request, during Emergent shapes: "the last toy really was horrible… I was planning on just removing it but got talked out and now every problem with this one is caused by it". The Inventory and items toy's capacity limit and paying for things by volume caused each problem found while playing Emergent shapes: patterns made from parts refused once plain wood ran out, crafting needed room to carry, and dirt couldn't be used. Crafting was made free and an unlimited carry was proposed in that toy. Decide whether a carrying limit or costs belong in the game at all before building on them again. If something like them comes back, it should be because a toy needs it.
- **Covered blocks.** Added 2026-10-31, deferred from Solid materials. Should blocks be roughly one material, with grass and snow a cover on a block rather than part of it? What should a cut through grass show: honest (a bottom slab is all dirt, since the look spans the boxel) or anchored (the look's top follows each box's top, so a grass slab has grass on top, at the cost of seams between a stair's steps)? Settle it once something needs to cut grass, like world generation or crafted shapes.
- **Lower contrast in the other looks.** Added 2026-11-02 at the user's request, from Wood and brick looks: the lower contrast given to bricks and wood "should eventually be added elsewhere" (stone, dirt, gravel, ice and so on). Mostly recipe numbers in `blocks.toml`; `noise` may want a speckle-like setting.
- **Generated detail in looks, plus mipmaps and distance filtering.** First of the next graphics toys (order agreed 2026-11-05: this, then Stored light, then the GPU boxel grid). Run look noise in the shader on the world texel position, so noisy looks (dirt, stone, gravel, ice) never repeat; then generated detail over baked looks (wood grain that follows each board, per-brick speckle), then a large-scale patch tint. Anti-aliasing designed once for both kinds of look: mip levels for baked looks, and fine generated noise fading toward its average with distance (the user expected mipmaps in Shading). Notes: `docs/notes/generated-detail.md`.
- **Finer terrain shapes.** Added 2026-11-05 at the user's request ("not now"): now that faces show detail below a block, let world generation make shapes finer than a block, perhaps down to eighths (8×8×8 bits a boxel) rather than full texels, for smoother slopes, ledges and rocks. Open: how such shapes are stored (today a block's shape is a list of boxes, and crafting registers each new shape as a block), what they cost to mesh and collide, and how creatures walk them (see Creatures on terrain).
- **Stored light.** Second of the next graphics toys (agreed 2026-11-05, after Generated detail): flood-fill light per boxel with red, green, blue and sky channels (the design doc's read-old automaton), baked smooth per vertex, relit on edits and across chunk borders, plus a first block that gives light so there is something to see. Open: whether it lives in `sim` (gameplay like spawning or plants can read it) or only in the client.
- **GPU boxel grid: sun shadows and true AO.** Third of the next graphics toys (agreed 2026-11-05, after Stored light). A copy of the boxel grid around the camera on the GPU at texel resolution, ray-marched by the shader (the design doc's way): real cast shadows from the sun only, and real texel-fine ambient occlusion that replaces Shading's crease and ridge lines, which can't see an occluder's size, a gap of a texel or a diagonal touch. One piece of infrastructure for both, so it's the method to try first; measure on the Iris Xe against Bevy's shadow maps (redraws ~10M triangles per cascade) or shadows baked per vertex (cheap, but fixes the sun) only if it's too slow. *2026-11-07, the user:* the sheen (Blinn-Phong) must be aware of boxels between a face and the sun: a highlight shouldn't shine on a face in shadow (ice under an overhang glinting). The sun's shadow test should gate the sheen as well as the direct light; see `docs/notes/shading.md`.
- **Robot voice and jump sound.** Deferred from Sound (2026-11-05): espeak lines for the robot (knocked out, back up, low health; rendered to files offline by `tools/`, so the game never links GPL code), and a jump sound (a push-off of the surface plus a pickable `jump_` sound). Notes: `docs/notes/sound.md`.
- **Sound juice.** Added 2026-11-05 at the user's request, during Sound: the standard game-audio juicing effects, on `kira`'s tracks and effects. Candidates: muffle everything (low-pass) for a moment when hurt and while health is low; drop pitch and slow sounds during hit-stop; duck other sounds under a big hit; a low sub-thump layer under heavy hits and slams; a little saturation on head hits; short reverb tails on big impacts; louder and brighter the faster you're moving. All tunable, each worth playing with on its own.
- **Acoustics.** After Sound: muffling behind walls and room feel (reverb from rays cast out from the ear) on the boxel grid; sound around corners later; full acoustic ray tracing skipped. Notes: `docs/notes/acoustics.md`.
- **Plank board ends.** *2026-11-02:* Wood and brick looks' woven planks may settle this, since east and west faces now show boards too; check once played. Added 2026-10-31, from Solid materials. Planks stay lined up with the world, so a wood panel or stair facing east or west shows a grid of board ends instead of boards. The user dislikes it but accepted it as a compromise for now. Options: a pattern that reads as boards along both level axes, letting a material turn only about the up axis, or picking the board direction per block when placed.
- **Items inside placed blocks.** Added 2026-10-30, from Loose items. A block placed over a resting item hides it inside the block; it can still be picked up. Push items out the shortest way, or refuse the placement, once it gets in the way.
- **Place on left click, break on right click.** Added 2026-11-01 at the user's request, from Inventory and items: "to keep controls consistent, placing should be left click and breaking should be right", so a block's main use is placing, as a tool's is swinging. Retire the old `break`/`place` names in the controls file (they'd bind the wrong action) and move the user's lines to `use`/`alt_use`. Revises the Skyrim-style controls decision. Fits with Generative crafting or Interaction controls.
- **Two hands.** Added 2026-11-03 at the user's request, the other half of Skyrim-style controls: two hand slots; one item held (or a two-handed one) keeps main use on left and alt on right, but two items, one per hand, give each button that hand's main use. A block in one hand and the club in the other places and swings without switching slots. May replace Place on left, break on right: a block's main use becomes placing, and breaking is its alt when held alone or a tool's main use (a pickaxe) in the other hand. To settle: how items go into each hand, which items are two-handed, what the hotbar shows. Fits Interaction controls' one input, one action.
- **Crouch to collect.** Added 2026-11-01 at the user's request: "your a little robot and crouching sucks the items in". Items are only pulled and picked up while crouching; walking over them does nothing. Proposed: an `ItemDropDef` flag the base mod turns on, a somewhat wider pull (about 3 m) and a shorter throw delay (about 0.5 s) since throwing and crouching rarely overlap, and maybe a visible suck-in later. Open: whether flying with the shared Ctrl key (fly-down) vacuums too.
- **Interaction controls.** Split out of Block editing (2026-10-25). Fix Minecraft's ambiguous right click once there are things to use, like crystals or devices. Principles: one input, one action (no catch-all "use" button that changes meaning with what you hold or look at), and what you see is what you get (a ghost of the result, red when not allowed).
  - *Added 2026-10-28:* decide how players reach shared boxels. Placing into an occupied boxel makes placement harder to read (every click has two possible boxels), so it may become a deliberate trick (a key, a tool, or a property of blocks like water) instead of happening on any click. Decide once combat and usable items show how crowded the controls get; the engine keeps full support either way. Also from play: two slabs at right angles make a stair, so a separate stairs block may not be needed.
- **Block interactions.** Unstable crystals: two block types, a touch trigger, a directional blast, and a way to push blocks so devices are possible.
- **Review placement against the checklist.** Added 2026-11-06 at the user's request: placement grew by fixing annoyances one at a time; check its rules against the checklist in `docs/notes/material-behavior.md` (no hidden tiebreaks, one sentence, predictable). *2026-11-07, the user:* placing rules are tied closely to the problems with crafting, so this belongs to proving crafting (see `docs/notes/crafting-3d.md`).
- **Speed build.** Added 2026-11-06 at the user's request, as a way to find better rules: the game sets placing challenges (start in a hollow cube, put stairs in every inner corner, vertical ones too) and the user builds them as fast as they can. Compare rules or two kinds of stairs, then write a short report on what felt good and what didn't. Notes: `docs/notes/speed-build.md`.
- **Test worlds for comparing rules.** Added 2026-11-07 at the user's request: toys like crafting and placing may be best judged as little games. Build a set of things (a door, a trapdoor, a mesh), save that world, then load it again under changed rules and feel how many steps each takes and what got better or worse. Needs saving. Goes with Speed build. Notes: `docs/notes/speed-build.md`.
- **Recheck the crafting rules.** Added 2026-11-06 at the user's request, during Functional crafting: a restriction should exist only to make players build blocks they know how to place. The top and side views accept the same stacks, but each makes shapes rest on a different face (the bottom, the back), and "nothing floats" means a knob beside a panel can't be built on top. A gentler rule (a layer may touch anything already connected to the base) would let both views make doors. Pairs with the 3D crafting view.
- **3D crafting view.** Added 2026-11-06 at the user's request, during Functional crafting: "maybe we should re think the grid maybe a 3d gui is more natural here, but that can be done later". Replace the 4×4 grid of stacks with a 3D view of the boxel to build in. *2026-11-06, from Material behavior:* the user enjoys designing components but fights the UI and hates building near-identical parts that are just rotations of each other ("like MC but shrunk down"). Notes: `docs/notes/crafting-3d.md`. *2026-11-07, after Blueprint library:* one 3D view to draft in, a transform pop-up to drag, turn and mirror, drafting apart from crafting, and the whole loop with placing looked at together; see the notes.
- **Crafting follow-ups.** *Items 1, 2 and 4 done in Functional crafting (2026-11-06); the edge mode (3) is left.* Four changes agreed with the user, to plan together: several materials in one shape as a bundle, building on the side of the table, an "edge" placement mode on middle click to compare, and no upside-down flip on the upper half of a wall. Notes: `docs/notes/crafting-follow-ups.md`.
- **Generative crafting.** A few materials, a few property ingredients and a capacity limit. The user aims for fully emergent crafting ("emergent shapes, authored verbs": properties come from materials, so even doors come out of the system), with a simple crafter whose odd recipes the advanced one explains. Drag and drop is designed here. A proof toy: trigger, toggle and listener, enough for a door and a lever. Notes: `docs/notes/generative-crafting.md`.
- **Cow looking.** Cows turn head and neck first and react by degrees (notice and carry on, face it, back off), like real cows; herds face a newcomer together. Notes: `docs/notes/cow-looking.md`.
- **Head hit boxes that follow the animation.** Hit boxes are fixed to the body and don't follow the animated head, so a grazing cow's head box stays where the head was. A headbutt needs the head box where the head is: move boxes with their bones, or have the brain pick box poses per clip.
- **Solid creatures.** Added 2026-11-03 at the user's request: enemies in many games feel unsolid, and standing on them would help. The player collides with creatures solidly and can stand on them, carried as they move (jump on a cow to escape, get bucked off, cross a herd on their backs). Notes: `docs/notes/solid-creatures.md`.
- **Stun by impact angle.** Added 2026-11-03 at the user's request: whether a charging cow is stunned should depend on the angle it hits the wall at. Today any charge that gets blocked on the ground (after a 0.15 s grace) stuns for the full `stun_time` (`base::cow`, Charge mode), because the brain only gets `BrainContext.blocked`, a yes or no. Needs the engine to report the impact: the wall's direction or the speed lost into it, like a slam's `into`. Then a head-on hit stuns fully, a glancing one scrapes along the wall or stumbles briefly, perhaps with stun time scaled by the speed into the wall.
- **A real hind-leg kick for the cow.** Added 2026-11-03 at the user's question, from Fighting cow ("do you have the animations for the kick?"). The cow model has no leg kick; its close attack is a head toss (the `punch` clip). A kick needs a new clip, the rear legs hand-keyframed in the model file, and an attack for when the player is behind the cow.
- **Combat depth.** A higher skill ceiling once enemies, tools and movement give combat something to tune against: swing direction from the mouse, air combos, a sweet spot, recovery cancels, a charged heavy swing, health from close kills, and player wall slams. Notes: `docs/notes/combat-depth.md`.
- **Liquid rule sets.** The rule sets from the design doc.
- **Advanced movement revisited.** After toys add tools, items and enemies. Rework push-out (block-jumping): real motion instead of a teleport, how much goes up, placing while crouched, a jump wiping out the kick, and a known flaw where the kick can point opposite the shift. The user's candidate directions (kick straight up, an analog kick, order-independent jumping) are in the notes. Notes: `docs/notes/advanced-movement-revisited.md`.

Ideas that come up while playing get added here.

### Infrastructure (added when a toy needs it)

- ~~Saving and loading, with block names in the save~~ Done (Saving, 2026-11-07), see Done toys. Creatures aren't saved yet.
- World generation. *The generator is done (Generated terrain, 2026-11-05); streaming is next, as startup meshing grows with the area.* Staging (generator, then streaming, then saving and LOD) and the creatures-on-terrain checklist. Notes: `docs/notes/world-generation.md`.
- LOD, once world generation exists
- Meshing off the main thread, once a remesh can take longer than a frame in ordinary play. Deferred from `fix-mesher-cost` (2026-11-08): costs late meshes and edits racing older work; finely cut gadgets are fast enough without it for now.

### Seam backlog (added when the game is ready for them)

- ~~**Data mods.**~~ Done (2026-10-27), see Done toys.
- **WebAssembly behavior.** *First part done (2026-11-06), see Done toys: the cow brain runs as WebAssembly and movement keeps its state in the engine.* Left: block behavior once a toy has some, and giving freed mod pages back once a mod grows big. Notes: `docs/notes/wasm-behavior.md`.
- ~~**Local server.**~~ Done (2026-11-05), see Done toys. Next steps toward multiplayer are in its Deferred list.
- **Sub-grid trial.** A rotating platform the player can stand on and be carried by, to test collision with moving, rotated grids before more physics piles up. Whenever a toy wants something that moves. Notes: `docs/notes/sub-grid-trial.md`.
- ~~**Sandbox limits.**~~ Done (2026-10-28), see Done toys.
- **Seed a modder.** Once a few seam slices are done, have an outside modder build a small toy on the API and fix what blocks them.

### API debt

Shortcuts taken around the API, each with the toy that needed it. Paid down in seam slices.

- **Fixed input actions** (movement toy; break, place and palette slots added by the block editing toy, crouch by the advanced movement toy, placement mode by the block shapes toy, drop and shape by the inventory toy; shape replaced by interact, and turn_part added, by the emergent shapes toy). Move, jump, sprint, crouch, use, second use, drop, shape, the palette slots and the debug keys are a fixed engine set in `sim::PlayerInput` and `client::controls`. Mods can't define their own actions. API version: mods register named actions with default bindings, and controllers read actions by name. The `key_actions!` table in `client::controls` (name, default, HUD text) is where registered actions would plug in.
- **Chase drill in the client** (advanced movement toy, temporary; ghost hits added by the combat feel toy). The ghost and objectives in `client::drill` are game logic living in the client, and it finds landmarks by the `landmark:` prefix on waypoint names. Kept by the local server seam (2026-11-05) as a good game for feeling the movement; it reads the client's copy of the world. Remove the drill rather than moving it into the API, unless a later toy wants real enemies, which would need entities and a game-rules hook in the API.
- **Tool look in the client** (combat feel toy). Every tool is drawn as the same club, posed by `client::combat`; mods can't give a tool its own model or swing poses. API version: a tool names a model and its poses.
- **Crafting menu in the client** (emergent shapes toy). The table's menu, its strokes and the turn and flip controls live in `client::crafting`; mods can't change how crafting is presented. The rule itself is in `api::crafting`. API version: a client-side mod API for menus, or a data description of the table's grid.
- **Fixed block properties** (functional crafting toy; `hinge` and `listener` replaced by `conductor`, `phase` and `starts_hidden` in the material behavior toy). The properties are a fixed set in `api::Property`, worked by `sim::signal`; mods give them to materials but can't write their own. API version: block behavior as WebAssembly (the seam backlog's WebAssembly behavior), with properties naming a mod's handler.
- **Health bar in the client** (fighting cow toy). `client::hurt` draws the player's health and the hurt flash; mods can't change how health is shown. API version: a client-side mod API for the HUD.
- **Sound kinds in the client** (sound toy). What makes a sound (steps, landings, swings, hits, kills, hurts, knock-outs, pick-ups, the servo and wind loops) and the tuning panel's pick lists are a fixed set in `client::sound` (`Kind`, `Hearing`); mods name sounds for these slots but can't add kinds or play sounds for their own events. API version: mods attach sounds to named events, or a client-side mod API for sound.
- **Camera feel in the client** (movement toy; hit-stop, view kick and shake added by the combat feel toy in `client::combat`). FOV widening, head bob, landing dip and step smoothing live in `client::feel`, so mods can't change them. API version: a client-side mod API for camera effects, presumably driven by the same tunables.

### Phase 2 decisions

- **A robotic player** (tentative, 2026-10-25): the user wants movement and camera feel to read as a robot. Landings are a shock absorber: soft landings compress smoothly, damaging ones bottom out hard. Later feel work (combat, damage) should keep to it.
- **Skyrim-style controls** (2026-10-31, from Combat feel; clarified 2026-11-03): whatever is held has a main use on left click and a second use on right click. Blocks break and place; a tool swings and guards. Part of the answer to the ambiguous-interactions gripe; the Interaction controls toy builds on it. *Only half of Skyrim's system is built:* the one-item case. In Skyrim, holding two things (one per hand) gives each button that hand's main use, and alt uses go unreachable; that half is the backlog's Two hands.
- **Sound** (2026-11-05, from Sound): the simulation is silent; the client hears what the server sends (events and snapshots), so new sounds need events, not sim audio. Sounds are data in `sounds.toml`, named by kind prefix (`step_`, `swing_`, `hit_`, `kill_`, `hurt_`, `knockout_`, `hardland_`, `pickup_`), and the mod keeps only used files; candidates live in `~/Projects/boxel-sound-packs/`. Loops are WAV. `kira` stays at 0.10 until Bevy is upgraded (ALSA clash). espeak, if used, is rendered offline.
- **Creature navigation** (2026-11-05, from Cow tweaks; revised 2026-11-06, Cow pathfinding): pathfinding is an engine service. A brain asks with `BrainOutput.go_to` and gets `BrainContext.route` a tick or more later; the engine searches standing spots with the real collision sweeps (so shaped cells work), hops one-block ledges for creatures with a `hop_speed`, drops at most `max_drop`, and runs one search a tick. Brains still see `Seen.clear_run` for straight lines like charges, which keep no edge guard and can go off cliffs; creatures take fall damage.
- **Crafting frames** (2026-11-06, from Functional crafting): the table's top and side are two frames of one rule (a side build is a top build tipped so the wall is the far side), each view keeping its own grid; a part used in a layer keeps how it stands in the world, in either view. Several materials make a bundle, placed whole into one boxel as one occupant per material.
- **Blueprints** (2026-11-07, from Blueprint library): a player's blueprints live in `sim`, saved per world by name; a variant is linked to its base (worked out from it every time); a layer may hold a blueprint, so a design never needs its inner gadgets carried; refused edits leave the library as it was. Drafting (designing a blueprint) and crafting (making from one at a table) are separate things. Menus never move what's under the pointer when something changes size.
- **Generated terrain** (2026-11-05, from Generated terrain): generators are pure per-chunk functions through `api::terrain`, plain data, positions relative to their terrain's center; the engine decides how much to fill (`world` section) and fills it explicitly in `game::load`. Regenerating replaces the terrain (edits lost) and lifts buried bodies out. Beyond the simulation distance creatures and items hold still.
- **Phase materials and signals** (2026-11-06, from Material behavior): what materials do comes from combinable properties (`trigger`, `conductor`, `phase`, `starts_hidden`), never door code. A signal flips each phase group it reaches, shows before hides, all or nothing; hiding takes everything within reach (1 texel) of the group, as derived kinds that join back exactly, so what's hidden is always what's within reach of a hidden group. Pieces coming back push the player out like a placed block; creatures and solids block the whole signal. Hidden pieces are seen and touched only while holding a phase material. Conducting is a flood fill when the signal is sent, not stored networks.
- **Saving** (2026-11-07, from Saving): saves keep names, never IDs, and blocks made while playing as recipes; generated terrain is made again from its saved values, so a save stores only the chunks edited since generation plus everything outside the terrain. Any new world state (creatures, fields, sub-grids) needs a place in `sim::save` and a format version bump. A save that can't be read stops the game rather than being saved over.
- **Endless materials** (2026-11-07, from Endless materials): a material the world can't give back may be listed as `endless` in `inventory.toml`: always at least 64, past any capacity, refilled every tick and after a load, shown as ∞. A stopgap until "Rethink the inventory".
- **Signature feature:** not decided yet. Choose it once the block interaction and crafting toys have been played, and record the choice and why in the design doc.

### Fixes

Bugs found in already-merged code, each fixed on its own `fix-` branch (see the workflow in `CLAUDE.md`). One line each: date, branch, what was wrong.

- 2026-10-28, `fix-step-smoothing`: a step-up moved the body but not its previous position, so the renderer blended the rise between ticks while the camera feel also eased it, making the view dip and snap when climbing. Latent until the block shapes toy gave the player a step height.
- 2026-11-07, `fix-step-under-low-ceiling`: a step-up rose the full step height if the ceiling overhead allowed it, ignoring a lower ceiling ahead, so a crouched player got stuck where a narrowing's floor and roof both had a small lip (e.g. 3/4-texel pieces in a 2-high tunnel).
- 2026-11-07, `fix-hotbar-shrink`: the hotbar's background grew with new slots but never shrank, as its centered contents spread out to the width egui remembered from the last frame. It now sets this frame's width; a small leftover gap is in the backlog.
- 2026-11-05, `fix-clear-run-hang`: a cow's clear-run sweep never counted a sub-millimeter last piece as blocked, so with the player a hair away and the cow against something it looped forever and locked up the game (the server runs in the client's tick).
- 2026-11-06, `fix-crafting-menu-layout`: the crafting window sized itself to its content and stayed centered, so the first crafted part's row (wider, with its buttons) widened it and moved the grid; the carried list was capped shorter than its column. Columns now have fixed widths and the list fills the tallest column.
- 2026-11-06, `fix-controls-table`: adding a key action meant five edits (field, default, parser arm, HUD format slot, the player's controls file). Now one row in `client::controls`'s `key_actions!` table gives the field, default, config name and HUD hint, and loading warns about actions the controls file doesn't list.
- 2026-11-08, `fix-mesher-cost`: a two-material texel checker gadget (2,048 boxes per material) made the ghost lag and edits beside it pause: every face scanned every box around it, faces of a boxel's pieces were compared all against all, buried faces between pieces were drawn (24,576 quads where 1,536 show), and the hotbar re-laid out its picture every frame. Now texel grids, faces covered whole are left out, and pictures are kept; a chunk of 20 checkers remeshes in 41 ms instead of 220+. Notes: `docs/notes/mesher-cost.md`.

### Done toys

Full write-ups (plans, implementation notes, verify rounds, decisions) are in `docs/done-toys.md`. One line each, newest first:
- **UI scale** (2026-11-08, `toy-ui-scale`, mini). Menus and on-screen text scale with the window's height (natural at 1080), times a size slider (0.5–1.5) in the tuning panel; egui and Bevy UI together.
- **Endless materials and a bigger sun** (2026-11-07, `toy-endless-materials`, micro). `inventory.toml` can list endless materials, kept topped up and shown as ∞ (base: brass, copper, phasite, antiphasite), which also fixed a save that had lost its copper; a bigger, crisp sun with flat drawn rays, all on sliders.
- **Blueprint library** (2026-11-07, `toy-blueprint-library`). Blueprints kept per world by name, made without crafting; linked variants (turn, mirror, shift a quarter, center, swap a material); a blueprint can go in a layer, so inner gadgets needn't be carried; a tree in the table menu, carried things as an icon grid, pictures that turn. The menu was judged too heavy: next comes one 3D drafting view.
- **Saving** (2026-11-07, `toy-saving`). Worlds are saved by name to `~/.local/share/boxel/worlds/`: blocks by name and crafted ones as recipes, terrain made again from its saved values with only edited chunks stored; the player, inventory and loose items come back, cows start fresh. Escape opens a world menu (save, reload, save as, open, new world, save and quit); F2 saves, F8 reloads, quitting saves; `--world <name>`.
- **Material behavior** (2026-11-06, `toy-material-behavior`). Phasite and antiphasite replace the hinge: a signal flips them between shown and hidden, taking what's within a texel of them, so a Minecraft door is two panels that swap; copper conducts; bundles break whole; hidden pieces show as ghosts while holding a phase material; signals are all or nothing and push the player out; up to 8 materials a design, 16 occupants a boxel; a checklist for rules.
- **Functional crafting** (2026-11-06, `toy-functional-crafting`). Several materials in one crafted shape (a bundle, placed whole, broken piece by piece); building on the table's side; a grid per view and loading a part back into it; a first try at materials that act (a door and lever from brass, iron and copper), to be redone in Material behavior.
- **WebAssembly behavior seam** (2026-11-06, `seam-wasm-behavior`). The cow brain runs as a sandboxed WebAssembly component (`base_cow`), bit for bit the same as native; movement controllers keep their state in the engine; `--native-brain`.
- **Cow pathfinding** (2026-11-06, `toy-cow-pathfinding`). Cows find paths round walls, through gaps and up block-high ledges with a hop when chasing, running off and regrouping; with no way up they wait below; charges run off cliffs and falls hurt. F7 shows the routes; the pasture has a course (pen with a gap and terraces, pillar, mesa).
- **Health bar under the hotbar** (2026-11-06, `toy-health-bar-bottom`). The health bar moved from the top of the screen to the bottom, under the hotbar, which sits higher to make room.
- **Shading** (2026-11-05, `toy-shading`). Crisp crease and ridge lines per texel from what's past each quad's edges, an AO curve with an Apply button, light from a movable sun and the sky instead of fixed brightness per face, a sheen per look (ice), a gradient sky with a sun and fog that fades the far world into it. About 2.7 ms a frame at 10M triangles. Also an F9 frame rate recorder (on `master` via `dev-perf-recorder`).
- **Generated terrain** (2026-11-05, `toy-generated-terrain`). A 512³ seeded terrain of hills, ranges, frozen lakes and boulders (Tectonic-inspired), its own place reached by teleport, with herds; live render and simulation distances, a regenerate, and a cost readout. 1024³ runs at about 25 fps; startup meshing (20 s) is the first limit.
- **Cow tweaks** (2026-11-05, `toy-cow-tweaks`). Cows charge only with a clear straight run (`Seen.clear_run`) and run well past; hurting a cow gives it and its herdmates a grudge until a knock-out (fight or run when you come near); giving up means running, and a cow run down turns and fights; the pasture is walled 8 high.
- **Sound** (2026-11-05, `toy-sound`). Positional sound through `kira`: footsteps and landings by surface, combat, block break and place, pick-ups, a printer-like servo on fast pans and wind at speed, with group volumes and in-game pick lists (and F5 reload) for trying sounds.
- **Local server seam** (2026-11-05, `seam-local-server`). The client talks to the simulation only through `protocol` messages and keeps its own copy of the world; single-player runs a local server; hit-stop freezes only the view; the block-jump trainer is gone.
- **Sprint toggle** (2026-11-04, `toy-sprint-toggle`). Sprint is a toggle (off on a second press or letting go of forward); default keys swapped so Shift sprints and Ctrl crouches and flies down.
- **Release mode and saved tuning** (2026-11-03, `toy-release-mode`). Release builds hide dev text (F1 flips it); the tuning panel saves its sliders to `~/.config/boxel/tuning.txt`, loaded at startup.
- **Fighting cow** (2026-11-03, `toy-fighting-cow`). Hit cows fight back with a head toss up close and a telegraphed charge from afar; charges into walls stun; the herd defends or runs; player health, regen, knock-out to spawn (resets the herd), fall damage, air recoil from hits.
- **Cow herd** (2026-11-03, `toy-cow-herd`). Cows keep together, follow a leader, spook together, and push apart (soft push, player included).
- **Cow brain** (2026-11-03, `toy-cow-brain`). A brain API like the movement controller (state as plain data in the engine); cows wander, graze, watch the player, flee, and walk for real.
- **Cow animation** (2026-11-03, `toy-cow-animation`). The cow model's clips play and blend; any clip can be previewed from the panel.
- **Ambient occlusion** (2026-11-02, `toy-ambient-occlusion`). AO baked per vertex in the mesher from neighbors measured in texels; strength 0.6, reach 16.
- **Wood and brick looks** (2026-11-02, `toy-wood-brick-looks`). Bigger, woven planks and staggered bricks with lower contrast, told apart by color rather than outlines.
- **Remove the charged jump** (2026-11-02, `toy-remove-charged-jump`). The crouch-charged jump is gone; the mid-air tuck stays.
- **Emergent shapes from crafting** (2026-11-02, `toy-emergent-shapes`). Shapes come from a 4×4 grid on a crafting table, pushed through by the face used; parts go back into the grid; every base shape is crafted.
- **Inventory and items** (2026-11-01, `toy-inventory`). The player carries amounts of materials sharing one volume capacity; placing costs volume; a drop key. (The user later found the limit harmful: see Rethink the inventory.)
- **Combat feel** (2026-10-31, `toy-combat-feel`). Skyrim-style controls (main use left, second use right), a club, cows as targets with hit boxes, head hits, knockback, wall slams and tunable juice.
- **Solid materials** (2026-10-31, `toy-solid-materials`). Looks are data recipes baked into 16³ solid volumes, so any cut shows the inside; no outside art for blocks.
- **Block-jump trainer** (2026-10-30, `toy-block-jump-trainer`). A dev tool (F5) that judges each block-jump attempt.
- **Loose items** (2026-10-30, `toy-loose-items`). Breaking drops debris of the block's material that tumbles, settles and is pulled in and picked up.
- **Block shapes and shared cells** (2026-10-28, `toy-block-shapes`). Shapes as boxes in texels, 24 orientations, up to 4 occupants per boxel, basic and precise placement.
- **Sandbox limits seam** (2026-10-28, `seam-sandbox-limits`). Runaway WebAssembly mods (loops, memory, recursion) are stopped and the game carries on.
- **Data mods seam** (2026-10-27, `seam-data-mods`). Base game content loads from TOML and files in a mod folder; errors name the file and line.
- **WebAssembly movement seam** (2026-10-26, `seam-wasm-movement`). The base movement controller runs as a WebAssembly component on wasmtime, matching native bit for bit.
- **Edge guard** (2026-10-25, `toy-edge-guard`). Crouching keeps you from walking off edges.
- **Advanced movement** (2026-10-25, `toy-advanced-movement`). Crouching, the mid-air tuck and block-jumping (push-out), feeling robotic.
- **Block editing** (2026-10-25, `toy-block-editing`). Place and break blocks, Minecraft-style.
- **Movement mechanics** (2026-10-25, `toy-movement`). Quake-style movement decided by the base mod's controller, tunable live.

## Phase 3: Toward launch (not yet sliced)

Once the toys have found what makes the game fun and the API has grown with them:

- **The mastery loop.** A finite stressor that comes to the player, with at least two ways to solve it, built around the signature feature.
- **API hardening.** Clear remaining API debt, make WebAssembly mods deterministic, and add versioning so the API can be called stable.
- **More early modders,** following the design doc's path to modder critical mass.

After that: multiplayer (the local server seam split client and server in one process; networking, prediction and many players remain), lighting, sub-grids, world views, field layers, signal networks and the console.

Single-player is meant to run a local server, so every slice keeps simulation and rendering separate, even before networking exists.
