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

1. **Fewer triangles: greedy meshing.** The design doc allows merging neighboring faces when texture, lighting and shading match. Generated terrain is long flat runs of one block; the catch is that baked light, AO and the crease lines differ per face, so see how much still merges, or whether those can move to a lookup the shader does. Speeds the main pass and the shadow maps together. Measure the triangle count before and after at the bench's spots (each line has it).
2. **Cheaper shadow maps.** Draw them only when the sun or the blocks near the camera change (a still sun is the default); a coarser mesh for casters; a shorter distance (the user already plays at 64 m).
3. **Store the far shadow** (the backlog's first idea): how lit each column's top is, worked out when the sun steps or a block changes, so a fragment makes one read instead of a walk; and make it run only past the maps' range even with them off, so each switch shows its own cost.
4. **GPU time per pass.** Bevy's render diagnostics can report the shadow pass and the main pass separately; in the bench's lines that would replace the inference above with numbers.
5. **LOD** for far terrain is the design doc's long-term answer to the triangle count and is infrastructure of its own.

*The user, 2026-11-10, asked "do you get the sense there is major room for improvent on this machine?"; the planner's answer was yes, a guess of shadows at 3–5 ms instead of about 10, unproven.*
