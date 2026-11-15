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

*Pasted 2026-11-14 from the clean-room answers file `2026-11-14-voxy-answers.md` (SHA-256 recorded in `voxy-answers-receipt.txt`, commit eac9f91). Our implementation is designed from these answers, not from Voxy's code.*

Answers to `boxel_game/docs/notes/voxy-questions.md` (SHA-256 `3f00b998dbe4d955d0f564bcddd256aa99b396b22d151e799439175a567aa098`).
Source read: Voxy 0.2.20-beta, commit `5bf33350a1cc7bb7e2b5d947b236c56d878b8a7d`.
Descriptions only; no Voxy code is reproduced here. Voxy is Y-up — "up" below means Y.
File paths are relative to `src/main/java/me/cortex/voxy/` (shaders: `src/main/resources/assets/voxy/shaders/`).

---

### A. Downsampling

**1. Levels and cell sizes.** 5 levels, 0–4. A level-k cell is 2^k blocks: 1, 2, 4, 8, 16. Each level is exactly 2× the one below. A section is always 32³ cells, so a level-k section spans 32·2^k blocks (32 … 512).
*Source: `common/world/WorldEngine` (MAX_LOD_LAYER = 4), `common/world/WorldSection`.*

**2. How a coarse cell is built.** Mip pyramid: each level comes from the 8 cells of the level below. It is done inside one 16³ game chunk section at a time (16³ → 8³ → 4³ → 2³ → 1), then each level is written into its 32³ section. One chunk section fills a (16/2^k)³ patch of a level-k section; at level 4 it is one cell. An update recomputes the whole 16³ pyramid.
*Source: `common/voxelization/VoxelizedSection`, `WorldVoxilizedSectionMipper`, `common/world/WorldUpdater`.*

**3. Which child wins.**
- If any of the 8 is non-air, the result is non-air. Air only if all 8 are air.
- Among non-air children, highest opacity wins (the block's light-blocking value, 0–15; leaves forced to 15).
- Ties: by corner, score 4·x + 2·y + z, highest wins — so +X first, then upper (+Y), then +Z.
- The winner's whole value (block, biome, light) is copied.
*Source: `common/world/other/Mipper`, `common/world/other/Mapper` (StateEntry).*

**4. Thin structures.** Nothing vanishes. One block anywhere in a 2^k cube makes the whole coarse cell that block, so walls, pillars, floors and single blocks survive at every level, fattened up to 16 m at level 4. Non-full blocks (fences, torches) survive if nothing more opaque shares the cell, and are drawn as their own shape scaled up. There is no special code for "important" or built things — survival comes only from the any-non-air rule.
Quirk: a block with no visible model can still win the mip but is drawn as nothing.

**5. Top surface vs buried.** Not treated differently. The only top bias is the Y tie-break, which ranks below X, so when a cell straddles the surface a buried material (dirt/stone) can win over the surface one (grass).

**6. Extra data per cell.** None — no coverage, count, averaged colour or per-face data. One 64-bit value per cell:

| bits | field |
|---|---|
| 0–26 | unused |
| 27–46 | block id (0 = air) |
| 47–55 | biome id |
| 56–63 | light (high 4 bits block light, low 4 bits sky light) |

Per section: an 8-bit "which children are non-empty" mask.

### B. Colours and appearance

**7. Colour.** The chosen block's real per-face texture (each block pre-rendered to 16×16 per face), repeated once per coarse cell and sampled with mipmaps. No averaging across children. Because a level is switched when a cell is about 2 px on screen, what you see is the texture's mip average — matching how full detail looks far away.
*Source: `client/core/model/ModelFactory`, `MipGen`, shader `lod/gl46/quads.frag`.*

**8. Per face, biome tint.** Each face has its own texture (grass top ≠ side). Fixed per-face shading for up / down / north-south / east-west, same values as the game. Biome tint is a table of one colour per (block × biome), computed once; applied to the whole face or only to grey pixels. No blending between neighbouring biomes. Biome comes from the game's 4×4×4-block biome grid.
*Source: `ModelFactory`, shader `lod/quad_util.glsl`, `client/core/rendering/section/backend/AbstractSectionRenderer`.*

**9. Lighting.** Stored per cell and used.
- Combining: if a solid child wins, keep its light. If all 8 are air: block light = average rounded down, sky light = average rounded up.
- Drawing: an opaque face uses the light of the cell in front of it; inset or translucent faces use their own. Raised to the block's own glow. Looked up in the game's day/night light table.
*Source: `Mipper`, `client/core/rendering/building/RenderDataFactory`, shader `lod/lighting.glsl`.*

**10. Shimmer / aliasing.** Full mip chain per face texture, with transparent pixels pre-filled with the nearest opaque colour before mipping. Mip chosen from the whole merged face. Cut-out test uses the sharpest mip so cut-outs don't erode. Fade over the last 10% of the render distance, plus fog and ambient occlusion. No dithering, no level cross-fade.

### C. Choosing which level to draw

**11. Rule.** Size on screen. A section's cube is projected; it is split into its 8 children while its screen area is more than 64 × 64 pixels (setting `subDivisionSize = 64`). That is about 2 px per cell. Outer limit: 16 top-level sections = 8,192 blocks, a circle on the ground plane.
*Source: shaders `lod/hierarchical/screenspace.glsl`, `traversal_dev.comp`; `client/core/rendering/hierachical/HierarchicalOcclusionTraverser`; `client/config/VoxyConfig`.*

**12. 3D or 2D.** 3D octree. Roots are 512³ sections loaded in a ring of columns around the player (all heights); the descent is fully 3D, so nearby cliffs and mountains get finer cells automatically. Caves are mostly hidden by occlusion culling (21). Empty subtrees are skipped.

**13. Transitions.** Hard switch — no fade, no morphing. A section's children replace it only when all of its non-empty children have finished meshes, so the swap is all-at-once. No explicit crack handling; coarse cells contain all of the finer solid volume, so seams are normally covered (inferred, not guaranteed by code).
*Source: `NodeManager`, `NodeChildRequest`.*

**14. No double drawing, no holes.**
- LOD only draws on pixels where full detail drew nothing (sky). Also, the boxes of loaded full-detail chunk sections are drawn into a depth image, and LOD pixels in front of them are discarded.
- A parent keeps drawing until its children are ready. If it has no mesh yet, its children draw instead. Mesh requests are throttled (target ~4,000 queued jobs).
*Source: `client/core/AbstractRenderPipeline`, shader `chunkoutline/outline.vsh`, `traversal_dev.comp`.*

### D. Storage

**15. Section layout.**
- Unit: 32³ cells at one level; in memory 32,768 × 64-bit values.
- Address: one 64-bit key = level (4 bits) + y (8 bits) + z (24 bits) + x (24 bits).
- On disk: key, small header (palette size, child mask), 32,768 × 16-bit palette indices, then the palette of distinct cell values (max 65,535).
- Default store: RocksDB, ZSTD level 1.
*Source: `common/world/SaveLoadSystem3`, `common/StorageConfigUtil`.*

**16. Persistence and edits.** Stored on disk; a changed section is saved when it leaves the in-memory cache (1,024 or 2,048 sections) or at shutdown. Updates come from chunks loading and unloading on the client, from full-detail sections rebuilding, and from blocks broken on chunk-section borders — so edits mostly arrive when the chunk unloads (LOD is never shown over loaded chunks anyway). An update re-mips the whole 16³ section and writes levels 0→4 immediately, stopping at the first level that didn't change, then re-meshes the changed sections and touched neighbours.
*Source: `WorldUpdater`, `common/world/service/VoxelIngestService`, `SectionSavingService`.*

**17. All-air / all-solid sections.** All-air subtrees are skipped via the child masks; empty meshes cost nothing. Solid buried sections produce no faces, so they cost nothing to draw. Both are still stored (they compress to almost nothing).

### E. Geometry and drawing

**18. Geometry.** Greedy merging of faces per slice; faces merge only if block, biome and light all match, up to 16×16 cells. Separate lists: translucent, two-sided, and one per face direction. Fluids meshed separately. No vertices are stored — each face is 8 bytes:

| bits | field |
|---|---|
| 0–2 | face direction |
| 3–6, 7–10 | width−1, height−1 |
| 11–25 | position in section (5 bits per axis) |
| 26–41 | block model id |
| 46–54 | biome |
| 55–62 | light |

Faces use the block model's real outline and inset, scaled by 2^k.
*Source: `RenderDataFactory`, `client/core/util/ScanMesher2D`, shaders `lod/quad_format.glsl`, `lod/gl46/quads3.vert`.*

**19. GPU pipeline.** Each frame:
1. Draw last frame's visible sections.
2. Build a depth pyramid from that.
3. Walk the octree in compute shaders (one pass per level): frustum test, depth-pyramid test, size test → draw list + mesh requests sent back to the CPU.
4. Test each listed section's box against depth.
5. Compute shader writes up to 7 indirect draws per section, skipping face directions that face away from the camera.
6. One indirect multi-draw.
7. Second pass for sections that just became visible.
8. Translucents sorted into 1,024 distance buckets.

Meshes are built on CPU worker threads.
*Source: `AbstractRenderPipeline`, `client/core/rendering/section/backend/mdic/MDICSectionRenderer`, shaders `lod/gl46/cmdgen.comp`, `lod/gl46/cull/raster.vert`.*

**20. Faces between sections.** The mesher reads the touching slice of each neighbouring section at the same level and drops border faces hidden by an opaque neighbour. It never looks at other levels. Edits re-mesh the touched neighbours.

**21. Occlusion culling.** Yes — the depth-pyramid test on the octree, the box test per section, and the second pass for newly visible sections (see 19). Pixels covered by full detail count as fully blocked.

**22. Counts and costs.** No measurements in the code. Limits: 65,536 faces per section per list; 400k opaque / 100k translucent / 100k second-pass draws; geometry evicted at 75% buffer use or under 256 MB free.

### F. Where LOD data comes from

**23. Never-loaded areas.** Blank. Voxy does not generate terrain. Sources: client chunks, saved world files, a Distant Horizons database importer, and chunks produced by the Chunky pre-generation mod. No world-generator path; fetching LOD from a server is only a TODO note.
*Source: `commonImpl/importers/WorldImporter`, `DHImporter`, `common/config/section/SectionSerializationStorage`.*

**24. Ingest speed.** No timings or throughput figures in the code. Uses cores ÷ 1.5 worker threads.

### G. Wrap-up (judgment)

**25. Ideas that matter most.**
1. Solid-biased downsampling ranked by opacity — shapes and builds never vanish, they get fatter. Carries over (generator samples and edited chunks alike).
2. Coarse cells drawn with the block's real per-face texture, mipmapped, at ~2 px per cell, with the game's face shading, per-biome tint and light from the neighbouring air cell — distance colour matches full detail. Carries over fully.
3. 3D octree by screen size, with all-at-once parent→children swaps and masking against full detail. Carries over; this is the per-cube level picking planned next.

**26. Thin-wall problem — check first.** The current rules are majority-based (more than half full, or a full plane across the cell). Voxy uses "any non-air child wins, ranked by opacity", so a 1 m wall can't vanish and alignment doesn't matter. Check, in order:
- (a) When shrinking edited chunks, let built boxels win with an OR rule (at least player-placed material).
- (b) Build each level from the level below, so a cell kept at level 1 stays kept upward — don't re-test against raw boxels at every level.
- (c) Accept the fattening (16 m thick at ~1.3 km); at ~2 px per cell it reads as a line.

A possible split (not something Voxy does): keep the majority rule for generator terrain to avoid inflating hills, and use OR only for edited chunks.

