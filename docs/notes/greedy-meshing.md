# Greedy meshing

*Both steps done: Greedy meshing (2026-11-10) and AO in the shader (2026-11-11, 273,263 → 185,080 quads on the 224 m terrain, at the ceiling of merging with AO off). Kept for the reasoning; see `docs/done-toys.md`.*

*Notes for the backlog's Greedy meshing toy. From a merge count on 2026-11-10, after the Bench toy, when the user asked "LOD or greedy mesher" for the frame rate. Greedy came first: the bench found frame time follows the triangle count (`docs/notes/shadow-speed.md`), greedy cuts triangles in every pass (main, each shadow map, any prepass) near and far, and it stays inside `client::mesher`; LOD is infrastructure (after streaming, built by the server per the design doc) and only helps far terrain. Asked whether it's still worth it without shadows: yes, the main pass is the same ~1M triangles, just less to gain.*

## The count

A throwaway test (not in the repo) generated the bench's terrain (the `[base]` of `tools/bench/bench.txt`: seed 120, width 11, height 6), meshed every terrain chunk with the real mesher and default AO, and merged the quads per chunk, per plane, as rectangles. A face merged only if it was a whole boxel face, its four corners matched each other (and the neighbor's) in each attribute the rule kept, and the edge between them was `FLAT` on both sides. The result matched the bench's own lines: 0.97M triangles, 363 chunks with faces, so the bench's spots see the whole terrain.

| Merge rule | Triangles | Saved |
|---|---|---|
| Today | 0.97M (483,190 quads) | |
| Plain greedy: light and AO must match | 0.28M | 71% |
| AO moved to the shader, light must match | 0.15M | 84% |
| Light moved to the shader, AO must match | 0.28M | 71% |
| Both moved | 0.15M | 84% |
| Both moved, creases ignored (upper bound) | 0.15M | 84% |

- **AO is the only thing that blocks merges.** Light changes nothing on open terrain (sky-lit evenly); creases change nothing (flat runs have flat edges). Plain greedy is worth doing first; AO in the shader is an optional second step for 13 more points.
- Every terrain face is a whole boxel face (0 partial). Builds of slabs, stairs and cuts will merge far less; not this toy's problem.
- The rule was cautious (a face's four corners must match). A rule that only needs matching values along shared edges, with interpolation kept linear, might merge a little more.
- *Rough guess, not measured:* ~0.7M fewer triangles at the bench's 2.5–4.5 ms per 1M-triangle pass is 2–3 ms off each pass; a shipped frame of ~17 ms might reach 8–11 ms. The bench decides.

## Questions for the plan

- **`MeshData::edges`** holds 2 bits a texel in a `u32` per edge: 16 texels, one boxel. `MeshData::face` divides by `FACE_SCALE`, also one boxel. A merged quad needs a new format for both (longer edges, or merged quads limited to plain edges, or the edges moved to a lookup).
- **Textures** are sampled from the look's volume by `solid` (texel position in the look). A merged quad must repeat the look per boxel in the shader (wrap the position), not stretch it.
- **T-junctions:** a big quad meeting several small ones along an edge can leave speckle gaps. *Seen in play and fixed by `fix-greedy-sparkles` (2026-11-11): `greedy::close_t_junctions` turns such a quad into a fan with its edges split at the other corners.*
- **`DrawnFaces`** and anything else that maps a quad back to one boxel: check what depends on one quad per boxel face.
- **Light's diagonal split** (`bake_light` picks the diagonal per quad by lighting): merged quads are uniform, so the split stops mattering for them.
- **Remesh cost:** an edit already remeshes the whole chunk; the merge is one extra sweep over its faces, small next to the edge, AO and light work. Fewer vertices make the GPU copy smaller. Measure an edit's remesh time before and after (`time_edits_in_a_save`, `time_meshing_the_terrain` in `client::editing`).
- **Measure:** the bench's `--quick master work` gives triangles and ms per spot.
