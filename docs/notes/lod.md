# LOD

*Notes for the LOD infrastructure entry in `SLICES.md`. Written 2026-11-13, during Loading by cube's Verify, from a talk about what's left for LOD.*

## What LOD has now (Far terrain, Loading by cube)

- Seven levels, each doubling the cell: level *k* cells are 2^*k* boxels a side, a level chunk still 32³ cells. At the defaults (256 m full detail, 2 km far, `level_reach` 1.6): level 0 to 256 m, 1 to ~410 m, 2 to ~655 m, 3 to ~1,050 m, 4 to ~1,680 m, 5 to 2 km, 6 only at the edge. Switch lines are jagged (the quadtree splits a whole column, judged by its middle).
- A coarse cell is one block: solid or empty, the material filling most of it, drawn as that material's look averaged to one pixel per facing way, mixed by slope (`far_shade`, `client/src/solid.wgsl`). Minority materials are dropped.
- Builds are shrunk into every level (`sim::far::shrink_edits`): since 2026-11-14 (Far terrain by cube) any built boxel holds its cell (`far_hold` 2), so builds never vanish, they fatten to the cell's size.

## What the user has seen

At Far terrain's Verify: "the color is the biggest issue, the distant hills look just green. and they instantly pop with the color with that Moiré Aliasing probplem. and the shape seems worse then minecraft voxy mod. so I get the sense there are better ways to combine cubes". On 2026-11-13 the user asked "I thought our LOD system didnt take into acount player build strucutures?" (it does, with the limits above).

## Highest value additions, ranked (2026-11-13)

*1–3 done in Far terrain by cube (2026-11-14); 4 and 5 are next.*

Ranked by what the user notices in play. A suggestion for the planner, not agreed: the next session decides with the user.

1. **Average colors per cell** (a "baked superblock"; the user asked "do we bake a superblock with a partial texture?"). Each coarse cell carries the average color of everything in it, per facing way, instead of the majority material's one. Hills keep their rock, dirt and grass mix, thin builds stay as a tint, and much of the moiré and popping should go, since cells stop flipping between materials across levels. Cheap: a few bytes a cell, built where cells are built now.
2. **Levels picked by size on screen.** Cells stay about the same pixel width everywhere, so a switch changes little on screen. Replaces `level_reach` with a pixel target as the quality slider. The real fix for popping.
3. **Builds stay visible.** A mostly empty cell holding an edit stays solid (or the "how full" score holds it at finer detail), and edits shrink into every level.
4. **Cubes instead of columns** (the octree by 3D distance). The biggest job and the least visible on today's hills; needed for caves, overhangs, deep pits and high terrain far away, and pays off the far terrain stand-in in the API debt list.
5. **The "how full" score** (the user: "count how full a level and use that to bias when it drobs"). Tuning on top of 2 and 3.

Probably not worth it yet: fading between levels (2 may make it unneeded), the far shadow past the render distance, compressing far messages, merging far faces of different slopes, ores and anti-xray for far data.

Suggested split: Far terrain by cube becomes 1 + 2 + 3, with 4 as its own toy after. This turns the current order around (the octree was planned first).


Clean-room questions on how Voxy does LOD (thin builds, colors, level picking, storage), with answers once they're back: `docs/notes/voxy-questions.md` (2026-11-14).
