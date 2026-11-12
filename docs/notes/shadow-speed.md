# Shadow speed

*Notes for the backlog's Shadow speed toy, from the Bench toy's shadow investigation (2026-11-10). Numbers are release builds in a 1080×1080 window on the Iris Xe, the mean of the bench's three terrain spots (about 1M triangles drawn, render distance 512 m), from `~/.local/share/boxel/bench/results.txt`. Rerun with `tools/bench.sh --study tools/bench/shadows.txt` (about 25 minutes), `shadows_small.txt` (a quarter-size window) and `drawing.txt` (MSAA and depth prepass).*

## Where the time goes

A frame takes about 17–18 ms with the shipped shadows and 6–7 ms without: shadows are about 11 ms, more than everything else together. By toy: Shading 5.6 ms, Stored light 6.1 ms, Sun shadows 17.9 ms.

| Set | ms | vs shipped |
|---|---|---|
| shipped (128 m, 3 maps of 2048, crisp) | 16.9 | |
| no shadows | 6.4 | −10.5 |
| shadow maps only | 13.0 | −3.9 |
| far shadow only (fills in everywhere with the maps off) | 13.2 | −3.7 |
| the user's tuning (64 m, 2 maps, soft) | 15.3 | −1.5 |
| cascades 1 / 2 / 4 | 15.8 / 16.6 / 19.3 | −1.1 / −0.3 / +2.4 |
| map size 512 / 1024 / 4096 | 16.0 / 16.6 / 23.3 | −0.9 / −0.2 / +6.4 |
| map distance 32 / 64 / 256 | 14.3 / 15.9 / 16.9 | −2.6 / −1.0 / 0 |
| soft / temporal filter | 18.0 / 18.1 | +1.1 / +1.2 |
| far reads 8 / 16 / 32 / 128 | 16.4 / 17.5 / 17.6 / 17.8 | within noise |
| far reach 128 / 2048 | 17.1 / 20.5 | +0.2 / +3.7 |
| render distance 256, shadows on / off | 13.0 / 5.0 | |
| quarter-size window (540×540): shipped / none / maps only / far only | 14.0 / 4.8 / 10.9 / 8.8 | |

Drawing (a later run, a little warmer: shipped 18.6, none 7.1):

| | shadows on | shadows off | shadows cost |
|---|---|---|---|
| shipped (4× MSAA, no prepass) | 18.6 | 7.1 | 11.6 |
| MSAA off | 15.0 | 5.8 | 9.2 |
| depth prepass | 21.2 | 11.4 | 9.7 |
| MSAA off, depth prepass | 16.2 | 8.3 | 7.9 |

## What it means

- **All GPU.** A probe of the GPU's idle time and every thread during four sets: the GPU was 100% busy at its top clock in each, no thread near a full core. Not Bevy's bookkeeping.
- **It follows the triangle count, not the pixels.** A quarter of the pixels cut the shadows' cost from 10.5 to 9.2 ms. A depth prepass, which does little but draw the triangles once more with the cheapest shader, costs 4.4 ms by itself (2.5 with MSAA off). So one pass over about 1M triangles is 2.5–4.5 ms here, and the shadow maps are more such passes.
- **Cascade count and map size barely matter** because cascades split the same distance: each chunk is drawn into about one map however many there are. The distance matters (32 m saves 2.6 ms).
- **The far shadow's read cap doesn't matter**: the walk up the pyramid of maximums usually ends within a few reads. Its cost is how many fragments run it; a longer reach costs (2048: +3.7 ms).
- **MSAA is on because it is Bevy's default**; nobody chose it. Off saves about 3.6 ms a frame (2.3 of it shadows) at the price of jagged edges. A choice for the user, by eye: the switch is in the tuning panel's View section.
- **A depth prepass is a loss** (hidden surfaces aren't the problem), unless the triangle count comes down a lot first.
- **The shadows-off numbers are a little slow**: with a light GPU load the game runs more frames, the CPU works harder, and the GPU is held to about 900 MHz instead of 1,080 (a shared power budget). So shadows cost somewhat more than the tables say. Bench lines record the clock; comparisons mark rows where it differs.

## Leads, in the order to measure them

1. **Fewer triangles: greedy meshing.** The design doc allows merging neighboring faces when texture, lighting and shading match. Generated terrain is long flat runs of one block; the catch is that baked light, AO and the crease lines differ per face, so see how much still merges, or whether those can move to a lookup the shader does. Speeds the main pass and the shadow maps together. Measure the triangle count before and after at the bench's spots (each line has it). *Counted 2026-11-10: 71% fewer triangles with plain greedy, 84% with AO in the shader; now its own toy, `docs/notes/greedy-meshing.md`.*
2. **Cheaper shadow maps.** Draw them only when the sun or the blocks near the camera change (a still sun is the default); a coarser mesh for casters; a shorter distance (the user already plays at 64 m).
3. **Store the far shadow** (the backlog's first idea): how lit each column's top is, worked out when the sun steps or a block changes, so a fragment makes one read instead of a walk; and make it run only past the maps' range even with them off, so each switch shows its own cost.
4. **GPU time per pass.** Bevy's render diagnostics can report the shadow pass and the main pass separately; in the bench's lines that would replace the inference above with numbers.
5. **LOD** for far terrain is the design doc's long-term answer to the triangle count and is infrastructure of its own.

*The user, 2026-11-10, asked "do you get the sense there is major room for improvent on this machine?"; the planner's answer was yes, a guess of shadows at 3–5 ms instead of about 10, unproven.*

## Settings study at 720p (2026-11-11)

*The user: "lets do a space serch for the biggest settings and try and make a 30fps, and 60fps profile", then "shadows and distence are more important", resolution "you pick". Benched at 1280×720 (half the user's 2560×1440 monitor each way, what a 50% render scale would draw), release, at `e789e36`. Rerun: `tools/bench.sh --study tools/bench/settings.txt` (about 20 min). The bench's three spots; ms is the mean of them, worst fps is the slowest spot's average.*

A landscape 720p window draws more chunks than the square 1080×1080 bench (a wider view), so shipped is slower here (30 ms) despite fewer pixels.

| Set (one change from shipped) | mean ms | vs shipped | worst fps |
|---|---|---|---|
| shipped (512 m, maps 128 m ×3 2048 crisp, far shadow, 4× MSAA) | 30.3 | | 26.0 |
| no shadows | 10.6 | −19.7 | 72.9 |
| shadow maps off | 22.7 | −7.6 | 31.3 |
| far shadow off | 19.8 | −10.5 | 47.5 |
| MSAA off | 19.4 | −10.9 | 47.9 |
| AO off / edges off / sheen off | 26.7 / 29.1 / 29.6 | −3.6 / −1.2 / −0.8 | |
| render 192 / 256 / 384 / 768 | 9.2 / 13.5 / 22.1 / 39.6 | −21.2 / −16.8 / −8.2 / +9.3 | 82 / 62 / 37 / 21 |
| map distance 48 / 64 / 256 | 24.7 / 25.7 / 30.7 | −5.6 / −4.7 / +0.3 | |
| cascades 2 / 4 | 29.3 / 32.4 | −1.0 / +2.1 | |
| map size 1024 / 4096 | 28.3 / 37.0 | −2.0 / +6.7 | |
| soft filter | 30.5 | +0.1 | |
| far reach 256 / 1024 | 29.6 / 32.9 | −0.7 / +2.6 | |
| shipped again (drift) | 30.6 | +0.3 | |

What it says:
- **MSAA and the far shadow cost about the same, ~11 ms each.** Together off wasn't measured; with 4× MSAA every shaded edge pixel runs more fragment work, which may be why MSAA costs this much. MSAA off is the cheapest big cut, and matters least for this game's blocky look.
- **Map distance 256 and the soft filter are nearly free** at 720p: the shadow maps cost more by their size (4096: +6.7) than by their reach.
- **Render distance is the other big lever** (triangles: 768 m draws 3.7M).
- Throttling was heavy in later runs (GPU at 840–900 MHz, "throttled 100%"), so candidates were confirmed twice each, alternating.

### The profiles

In `~/.config/boxel/profiles/` (machine-specific, outside the repo). MSAA, edges, sheen and AO are tuning-panel checkboxes, not saved in the tuning file, so they're unticked by hand each run.

| Profile | Settings (changes from shipped) | worst fps (two runs) | 1% low |
|---|---|---|---|
| **30 fps** | MSAA off, edges off, render 576 m, map distance 256 m, soft filter | 40.0 / 38.9 | 34 / 33 |
| **60 fps** | MSAA off, edges/sheen/AO off, render 384 m, map distance 64 m, map 1024, soft filter (far shadow kept) | 82.6 / 75.5 | 62 / 50 |
| 60 fps, AO kept | as above but AO on, render 320 m | 69.0 / 67.8 | 59 / 58 |
| rejected: 60 fps with AO at 384 m | | 68.2 / 62.5 | |

Edges are off in both because the user's tuning already has crease, ridge and edge width at 0. The 60 fps profile keeps the far shadow and gives up AO before distance, per the user's priorities; fog closes in at 384 m and hides the far mountains.

## Algorithmic fixes to consider (2026-11-11)

*Discussed after the settings study. The user: "Do you think we need more opimizations because the game is so bare looking, we will need trees, bushes, water", "I don't want to opimize for photo takeing", "Is there any operntunity to get down and dirty in our core systems for alg inporvemts". Guesses at the size of each, none measured; in the planner's suggested order.*

1. **GPU time per pass.** Bevy's render diagnostics time the main pass, each cascade and the far shadow separately; put them in the bench's lines so every lead below is a measurement, not an inference from frame totals. About a day.
2. **A shadow-only mesh.** Shadow maps need depth only, so their mesh can merge faces across blocks, light and AO (which the visible mesh can't), skip the T-junction splits, and probably keep only sun-facing faces. Could cut every shadow pass's triangles several-fold; the planner's best payoff for the work, and trees' leaves would cast through it from the start.
3. **Store the far shadow** (lead 3 above): one read per pixel instead of a walk, updated when the sun steps or a block changes. Far shadow off saved 10.5 ms at 720p with everything else shipped.
4. **Occlusion culling.** Hills hide whole valleys at the overlook, yet every chunk in range is drawn, and drawn again into the shadow maps. A flood fill through chunks the camera can see into (the usual boxel-engine way) or Bevy's GPU occlusion culling. Medium to big in hills, nothing on flat ground.
5. **Border splits only at real corners** (from `fix-greedy-sparkles`): mesh a chunk after its neighbors so its border edges split only where a neighbor's corner is. Up to ~4 ms back.
6. **Overdraw**: sort opaque chunks near to far. Lowest; the depth prepass result says hidden surfaces aren't the main cost.
7. **LOD** for far terrain: the biggest for long views (triangles grow with distance squared without it), and the most infrastructure; it also makes every shadow redraw cheaper.

**Cached shadow maps** (asked about): redraw a cascade only when the sun steps, a block in it changes, or the camera crosses its grid step (cascades snapped to a coarse world grid and fitted to a circle around the camera, so turning never redraws); moving things (cows, items, the club) drawn each frame over a cached terrain-only copy. The user's point: in normal play "isnt that going to be almost evry frame". Answer: the near cascade yes, but it's cheap; the far ones, which hold most triangles, would step every second or two at walking speed. Needs our own shadow pass (Bevy redraws every frame), so after LOD and the shadow-only mesh.

**The bench needs a moving case first.** It stands still at three spots, which would flatter any caching. Before building caching (or judging streaming), add a fly path at walking and sprinting speed.
