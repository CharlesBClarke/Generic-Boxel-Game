# Roadmap

Work happens in slices. Each slice goes through the stages in `CLAUDE.md`: **Not started → Plan → Implement → Test → Verify → Done.**

There are three phases. The middle one is the longest and the point of the early project: get something playable, then improve game feel one toy feature at a time. The mod API grows alongside the toys rather than coming at the end.

---

## Phase 1: Something playable

One slice: just enough engine to walk around in and start playing.

### Slice 1: Minimum playable engine

**Stage:** Not started

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

---

## Phase 2: Play with game feel

A run of small toy slices. Each one takes a single game-feel feature that doesn't need much else, makes it playable, and tunes it until it feels right. We pick the next toy together when the current one is done. The list below is a backlog, not a fixed order.

**Rules for this phase**
- **Infrastructure arrives when a toy needs it.** World generation comes when a toy needs more space or variety than the test area. Once world generation exists, we start thinking about LOD. Saving comes when losing your work between sessions gets in the way. Each of these becomes its own slice at that point.
- **Toys go through the API.** When a toy needs something the API can't express, extend the API. A shortcut around it is allowed to keep a toy moving, but it goes in the API debt list below.
- **Seam slices along the way.** Every few toys, or when the API debt list gets long, run a short slice that pays down debt or adds one real mod feature. The seam backlog below lists the candidates.
- **Tuning beats polish.** Values should be adjustable while the game runs where that helps.
- **Tests cover the logic, playing covers the feel.** Verification for a toy is mainly the user playing it and deciding whether it feels right.

### Current toy

None yet. Picked after slice 1 is done.

### Toy backlog

- **Movement mechanics.** Acceleration, friction, jump, step-up, and anything else that makes moving feel good.
- **World interaction controls.** Breaking and placing with clear, predictable inputs that show what will happen before you press, fixing the "ambiguous interactions" gripe.
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
- **WebAssembly behavior.** Sandboxed mod code for block behavior, once a toy has behavior worth sandboxing, such as crystals or liquids. Sleeping blocks and read-old, write-new automata.
- **Seed a modder.** Once a few seam slices are done, have an outside modder build a small toy on the API and fix what blocks them.

### API debt

Shortcuts taken around the API, each with the toy that needed it. Paid down in seam slices.

None yet.

### Phase 2 decisions

- **Signature feature:** not decided yet. Choose it once the block interaction and crafting toys have been played, and record the choice and why in the design doc.

### Done toys

None yet.

---

## Phase 3: Toward launch (not yet sliced)

Once the toys have found what makes the game fun and the API has grown with them:

- **The mastery loop.** A finite stressor that comes to the player, with at least two ways to solve it, built around the signature feature.
- **API hardening.** Clear remaining API debt, make WebAssembly mods deterministic, and add versioning so the API can be called stable.
- **More early modders,** following the design doc's path to modder critical mass.

After that: server/client split and multiplayer, lighting, sub-grids, world views, field layers, signal networks and the console.

Single-player is meant to run a local server, so every slice keeps simulation and rendering separate, even before networking exists.
