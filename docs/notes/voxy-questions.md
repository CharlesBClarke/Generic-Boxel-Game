# Voxy questions (clean-room brief)

*Written 2026-11-14, during Far terrain by cube's Verify, after one-boxel walls vanished too early in far terrain. The user is handing this to a model that reads Voxy's source; the answers come back here as a description, and our implementation is written from the description alone.*

---

## Prompt

You are reading the source code of **Voxy**, a level-of-detail (LOD) renderer for Minecraft. Your job is to explain *how it works* so that another team can design their own system from your explanation without ever seeing the code. This is a clean-room write-up:

- **Describe, don't copy.** Explain algorithms, data layouts, rules and thresholds in plain prose, lists, small tables or math. Do not paste source code, and do not write code that mirrors its structure line for line. Short identifiers (class, method and field names) are fine as references.
- **Cite where you found it.** For each answer, name the files and classes it comes from, so follow-up questions can point at them.
- **Give the real numbers.** Where there's a constant (a size, a threshold, a distance, a weight, a bit layout), give its value and what it means.
- **Say what you didn't find.** If something isn't in the code, or you're inferring it, say so plainly ("not found", "inferred from X"). A guess presented as fact is worse than a gap.
- **Stay on the questions.** Skip Minecraft-specific plumbing (mixins, mod loader, world loading, config screens) unless a question needs it.

**Context about the team asking**, so you can point out where Voxy's approach would or wouldn't carry over:

- Their game is a boxel (voxel) game with 1 m boxels in 32×32×32 chunks, Z up, terrain that goes on forever, and players building and digging anywhere.
- Full detail is drawn out to about 256–384 m. Past that, out to about 2 km, they draw **far terrain** in levels: a level-*k* cell is 2^*k* boxels a side (levels 1 to 6), each level chunk still 32³ cells.
- Far terrain is built on the **server** from the world **generator**, sampled coarsely (one sample per coarse cell), not from full-detail chunks: the full-detail world out there was never loaded. Edited chunks (the player's builds) are stored, and are shrunk into the coarse levels when far terrain is built.
- A coarse cell is drawn as one block of one main material, now with a second material and a share mixed in. It's solid if more than half filled. A build keeps a cell solid if what was built fills a boxel-thick plane right across it.
- **The problem that prompted this:** thin builds, such as a one-boxel-thick wall a few meters tall, disappear from far terrain too early. The plane rule needs a wall as tall as the cell (4 m cells from about 400 m, 8 m from about 650 m, 16 m from about 1.3 km), lined up with the cell's edges.
- Other open problems: popping when levels switch, cells' colors not matching full detail, and the next big job, picking levels per cube (an octree by 3D distance) instead of per column.

Answer the questions below in order, under the same numbers. Where Voxy has no answer to a question, say so and move on.

---

## Questions

### A. Downsampling: how a coarse cell is made from finer ones

1. How many LOD levels are there, and how big is a cell (in blocks) at each? Is each level exactly 2× the one below?
2. How is a level-*k* cell built: from the 8 cells of level *k*−1 (a mip pyramid), straight from full-resolution blocks, or some other way? Describe the rule that picks what the coarse cell becomes.
3. When the 8 finer cells disagree (some air, some solid; different blocks), which wins? Is there a priority (e.g. solid over air, opaque over transparent, some block types over others)? Give the exact rule and any tie-breaking.
4. **Thin structures.** What happens to a one-block-thick wall, a fence, a pillar, a floor or a single block at each level? Does it survive, get fattened, or vanish, and at which level? Is there any code that deliberately keeps thin or "important" things (anything not natural terrain) visible at coarse levels?
5. Does the rule treat the top surface (what you see from above) differently from what's buried? Is anything done so that a coarse cell shows the material on its surface rather than what fills its volume?
6. Does a coarse cell keep any record of what's inside it beyond one block, such as an averaged color, a mix of blocks, how full it is, or per-face data? If so, describe the layout (bits, fields).

### B. Colors and appearance

7. How is a coarse cell's color decided? Is it the chosen block's texture, an average of the textures, an average color per face, or something else?
8. Are colors or textures averaged per face direction (top vs side)? How are biome tints (grass, leaves, water color) handled at distance?
9. Is lighting (sky light, block light) stored with LOD data and used when drawing? How is it combined from finer cells?
10. Is anything done against shimmering, moiré or aliasing at distance (texture mip choices, dithering, fading)?

### C. Choosing which level to draw where

11. How does Voxy decide which level to draw at a given place: distance thresholds, size on screen, an error metric? Give the formula and its constants.
12. Is the choice made per section in 3D (an octree, by 3D distance) or per column in 2D? How does that cope with mountains, deep caves and overhangs?
13. How are transitions between levels handled: hard switch, cross-fade, dithered fade, geometry morphing, or nothing? How are cracks or overlaps where two levels meet avoided?
14. How does it avoid drawing full-detail terrain and LOD terrain in the same place, and avoid holes while finer data is still loading?

### D. Storage and data structures

15. What's the unit of LOD storage (a "section"?), how big is it, and how are sections addressed (level plus position)? Describe the in-memory and on-disk layout of one section: per-cell bits, palettes, compression.
16. Is LOD data kept on disk? When is it written, and what triggers rebuilding it? How are player edits (placing and breaking blocks) carried up through every level, and how quickly?
17. How are sections that are all air, or solid all through and buried, handled? Skipped, stored compactly, or culled?

### E. Building the geometry and drawing

18. How is a section turned into geometry: per-face quads, greedy merging, something else? What is in a vertex (bit layout)?
19. Is drawing done on the GPU (compute shaders, indirect draws, GPU culling)? Describe the pipeline: what's uploaded, what's culled where, how draw calls are issued.
20. How are faces between neighboring sections handled: are hidden faces at section borders removed, and does it look across into neighboring sections or other levels to do so?
21. Is there occlusion culling (hidden mountains behind mountains)? How does it work?
22. Roughly how many triangles or quads does it draw at typical view distances, and what are its main costs? Any measurements or comments in the code?

### F. Where LOD data comes from

23. Voxy builds LOD from chunks the game has loaded (or from saved world files). What happens where no chunk has ever been loaded: is it blank, or does it generate anything? Is there any support for building LOD from a world generator directly?
24. How long does ingesting one chunk take, and does the code mention throughput (chunks per second)?

### G. Wrap-up

25. In your own judgment, what are the two or three ideas in Voxy that make distant terrain look most like full detail? Which of them carry over to a game that builds far terrain from its generator rather than from loaded chunks?
26. What would you check first in the team's thin-wall problem, based on how Voxy treats thin structures (question 4)?

---

## Answers

*(Pasted here when they come back. Our implementation is designed from these answers, not from Voxy's code.)*
