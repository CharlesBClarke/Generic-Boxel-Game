# Shading

*Built 2026-11-05 as the Shading toy (see `docs/done-toys.md`); stored light and sun shadows are their own backlog entries now.*

Moved from the `SLICES.md` backlog on 2026-11-03; the backlog keeps a short pitch.

**Shading.** Added 2026-11-03 at the user's request: shapes look flat, and ambient occlusion is hard to see even at strength 1. Three reasons, from the code: AO is worked out at a face's four corners and blended across the whole 1 m face, so it's a faint block-wide gradient rather than a line along the crease; strength scales how full the space by a corner is, so a floor against one wall only reaches 0.5 at the corner; and outside edges (a stair's lip, a slab's edge, a wall's corner) get nothing, and most of what shapes show is outside edges. Everything here is client-side and tunable in the panel:
1. *AO curve*: darkening rises along a curve instead of in a line, so creases get darker while open faces stay bright. The user likes this one.
2. *Creases and ridges, per texel*: the mesher records what's past each of a face's four edges (an inside crease, an outside ridge, or flat); the shader darkens texels within a few texels of a crease and lightens those near a ridge a little, one flat shade per texel so it stays crisp and blocky. Sliders: crease darkness, ridge brightness, width in texels. Replaces corner AO, or keeps it only for corners where three faces meet. A few bytes per vertex (packed edge kinds, where on the face) and a few operations per pixel. Also removes the faint diagonal crease from per-vertex blending.
3. *Sun and sky*: replace the fixed brightness per face direction (`client::mesher::shade`: top 1, east/west 0.8, north/south 0.65, bottom 0.5) with a sun direction plus soft sky light. Faces point along axes, so it's six numbers a frame passed to the shader with no remeshing, and time of day becomes possible.
- *Later, not this toy:* stored light (the design doc's red, green, blue and sky channels per boxel, spread as a read-old automaton in `sim`) is infrastructure and arrives with caves from world generation or the first block that gives light; then dynamic lights; sun shadows last (ray marching per the design doc). Shading here multiplies over all of it.

## Sheen in shadow (2026-11-07)

Recorded at the user's request: the Blinn-Phong sheen needs to know about boxels occluding the path to the sun. Today a face's highlight comes from the sun direction alone, so a shiny face in shadow (ice under an overhang) still glints. Whatever casts sun shadows (the GPU boxel grid's ray march, see the backlog) should gate the sheen as well as the direct sunlight; only the sky's light reaches a face in shadow.
