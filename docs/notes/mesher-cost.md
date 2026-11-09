# Mesher cost (fix `fix-mesher-cost`)

Found 2026-11-08 while playing Experimental materials (batch A). Not a toy: a fault in code from earlier toys, fixed on its own branch off `master` per the workflow, then `master` merged into `toy-experimental-materials`.

## What the user saw

A 3D checker gadget (a gadget built from shrunk gadgets, so the checker is at texel scale) "lags the game too much". In detail: "the ghost is very lagy when I place it its mostly fine unless I interact with anything directly adjasent to it".

## Cause (read from the code, not yet measured)

Shapes are stored as lists of boxes (`BlockDef::shape`, from `ShapeGrid::to_boxes`). A texel checker can't merge: 2,048 one-texel boxes, 12,288 faces.

- **The mesher is quadratic per boxel.** `client::mesher::quad` works out each face's edges (`edges`, 8 scans: 4 edges, front and behind) and its corners' AO (`occlusion`, 4 scans) by walking every box in the 27 boxels around (`neighborhood`). A texel checker: 12,288 faces × about 12 scans × 2,048 or more boxes, about 300 million box checks per boxel, on the main thread.
- **The placement ghost** (`editing::update_ghost`) remeshes whenever the occupant it would place changes, its turn included, so it rebuilds as the player looks around. Its AO is off, but `edges` still scans: about 200 million checks per rebuild.
- **Edits next to it:** placing, breaking, a signal or a blast remeshes the chunk (`remesh_changed_chunks`, on the main thread); every face of every boxel beside the checker scans its 2,048 boxes too. Debris from it (`items::item_mesh`) and the hidden ghosts (`update_hidden_ghosts`) mesh the same way.
- **Smaller:** `editing::draw_outline` draws one gizmo cube per box, every frame (2,048 for a checker). The crystal touch test (`sim::blast::touch_directions`, through `BlockRegistry::origin`) rebuilds each piece's mask from its box list every time; fast enough, the same habit.

## The fix (done 2026-11-08)

Planned steps 1–5 were built, then a trace of the user's game (`--features trace`, see `crates/game/Cargo.toml`) and a harness loading their save (`time_edits_in_a_save` in `client::editing`) found more:

- **Texel grids:** past 64 boxes, a neighborhood is a 48³ grid of texel rows, filled straight from each occupant's turned shape; edges, coverage and AO (a summed-volume table) read it. Ordinary neighborhoods keep the box list. Equality tests mesh a busy chunk and every block alone both ways. *Debt:* edge and AO rules now live in two places; if that chafes, try the grid path alone after measuring ordinary terrain.
- **Coinciding faces** (pieces in one boxel) are kept by facing and plane; comparing all against all made a two-material checker 209 ms alone.
- **Covered faces left out:** a face whose front texel layer is full (a full neighbor, another piece, its own other boxes) isn't drawn: a two-material checker went from 24,576 quads to 1,536. This was first left out of the plan for changing quad counts; the trace showed the GPU copy of huge chunk meshes (50–110 ms) and it went in. Only full coverage culls; partly covered faces stay. See-through materials, if they come, must not cover.
- **Ghost:** meshes kept by occupant (48); a bundle's pieces are meshed together, as placed.
- **Outline:** shapes of more than 64 boxes are outlined by their bounding box.
- **Registry** keeps each block's `ShapeGrid`.
- **Pictures** (hotbar, crafting) are tessellated once per size, turn and scale and kept in egui memory: about 14 ms a frame in the trace.

Result on the user's save: an edit in a chunk of 20 two-material checkers remeshes in 41 ms (release), was 220+ ms plus the GPU copy. The user: "ok it works very well".

## What the fix can't change

A texel checker really has 12,288 visible faces, about 25,000 triangles; at about 340 bytes a face in our vertex format that's roughly 4 MB of mesh per block, and some milliseconds to remesh even with the fix. A few are fine; a house of hundreds isn't (gigabytes, hitches of seconds on edits). Merging faces does nothing for a checker. The real answer is the backlog's GPU boxel grid (ray-marching texels, cost by pixel rather than by detail).

## Decided and open

- **No detail budgets.** The user (2026-11-08): "rules can't feel arbitrary, like how we made the crafting table 2.5d the way the crafting table works feels like its a machine and it just sow happens to enforse some rule, some ugly thing that says no more then n faces per face feels worse. I get in a game like this players will find lag machines but it should be a bit harder. but im closor to scraping alot of the genrative building then adding arbitrary ruels".
- **Open, the user undecided ("I dont know"):** a Phase 2 decision that *the cost of anything the engine does with detail depends on how many boxels are involved, never on what's in them*: bitmasks inside a boxel, event-driven work across boxels (with the existing caps on signals, phase groups and beams only reached by deliberately huge builds), and the GPU grid for drawing. The user confirmed "continuity" meant connection checks (support, phase groups, touching, copper reach); those run on the CPU, so ray marching doesn't help them, but bitmasks keep them bounded per boxel. Batch C's "falls" support check should be designed this way.
