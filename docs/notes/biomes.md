# Biomes: continents, sub biomes and motifs

Notes from the design talk of 2026-11-20. The design doc's "A world of continents" holds what's agreed; this file holds the open questions and the technical side, for the planners of the 3D terrain toy and the super biome toy.

## What the user asked for (quoted)

- "terrain that couldn't exist on earth."
- "I wanted a system where there where super bioims, and sub bioms. the supper bioms would add motifs that coulod apply to every biome. think of dissney land every  secton has a resterant, and a resterant has cerntin things, but the resterants in the diffrent sectons inherent the languege of the section."
- "I wanted some to be very 3d like a very alien terrain that where the terrain like a Trypophobia terrain where the land is fored into boney conttections and more then 50% air."
- "I want theses supper biomes to be like cotenents, so it sould support most bimoes so bone reef is good, but frost hills is kind of wrong, a contenent should have warm and cold regions."
- On "motif" meaning the super biome's style language: "motif is good".

Terms (super biome, sub biome, motif) are in the design doc's Terminology table.

## The order agreed (2026-11-20)

1. Far terrain looks paused after pass 1: LOD tuned for today's hills would be tuned for the wrong world.
2. This design session.
3. **3D terrain toy**: a density generator, today's hills rebuilt on it, one Bone Reef patch to measure speed and see how far terrain copes.
4. **Super biome toy**: the layering and the borders. "this can wait".
5. Back to LOD, aimed at the real terrain.

## Why a height map can't do it

`base::terrain` (Generated terrain, 2026-11-05) works out one ground height per column, from Tectonic-style fields (inland, erosion, ridges). Nothing can be under anything else, so no overhangs, arches, struts or land that's mostly air. Tectonic itself runs on Minecraft's 3D density; we took its ideas into a height map to keep the first generator small. The engine side is already 3D (3D chunks, the far octree, `fill_coarse` taking 3D cells, light tested on overhangs); the generator is the limit.

## 3D density and the interpolation trick

- Density: a value at each point, solid where it's above 0. Today's fields still give the large shape (roughly "height minus z" plus 3D noise), and a motif adds its own shaping.
- Sampling 3D noise at every boxel is 32,768 samples a chunk. The usual trick: sample on a coarse grid (Minecraft uses 4×8×4) and blend between the samples, about 100× fewer samples.
- **The catch for struts:** blending at 4 boxels erases struts thinner than about that. Options: a sample spacing per motif; motif shapes cheap enough to work out at every boxel (a gyroid is a few sines; 3D cell noise is a handful of hashes); or a coarse grid for the slow fields plus a fine pass for the motif.
- Shapes for the porous look: 3D cell (Worley) noise, whose cell walls and edges become webs and struts; gyroids and other surfaces that repeat in 3D, naturally porous and connected; domain warp so neither reads as a grid. How much is air is the threshold.
- Measured 2026-11-20, today's height map in release: 3,072 chunks in 0.97 s wall, 2.47 s on the workers (~0.8 ms a chunk). Each chunk in a column works out the same 34×34 heights again, and `tops` repeats them; a cache per column is a cheap win whatever happens. Far build in Far terrain looks pass 1: 8.9 s wall, 6.5 s of it `mesh_far` on one thread, so meshing, not generation, was the larger far cost then.
- Determinism: noise must stay bit-for-bit (`libm`, see the design doc).

## What 3D terrain breaks or strains

- Things that ask "how high is the ground here": the bare-stone slope check, surface layers (grass, dirt), spawn and creature placement, waypoints, boulders. They need a search down through density, or a surface found per column.
- Tops stay correct (they're a bound, never too low) but say little in land that's mostly air: sky light below is worked out by flood, not known from the tops.
- The far shadow is a height map of tops: in a reef, everything under the top layer reads as shadowed.
- The mesher counts unloaded ground under the tops as solid (`FullBoxels`, `buried`): wrong nearly everywhere at the edge of what's loaded in porous land, so faces will be missing or extra there.
- Far terrain: struts thinner than a far cell vanish or swell into blobs depending on the "fills most of it" rule. Porous land may want its own coarse rule (keep connectivity, or a "how full" mix).
- Cows: pathfinding (`sim::path`) walks columns of ground; struts and arches need it to handle standing on things with air below and above.
- Chunk cost: porous land has far more faces per chunk than hills, so meshing and triangle counts go up.

## Open questions

- **Borders.** How continents meet (a sea, a blend zone, a hard seam?), and how sub biomes blend inside one. Blending density fields is easy; blending motifs that disagree (struts vs. solid hills) is the hard part.
- **What a motif can change.** Shapes and how much is air, palette (which blocks), small features (boulders become bone nubs?), maybe sky, light and sound later.
- **Climate.** Fields for warmth and wetness inside each continent; how big the regions are; whether height cools things.
- **Engine vs. base mod.** Is the biome layering an engine service (registries of super biomes, sub biomes and motifs that any mod adds to) or a framework inside the base mod's generator? The modding goal ("a mod's new sub biome is styled by every continent") points at an API registry. Which mod picks the continents?
- **How big a continent is**, and whether the player's spawn continent is chosen.
- **Gameplay.** The design doc's "terrain that changes gameplay": what porous land does to movement, building and fighting (falls, cows charging off struts, line of sight).
