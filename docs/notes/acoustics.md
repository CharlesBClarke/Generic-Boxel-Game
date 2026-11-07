# Acoustics

Moved from the `SLICES.md` backlog on 2026-11-03; the backlog keeps a short pitch.

**Acoustics.** Added 2026-11-03 at the user's request (sound ray tracing; an earlier discussion of it was never written down). After Sound. A boxel grid makes it cheap: rays step through boxels the way block picking does. Levels:
1. *Muffling behind walls* (easy): a ray from the ear to each playing sound every few frames; solid boxels crossed lower its volume and high frequencies (partial shapes count partly, by texels).
2. *Room feel* (easy to medium): 32 to 64 rays out from the ear a few times a second; how far they go gives the room's size and how many reach open sky tells a field from a cave; that drives a reverb. Materials could absorb or reflect (stone hard, dirt and wood soft), through block material definitions.
3. *Around corners* (medium, later): a flood fill through air boxels from the sound to the ear gives the path length and the direction it arrives from (a doorway, not through the wall). The same grid flood idea as stored light; could share it.
4. *Full acoustic ray tracing* (many bounces, per-material echoes, like Steam Audio): skip; a world players keep changing breaks the precomputation those rely on, and 1 to 3 get most of the effect.
- Best once there are caves or buildings to hear it in (the test area's brick and wood house gives a little). Needs the filters and reverb from Sound's audio library choice.
