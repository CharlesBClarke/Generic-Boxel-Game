# Boxel Game Design Document

Oct 18, 2026 · @Charles

## Project goal

Build the ideal foundation for a community-content boxel survival game: as unlimiting as possible for modders, with just enough built-in content that the game is enjoyable on its own.

The project succeeds if modders choose to build on it. Modders follow players, so the base game has to be worth playing before the modding API can win anyone over.

**Principles**

- Learn from other boxel games, Minecraft first, and remove the engine assumptions that forced their mods into hacks.
- Build every piece of base content through the public modding API, so the game proves the API works.
- Treat early development as an experimental zone: use content to find what makes the game stand alone, and to find gaps in the API.
- Launch the game, not the platform. "Moddable" is the second selling point, not the first.

**Path to modder critical mass**

1. Vertical slice: find the fun with small playable toys, building toward a loop around one signature feature Minecraft can't do well.
2. Open the seams as we go: toys use the API from the start, the API grows when a toy needs more, and any shortcuts are tracked and paid down along the way.
3. Seed modders early: give the tools to a handful of modders before launch and fix what blocks them.
4. Launch the game, with those early mods as proof the API is real.

## Design pillars (tentative)

Two working pillars tie the rest of this document together. Both are hypotheses to test in the play phase.

**1. The mastery loop.** Satisfaction comes from taking control of your environment.

1. A stressor appears: creatures wreck your build, winter is coming.
2. The player chooses how to respond, using systems they understand.
3. They put in effort and solve it.
4. They gain control over that part of their world.

Power means control over the environment, not bigger stats. To test: stressors should have several possible solutions, and threats should be finite, so players can eventually earn a cozy life.

**2. Compositional complexity.** Players learn simple rules and express themselves by combining them. At the engine level this is the firmest idea in the document, because the modding goal depends on it: systems from different mods should combine without anyone planning for it. At the game level it is a hypothesis. To test: how deep the base game should go, whether players can predict results, and how to avoid one dominant combination.

The gripes, stories and tentative directions below are mostly ways of applying these two pillars.

### A checklist for rules

Rules grew by fixing little annoyances one at a time, and the result was rules nobody could predict (the hinge's tiebreaks and slides). Agreed 2026-11-06, from Material behavior: check every rule against this list before building it.

1. Predictable from what you see; if not, show it first (like the placement ghost).
2. No hidden tiebreaks: a rule that needs one is wrong; let the player choose, or refuse with a reason.
3. Fits in one sentence (the crosshair hint). "Unless" means it's too fussy.
4. Local and stateless where possible.
5. Undoable: doing it again gets back exactly.
6. Restrictions only stop the player making things they can't use or place.
7. Small verbs that combine, over clever ones with cases.

## Terminology

| Term | Meaning |
| --- | --- |
| Boxel | One grid cell of the world. Used instead of "voxel" throughout the project. |
| Sub-grid | A volume of boxels with its own position and rotation, separate from the main world grid. Ships, vehicles and contraptions are sub-grids. |
| World view | A camera into any grid, loaded and drawn by the client. The player's own view is one world view among possibly many. |
| Occupant | Something that fills a boxel position. One position can hold several occupants, which generalizes Minecraft's waterlogging. |
| Engine | The Rust core. Ships with no game content. |
| Base game | The built-in content, written as a mod on the public API. |

## Engine and mods

The engine is written in Rust and ships with no content. The base game is a mod built on the same public API every modder uses.

Early on, the API is plain Rust and the base game is compiled in against it. Data files and WebAssembly arrive in stages, as the game needs them.

**WebAssembly so far.** The base game's movement controller (since 2026-10-26) and cow brain (since 2026-11-06) run as WebAssembly components on wasmtime. Each kind of mod behavior gets a WIT interface in the API crate. Mods write the same plain Rust traits either way, and an export macro wraps them for the boundary. Components import nothing unless a host service grants it.

**Determinism.** Mod code must give the same results bit for bit, natively and in WebAssembly, on every machine, so replays and multiplayer stay in sync. The API uses the `libm` crate for trig rather than the platform's math library, and wasmtime canonicalizes NaNs. Tests check native and WebAssembly results bit for bit.

**Three kinds of mods.** One mod can combine several kinds.

| Kind | What it contains | Trust |
| --- | --- | --- |
| Data/asset | Blocks, items, recipes, textures, models, worldgen. No code. | Safe; servers can push it to players. |
| Sandboxed | WebAssembly code, usually bundled with GPU shaders and compute shaders. | Sandboxed; servers can push it to players. |
| Trusted native (deferred) | Compiled code with deep engine access. | Not built for now; see below. |

Sandboxed mods are expected to be the most common kind: game logic in WebAssembly, with compute shaders for heavy work like fluids or lighting.

**Mod memory.** Each sandboxed mod has its own memory, so space one mod frees can't be used by another. WebAssembly memory also never shrinks, so without care a large mod pack would use the sum of every mod's peak memory. Three rules keep it close to what a single shared heap would use:

- **State lives in the engine.** Entities and blocks a mod controls keep their state as plain data in the engine, passed in on each call and returned changed, like `MoveContext` and `MoveOutput`. A mod's own memory holds only working space and its fixed data, so it stays small no matter how many entities exist. Creature brains (since 2026-11-03) and movement controllers (since 2026-11-06) follow this fully: one per kind, each creature's or body's memory kept by the engine as a short list of numbers.
- **Mods give pages back.** The allocator the API supplies to mods tells the engine when it frees large page-aligned regions, and the engine returns those pages to the OS. Modders get this without doing anything.
- **Mods can be restarted.** A mod that holds no state of its own can be reloaded at any time: after a crash, to reclaim memory, or for hot reload.

Large temporary buffers, such as pathfinding grids or worldgen scratch space, can be engine-owned data reached through a handle. Per-mod limits on memory and fuel only stop runaways; they are not a share of RAM, and their total can far exceed it.

**The native tier is deferred.** Native mods can't truly be constrained, and most of what they would be used for is better covered in other ways:

- **Host services.** The engine offers capability-gated features that sandboxed mods can call once the player approves them, such as audio and voice streams, a networking channel, VR, a filtered web connection and video export. Mods using them stay sandboxed and can still be pushed by servers.
- **Open source.** Better core systems, like a faster renderer or lighting engine, are contributed to the engine itself or built as forks, rather than patched in by mods.

What's left for native mods is small: integrations with obscure hardware or software, and experiments that need raw speed or access before a host service exists. Engine interfaces should be designed so a native tier can be added later if needed. Requests for native access decide which host services to add next.

**Movement is content.** How a body moves (acceleration, friction, gravity, jumping) is decided by a movement controller the mod supplies; the base game's is Quake-style. The engine owns what every controller needs and what must stay consistent: body state, collision, impulses from any source, and the surface properties of what the body stands on. A knockback from one mod therefore works with another mod's controller.

### Simulation timing

- **Fixed base tick, likely 60 per second.** The simulation advances in fixed steps so results are deterministic, physics stays stable and multiplayer clients can stay in sync. 60 is a good base because it has many divisors.
- **Multi-rate updates.** Systems can run at integer fractions of the base rate (every 2nd, 3rd, 10th tick and so on). The engine staggers them so they don't all land on the same tick.
- **Sleeping by default.** Most blocks don't tick at all; they wake on a change or a scheduled time.
- **Parallel and lazy.** Independent regions of the world tick on separate threads, and unloaded areas catch up by calculation when players return rather than being simulated tick by tick.
- **Automata read the old state and pull, never push.** Boxel rules (liquids, heat, light, logic, mod field layers) read from the previous tick's copy and write a new one, and each boxel only decides what flows into itself. Update order stops mattering, nothing is created or destroyed by two cells pushing into the same spot, and any boxel can run on any core or on the GPU. Rules that move things, such as falling sand, can use alternating 2×2×2 blocks instead. The second copy is kept only for active areas. Signals travel one boxel per step, so fast circuits need several steps per tick, and discrete events like picking up an item use a separate claim system.
- **The API speaks in seconds, not ticks.** Mods write durations and speeds in real units; the engine converts them. This lets the base rate change without breaking mods. The exception is exact tick timing for contraptions, which is part of the stable API so player builds don't break.
- **Rendering uses delta time.** Rendering, animation and the camera run at the display's frame rate, blending between simulation ticks for smooth motion.

### Console

The in-game console is a bash-like shell, but it only ever touches the game. It is never a real system shell, which would let any player or mod with console access reach the host computer and break the sandbox.

- **Pipes and filters.** For example, `entities near 50 | where type=zombie | kill`.
- **Structured output.** Commands return data (entities, blocks, coordinates) rather than plain text, so commands from different mods pipe into each other reliably.
- **Variables, loops and saved scripts,** so admins can automate tasks.
- **Tab completion and history.** Mods register commands with typed arguments, and the console completes them.
- **Permissions per command,** checked on the server.

### Signal networks

Connected blocks such as wires, pipes and cables form graph networks rather than being simulated block by block. Each connected network holds a single value, so a whole line of wire is on or off together however long it is.

- **No per-tick cost.** Wire blocks only store a network ID. When a wire is placed it checks its six neighbors once and joins or merges networks; breaking one may split a network with a flood fill over that network only. Non-network neighbors are never checked again.
- **Event-driven.** When an input changes (a lever, a sensor), its network is marked and recomputed at the tick boundary. Outputs such as pistons and lamps wake only if the value actually changed, so idle circuits cost nothing.
- **Instant and unlimited range.** No distance falloff and no repeaters for range. Delay is added on purpose with components, as in Factorio's combinators.
- **Meets the automata once per tick.** Networks read inputs from the previous state and write outputs into the new one, so both systems stay deterministic. Crossing the boundary costs one tick, which is consistent and part of the stable contraption timing.
- **Parallel per network.** Separate networks solve independently on different cores.
- **Cheap to render.** Wires look up their network's value in the shader, so a lever lights the whole line without remeshing chunks.
- **Open choices:** whether values are on/off or numbers (and how inputs combine), and whether a wire can carry several channels.
- **Built so far (2026-11-06, Material behavior):** a much simpler stand-in. A conductor (copper) carries a signal from a trigger through face-touching boxels holding it, found by a flood fill when the signal is sent, with no stored networks and no values: a signal is a single pulse that flips what it reaches.

## World model

The engine's data model assumes these capabilities from day one, because retrofitting them would mean rewriting storage, physics, rendering and networking. Each one is built when the game needs it, but addressing and storage are shaped for all of them from the start: the world is one grid among many, and chunks store separate layers.

**Sub-grids.** Any volume of boxels can have its own position and rotation. Ships, vehicles, elevators and moving contraptions are sub-grids, with collision, lighting, rendering and syncing handled by the engine rather than each mod. A sub-grid can be moved by simple rules or by a physics controller, so rigid-body physics stays possible.

**Multiple occupants per boxel.** One boxel position can hold more than one thing, generalizing Minecraft's waterlogging beyond "a block plus water." Built in the block shapes toy (2026-10-28): a boxel holds up to four occupants, each a block and its orientation, in the order they were placed, and storage, collision, aiming, breaking and meshing all handle them. Where two occupants' faces coincide, the oldest one's face shows. This is a fundamental engine capability, deliberately there from the start; how players get at it (on any click, or as a deliberate trick) is still open. Since 2026-11-06 (functional crafting), a crafted design of several materials is placed as one occupant per material in one boxel, so the four-occupant limit is also the most materials a design can hold.

**Multiple world views.** The client can load and draw any grid from any camera, not just the world the player stands in. Uses include portals, security cameras and screens, mirrors, blueprint previews, structures shown inside menus, minimaps, replays, split-screen and pocket dimensions.

**3D chunks.** The world is split into cubes, not full-height columns, so worlds can be arbitrarily deep or high and only the parts near players are loaded.

**Level of detail (LOD).** Distant terrain is drawn at reduced detail, so players can see far without loading full chunks. 3D chunks make this easier, since each cube can be simplified on its own. Mods' blocks must work with LOD automatically, without extra code.

**Field layers.** Mods can register fields: a value stored across the world, such as temperature or fog density. The engine handles storage, saving, syncing, moving with sub-grids and fast updates. Each field chooses its own resolution, from per boxel to one value per region, and only stores places that differ from the default. Both gameplay and rendering can read fields: a fog field can thicken the air visually and limit how far creatures see. Which fields belong to the shared core vocabulary will be decided in the play phase.

**Coordinates.** +Z is up and coordinates are right-handed, with +X east and +Y north. Height is the odd axis, so the two ground axes are never confused. This matches Blender and most CAD and map tools. Bevy and glTF are Y-up, so the engine converts once when importing models and when rendering; mods only ever see Z-up.

**Floating origin.** Precision must not get worse the farther a player travels; Minecraft's far lands and jittery movement at large coordinates are the failure to avoid. Built from day one, because position types touch every system.

- **Positions are never a single large float.** A position is an integer cell (chunk and boxel) plus a small floating-point offset inside it. Simulation, saves, networking and the mod API all use this form, so precision is the same everywhere in the world.
- **Rendering is relative to a moving origin.** Each world view keeps its own origin near its camera and recenters when the camera moves far enough away. Everything is drawn relative to that origin, so the GPU only ever sees small numbers.
- **One origin per world view, not one per game.** Portals, security cameras and split-screen each have their own origin. The server has no origin at all, since it never renders.
- **Sub-grids use their own local space,** so a ship's interior keeps full precision wherever the ship is.
- **Mods never see the render origin.** They work in cell-plus-offset positions, and the engine handles the conversion.

Sub-grids and world views may be one system: worlds are grids, and the client can view any grid from any camera. A pocket dimension is a sub-grid not placed in the main world; a portal is a camera looking into one.

## Rendering

- **Runs on integrated GPUs.** Advanced GPU features (indirect-count drawing, mesh shaders, hardware ray tracing) are optional fast paths, always with a fallback.
- **Colored light from the start.** Stored light holds red, green, blue and sky channels. Moving lights, such as held torches and lights on sub-grids, are drawn as dynamic lights rather than written into the stored grid.
- **Rasterizer first.** A traditional triangle renderer comes first. Ray marching through boxel data comes later, for shadows, ambient occlusion and lighting effects.
- **Baked ambient occlusion.** Inside corners darken per vertex, baked by the mesher from how much of the space in front of each corner block shapes fill, measured in sixteenths so partial shapes count by volume. Free each frame and cheap on integrated GPUs, unlike screen-space AO (2026-11-02). On a curve, so creases go dark while slight occlusion stays light. Over it, the shader draws a crisp line a few texels wide along each inside crease (darker) and outside ridge (lighter), from what the mesher records past each quad's edges, texel by texel (2026-11-05). Real occlusion at texel scale waits for ray marching through boxel data.
- **Light and air in the shader.** Faces are lit by a sun direction and the sky's light, not baked brightness, so the sun moves without remeshing; looks can have a sheen (Blinn-Phong) as data; the sky is a gradient with a sun, and fog fades the far world to the sky's color in that direction, hiding the render distance's edge (2026-11-05). The game renders natively through wgpu (Vulkan, DirectX 12 or Metal); there is no browser build.
- **32×32×32 chunks.**
- **Culled meshing by default.** Each visible face becomes its own quad. Identical neighboring faces are merged (greedy meshing) only when texture, lighting and shading match; blocks that need per-face variation opt out.
- **One grid, transform and view system.** The world, sub-grids, pocket dimensions, portals, cameras and mirrors all go through the same code path.
- **LOD generated by the server.** Far terrain is built on the server and streamed to players. Each block's far appearance is derived automatically from its model, so mod blocks work with LOD without extra code.
- **Stable render graph for mods.** Sandboxed mods add materials and render passes through a stable interface. Replacing core parts of the renderer is done by improving the open-source engine itself.
- **Procedural, solid looks.** Blocks have no painted textures. Each material's look is a small data recipe (noise, bricks, planks, rings, a top cover) baked at load into a solid volume of 16×16×16 texels that fills its boxel, and every face shows the texels just inside it. Any cut through a block, including shapes nobody drew textures for, shows the right part of its material, so no outside art is needed. A material decides whether its look turns with its blocks (a log's rings do; planks and bricks stay level). Replaced the earlier plan of borrowing Luanti's textures (2026-10-31). Creature models are a separate matter: the cow is borrowed CC0 art from Luanti's `mobs_animal`, credited in the mod's `CREDITS.md` (2026-10-31). Since 2026-11-03 it keeps its skeleton and clips, converted to glTF; models reach the engine as plain data either way.

## Multiplayer

Recommendations from the multiplayer research. Rates and budgets are starting points to tune in playtests.

**Architecture**

- **Server-authoritative.** The server decides everything. Single-player runs a local server inside the game, so there is only one code path. *Built (2026-11-05):* the `server` crate owns the simulation and the client keeps a copy of the world from `protocol` messages alone; a lint keeps the client from reaching the simulation. Effects that would stop the world for everyone, like hit-stop, live in the client's view.
- **Ticks and updates.** The simulation runs at 60 ticks per second; entity updates go out 20 to 30 times per second, most important first (nearby, visible, fast-moving), within a per-player bandwidth budget. Other players and creatures are shown slightly in the past and smoothly interpolated.
- **Separate channels.** Chunks and block edits use reliable, ordered streams per region, so a big chunk never delays combat. Entity movement uses fast, unreliable messages.
- **Transport.** QUIC (the `quinn` crate). For player-hosted servers, `iroh` adds connections by public key, NAT hole punching and community-run relays, so no port forwarding is needed.
- **One address for everything.** Storage, networking, subscriptions and permissions all use (world, grid, chunk, layer).

**World data**

- **One chunk format on disk and network.** Each layer (blocks, fluids, overlays) gets its own palette; uniform chunks cost almost nothing; batches are compressed with zstd using a dictionary trained on real chunks.
- **Rate-limited chunk sending** per player, so exploring doesn't choke the server's upload. Home upload speed is the first limit player-hosted servers hit.
- **Anti-xray by design.** Hidden interiors are sent as "unknown solid" until a face is exposed; far terrain (LOD) never reveals ores.

**Hard features**

- **Combat.** The local player's movement, attacks and projectiles are predicted instantly. The server checks hits by rewinding targets to what the shooter saw, capped at about 150 to 200 ms. Creatures telegraph attacks with wind-ups longer than typical lag. Hit feedback plays immediately on the predicted hit.
- **Sub-grids.** A grid's position and rotation are synced like an entity. Players riding a grid are synced in the grid's local space, which removes the classic sliding and rubber-banding. The server is the authority, optionally handing control to the driving player.
- **World views.** Each portal or camera adds its own smaller, lower-priority subscription, with a recursion limit and automatic downgrading when performance drops.
- **Deterministic automata.** Liquids and logic run on clients using integer-only rules. The server sends periodic per-chunk checksums; a mismatch resends only that chunk. Anything that grants items stays server-only.

**Mods and identity**

- **The server owns numeric IDs.** It sends a name-to-ID table per registry; clients remap. Saves store names, not raw numbers.
- **Mods pushed by content hash.** Missing mods are fetched from any source and verified by hash.
- **Synced mod data is declared by schema,** so the engine handles sending it; each mod gets a bandwidth quota. Mod code that must match across machines runs in a deterministic WebAssembly configuration.
- **Public-key identity.** Players and servers are keypairs; join links contain the server's key, so one click connects.

**Scaling**

- **Region-parallel simulation** (like Paper's Folia) is part of the mod API from the start: mods only touch their region's data and message other regions.
- **Per-region time dilation** when a region falls behind, instead of the whole server lagging.
- **Targets:** 2 to 16 players on a home PC; 50 to 150 on a dedicated multi-core server.

**Griefing**

- **In the core engine:** claims and permissions checked on every edit (including mod actions), a log of every block change, and rollback.
- **Configurable raid rules:** siege timers (a breach starts a countdown and alerts owners), upkeep-funded protection, damage reduced while owners are offline, or freezing offline bases. Friend servers might use claims only; PvP servers might use siege timers.

## Candidate features (not yet decided)

These came up in discussion but are not confirmed. Tick the ones to keep.

**Fixes for Minecraft engine limits**

- [ ] Micro-blocks or variable-resolution blocks for detail smaller than one boxel
- [ ] Cheap arbitrary data on any block, without a heavy block entity
- [ ] Fluids with simple, learnable rules rather than realistic simulation
- [ ] Arbitrary block orientations, slopes and rotated collision boxes. *Partly built (2026-10-28):* blocks can take any of the 24 grid-aligned orientations, with shapes made of boxes in sixteenths of a boxel and collision on those boxes. Slopes and free angles are still open.
- [ ] Persistent block damage, and structural support as a simple rule rather than a physics simulation

**Platform ideas from earlier research**

- [ ] Open source with federated mod distribution, or at least a switchable mod-download URL
- [ ] Close GPU access for mods, including general compute
- [ ] A shared core vocabulary (materials, tiers, damage types, item/fluid/energy transfer) so mods work together
- [ ] Design tokens so mods look and feel like they belong together
- [ ] Multithreaded simulation
- [ ] One-click modded multiplayer, with servers pushing mods to players
- [ ] Mods depend on shared concepts, not specific items, to avoid Luanti-style fragmentation
- [ ] Stable, versioned API with deprecation windows
- [ ] Replaceable render pipeline with clear stages mods can add to or swap

## Minecraft gripes to improve

| Gripe | What it means |
| --- | --- |
| Punishes boldness, rewards hiding | Staying cozy at the base is always safe. The only time you're punished is when you take risks, so the best strategy is to play it safe. |
| Combat feels floaty | Hits lack weight and feedback, so fights feel imprecise. |
| Value comes only from rarity | Making something rarer is nearly the only way the game makes it valuable. Top stats are gated behind long, unfun grinding, which also makes losing that gear extra punishing and pushes players back toward playing safe. |
| The cozy life is either instant or impossible | In Minecraft, a safe, comfortable life comes almost immediately, so there's little to earn. In games like RimWorld, difficulty grows forever, so it's never reached. A cozy life should be a real goal players can achieve and then enjoy. |
| Bases can't be protected | Dozens of players can build a mega base together, and one griefer can destroy it while no one is online. Hiding a base is unrealistic, and once its location is known there is nothing in the game to slow an attacker down. |
| Inventory discourages variety | Every kind of item costs a slot, so carrying many block types, shapes and colors is punished and players stick to a few materials. Each mod that adds blocks makes this worse. Possible directions: limit by weight or volume instead of slots; carry a material and choose its shape when placing; build directly from base storage within some range. Tried (2026-11-01): the player carries materials, not blocks, all sharing one capacity by volume; placing uses up the block's volume of its material, and a palette slot holds a material whose shape is picked with a key. |
| Ambiguous interactions | One button does too many things depending on what you hold and what you look at. Eating with torches in the other hand places torches everywhere; aiming a bow and drifting over a door opens it and lets an enemy in. Placement so far (2026-10-28): a ghost shows exactly what placing will do, red when refused, and one key switches between basic placement (Minecraft-like) and precise placement (the part of the face aimed at picks how the block sits, shown by a guide), rather than overloading crouch. Since 2026-10-31, whatever is held has a main use on left click and a second use on right click (Skyrim-style): a block breaks and places, a tool swings and guards. |
| No creativity in 2D crafting (tentative) | The crafting grid is fixed patterns to memorize or look up. Players have no room for choice or expression. Tried (2026-11-02, emergent shapes toy): shapes come from crafting, not from a list of blocks. Players stack materials and parts up to four layers high on a 4×4 table, nothing may float, and the result keeps the frame it was built in, so slabs, stairs, thin and corner stairs, buckets and posts all come from one rule. A first rule that pushed a pattern through the block made mostly useless shapes and unpredictable placement, and was replaced. Added (2026-11-06, functional crafting): several materials in one design, and building on the table's side (the grid is a wall, stacks grow toward you) as a second frame of the same rule; a part used in a layer keeps how it stands in the world. |
| No compositional complexity (tentative) | Things don't combine into new things with their own properties. An item is what it is, not a sum of its parts and materials. First try (2026-11-06, functional crafting): materials bring properties (trigger, hinge, listener) into any shape made of them, enough for a door and lever with no door code. The hinge's rules turned out arbitrary in play; redone with a checklist for rules in Material behavior. |

## Tentative design directions

These are working ideas, not commitments. They should be tested in the play phase.

**Threats are finite, and the world can be conquered.** Threats don't escalate forever. The base game has an ending: players push out, take on the world's challenges, and earn a cozy life that then stays safe. This resolves the tension between rewarding boldness and wanting a cozy home, because boldness is how you get there.

**Challenges are forced, not optional.** At least in the base game, threats come to the player, who has to move fast and overcome them. This is the opposite of Minecraft's direction, where challenges tend to be either background annoyances you put up with or optional ones you can avoid entirely (Mojang has reportedly said a creeper-like threat today might live in one distant region). Pressure pushes players to conquer the world, and conquering it is how they earn the cozy life.

**Abstract systems over realistic simulation (base game).** The base game prefers simple, learnable rules players can understand and exploit over complicated physics simulations. Abstract rules are cheaper to build and run, easier to sync in multiplayer, and predictable enough to support the composition principle. Minecraft's best systems already work this way: redstone and infinite water sources are abstractions players learned to exploit. This is a taste for the base game's content, not a limit on the engine: mods must be able to add advanced simulations, including full physics and realistic fluids.

**A robotic player (tentative).** Movement and camera feel should read as a robot. Landings work like a shock absorber: a safe fall compresses smoothly, while a damaging fall bottoms out hard, so the camera itself tells players when a fall hurt. (Built with fall damage, 2026-11-03: the hard landing starts where falls start to hurt.) Crouching mid-air tucks the feet up to clear higher ledges. (A charged crouch jump, where crouching on the ground wound up a spring for a higher jump, was tried and removed on 2026-11-02.) Later feel work, such as combat and damage, should keep to this.

## Features to play with

Ideas to try in the play phase. None are commitments.

- **Aggressive combat, Doom (2016) style.** Attacking up close is how the player regains health and resources, so pushing forward is safer than hanging back. First pass played (2026-10-31): melee hits with hit-stop and recoil, head hits for double damage, the player's speed carried into a swing, and wall slams. The aim is expressive combat with a high skill ceiling; tuning waits for enemies and tools. Second pass (2026-11-03, Fighting cow): cows fight back with a telegraphed head toss up close and a charge from far off, which stuns them if dodged into a wall; the herd chooses to help a hurt cow or run. The player has health that comes back after a while, a guard that softens blows, and fall damage; a hit in mid-air pushes the player back, so swinging down onto a cow breaks a fall. Killed cows stay dead until the player is knocked out, so a herd is beaten in one life. Third pass (2026-11-06, Cow pathfinding): cows find paths round walls and hop up block-high ledges, so standing on a block is no longer a free escape (with no way up they wait below); a charge still runs straight and can be baited off a cliff, and falls hurt creatures, so terrain is a weapon.
- **Long time horizon threats.** A looming event, such as winter coming, that players must prepare for, pushing them out to gather and build before it arrives.
- **Rigid-body sub-grids.** Rigid-body physics is simple enough to try. The main risk is that it feels janky, so players should walk in a sub-grid's own local space to keep riding moving things smooth.
- **Terrain that changes gameplay.** Different terrains affect how players move, build, fight and survive, so where you are matters, not just what blocks are there.
- **Liquid rule sets.** Liquids in Minecraft's sense: simple rules, not a simulation of real fluids. Try different rule sets, such as Minecraft's source blocks and infinite water, finite amounts that spread out, or levels that even out between connected pools, and see what each lets players build and exploit.

### Liquids: what the rules should allow

The goal is to find simple rule sets that give these behaviors. Some pull against each other, which is what the experiments are for.

**Movement**

- Flow.
- Horizontal flow without losing height, like a river.
- Intuitive filling of spaces.
- Intuitive draining of spaces.

**Player intuition**

- Connected pools settle to the same level, like water in a U-shaped tube.
- Clean waterfalls that fall straight down and pool at the bottom.
- Readable state: players can see how full a space is and which way liquid flows.
- No surprise creation or deletion. If infinite sources exist, that is a clear, known rule.
- Consistent behavior around partial blocks such as fences and slabs.

**Building and exploiting**

- Determinism: the same setup always gives the same result.
- Currents that carry items, creatures and players.
- Buckets, pumps and pipes that players can reason about.
- Clear rules when two liquids meet, like water and lava making stone.
- Works on sub-grids: ship hulls keep water out, and liquid on a moving ship stays put.

**Technical**

- Liquid settles and stops updating once stable.
- Oceans and lakes cost nothing until disturbed.
- A broken dam doesn't lag the server.

**Starting point: cellular automata.** Each boxel follows simple rules based on its neighbors, as Minecraft, Terraria and Noita do. Known hard cases and likely fixes:

- **Rivers that keep their height:** flat surfaces give liquid no reason to flow one way. Give each cell a flow direction, or give rivers sources and sinks.
- **Levels evening out over distance:** local rules move liquid one cell per step, so large bodies level slowly. Track connected bodies as a group and only run the automaton where things change.
- **Update order:** updating cells in a fixed order makes liquid drift one way. Compute each step from the previous state all at once, which also allows running in parallel on the GPU.
- **Speed:** one cell per tick is too slow for floods. Run several steps per tick, or move faster through open space.

Deterministic rules also help multiplayer: clients can run the same automaton, and the server only sends changes.

## Signature feature candidates

The two ideas most likely to give the game its identity. Both test the compositional complexity pillar directly, and both are cheap to test: no sub-grids, multiplayer, LOD or advanced rendering needed.

**Unstable crystals.** Two block types that collapse into a directional blast when they touch, and keep that behavior inside player-built devices (see the story below).

- Rules to test: two block types, a touch trigger, a blast in the facing direction, and blocks that behave the same inside a contraption.
- Question: do players invent devices on their own, or use the obvious trick once and stop?
- Success: testers build something nobody planned, such as a multi-shot or a trap.

**Generative crafting.** Items carry properties that can be applied to any boxel; patterns work on all block types; materials have natural attributes and a capacity limit.

- Rules to test: a handful of materials with natural properties, a handful of property ingredients, a capacity limit, and obvious things (sharp on a stick makes a sword) working the first time.
- Question: do players experiment with combinations, and can they predict the results?
- Success: testers can explain the rules back after a short session, and argue about their favorite combination.

**Combined test.** Once each works alone, let crystal blocks carry crafted properties (a crystal made heavy, a casing made reflective). If the two systems multiply into each other, that is the strongest proof of compositional complexity.

## Stories

Stories imagine what the design philosophy could feel like in play. They are not feature commitments: any mechanic in them can change or be cut. Each ends with what it is meant to show.

### The unstable crystals

Deep underground, a player finds two kinds of strange, radioactive-looking blocks, one blue and one green. By accident, they discover that when a blue block touches a green one, the pair collapses into a powerful blast of particles fired in the direction the green block faces.

The player starts carrying both and learns to use them as a weapon, rushing to place the pair in the right arrangement while under attack.

Later, a friend shows them a better way. By arranging the blocks in a crafting table with a trigger and a piston, one click pushes the blue block into the green one. The player now has a weapon they built themselves, not one the game gave them. They go on to exploit other block properties to add multiple shots, bursts and full-auto fire.

**What it shows:** simple rules combining into tools the designers never scripted; discovery through experimentation; boldness rewarded by learning to act fast in danger; crafting as a place for invention; and players passing knowledge to each other.

## Open questions and research needed

**Research not yet done**

- [x] Boxel rendering: meshing approaches, ray-marched or sparse-octree rendering, GPU-driven pipelines, LOD, and how Teardown, Vintage Story and Veloren do it
- [x] Multiplayer: server authority, client prediction, syncing large changing worlds and sub-grids, bandwidth, scaling to many players

**Decisions pending**

- Boxel scale. Proposal: keep roughly Minecraft's ratio (1-meter boxels, player about 1.6 to 1.8 boxels tall) and get fine detail from micro-blocks. The scale should be fixed in the core vocabulary so mods can rely on it.
- Signature feature: which one thing Minecraft can't do well should the vertical slice be built around?
- Which matters more: maximum modder freedom through patching, or performance plus one-click modded multiplayer?

**Hard problems to keep in mind**

- **Dominant strategies and runaway metas.** Composable systems can lead to a "nuke": one combination so strong it beats everything. In multiplayer this can turn into a dark-forest meta, where everyone hides and the only answer is a doomsday weapon of your own. Such metas are hard to break out of once they form. Possible tools: situational strength, built-in costs and threats that react to what players rely on. Mods are free to break balance; the base game should offer a balance design language so mods that want to fit in can.
- **Protecting bases from griefers.** Any answer has to work for large groups whose members aren't always online. Current plan: claims, change logs and rollback in the core engine, with configurable raid rules such as siege timers (see Multiplayer).
