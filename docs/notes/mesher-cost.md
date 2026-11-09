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

## The fix

1. **A failing benchmark first:** mesh a texel checker alone, and a chunk with a few of them beside other blocks; assert a time budget (and that quad counts and output are unchanged against the current mesher on ordinary shapes).
2. **Mesher on bitmasks:** build the 3×3×3 boxels around a boxel once as texel bits (48³ bits; `api::ShapeGrid` is the per-boxel form) and answer `edges` and `occlusion` from the bits: a fixed cost per face, whatever the box count. Perhaps a summed-volume table for AO. Also cull faces where a shape's own boxes touch each other (and boxes of other occupants in the boxel), since faces are culled only against full neighboring blocks today.
3. **Ghost cache:** keep built ghost meshes by occupant (block and orientation), so turning back and forth reuses them.
4. **Outline:** for shapes with many boxes, outline the shape's bounding box, or only the box aimed at.
5. **Masks cached per block kind** (the registry keeps each kind's `ShapeGrid`, turned per orientation as needed), used by the touch test, the phase grab and anything else that wants texels.

Nothing should look different. Check that every mesher test still passes unchanged.

## What the fix can't change

A texel checker really has 12,288 visible faces, about 25,000 triangles; at about 340 bytes a face in our vertex format that's roughly 4 MB of mesh per block, and some milliseconds to remesh even with the fix. A few are fine; a house of hundreds isn't (gigabytes, hitches of seconds on edits). Merging faces does nothing for a checker. The real answer is the backlog's GPU boxel grid (ray-marching texels, cost by pixel rather than by detail).

## Decided and open

- **No detail budgets.** The user (2026-11-08): "rules can't feel arbitrary, like how we made the crafting table 2.5d the way the crafting table works feels like its a machine and it just sow happens to enforse some rule, some ugly thing that says no more then n faces per face feels worse. I get in a game like this players will find lag machines but it should be a bit harder. but im closor to scraping alot of the genrative building then adding arbitrary ruels".
- **Open, the user undecided ("I dont know"):** a Phase 2 decision that *the cost of anything the engine does with detail depends on how many boxels are involved, never on what's in them*: bitmasks inside a boxel, event-driven work across boxels (with the existing caps on signals, phase groups and beams only reached by deliberately huge builds), and the GPU grid for drawing. The user confirmed "continuity" meant connection checks (support, phase groups, touching, copper reach); those run on the CPU, so ray marching doesn't help them, but bitmasks keep them bounded per boxel. Batch C's "falls" support check should be designed this way.
