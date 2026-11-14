# Look variants

*Notes for the Look variants backlog entry in `SLICES.md`. Written 2026-11-14, at the close of Grass depth, from a talk after `fix-noise-mip-mix`. The lazy version was built and closed the same day (`docs/done-toys.md`); seams were judged fine, so Wang cubes stay here as the fallback.*

## Why

A look today is two tools: the baked 16³ volume, which degrades with distance through its mips, and generated detail (noise and grain from the world position, `api::look`, `look_noise.wgsl`), worked out per pixel, which degrades by fading octave by octave and needs its own averages kept equal to the mips. `fix-noise-mip-mix` was a bug in keeping the two in step. The user (2026-11-14): "OK the game looks bad, I accepted that it looks bad hoping for a future toy where we build a material asset tule where a user can make a 16*16*16 color and, reffrence the noies tools to make it look good. We added nosie just so things don't repet every time. but what I don't like is that we have two diffrent tools that effect the texture on blocks and how they degrade with distence."

## The idea

Noise becomes a tool for making looks only; the game draws nothing but baked volumes. Repetition is broken by **variants**: a look's recipe is baked several times with different seeds, and each block shows one, picked by a hash of its position. Everything degrades one way, by the mips, which far terrain and later LOD averages read too. The runtime noise, the detail byte, the fade sliders (`noise_fade_start`, `noise_fade_end`) and the noise half of the looks table go. Probably cheaper: per-pixel noise cost ~3 ms at the bench's `recorded` (Generated detail, 2026-11-11); picking a variant is a hash. Memory: 16 KB a variant.

Still one look per material: the look holds its variants.

## Lazy first (the user's pick)

The user: "what do you sugest, maybe lets try the lazy way, noise that dosent loop see if its too bad". Bake N variants (say 8) of each noise look, the noise not wrapping at the cube's edges, picked per block at random; accept seams where neighbors differ and see whether they show at 1.5 to 2 texel speckle. A pick list for N (1, 4, 8, 16) to judge seams and repetition; changing it may need a restart (variants are baked at load).

## If seams show: Wang cubes

Each face between two blocks gets a label (A or B) from a hash of its position, shared by both blocks; a block shows the variant baked for its six labels: 2⁶ = 64 variants (1 MB a look). Value noise makes the baking exact: the lattice points on a face plane (and those its blend reaches) come from that face's label, the interior from the variant's seed, so neighbors meet seamlessly. Lattice points on cube edges and corners belong to several faces and are fixed for all variants (a small regularity, probably invisible in speckle). Variants can't also be turned or mirrored. Labels on corners instead (2⁸ = 256 variants) avoid the fixed edges. Same pipeline as the lazy version; only baking and picking change.

## Open

- Large-scale variation (patches meters across, the "Octave noise and patch tint" entry): a gentle tint by world position that averages to no change, so it can't fight the mips.
- Grain (wood, logs): baked into variants like noise; planks and bricks already line up at block edges, so their variants vary board and brick colors.
- Looks painted texel by texel (the material tool) have no recipe to vary: one volume, or the tool's noise brushes applied per variant.
- Reverses the Generated detail decision ("generated detail comes from the world position through `api::look`"): update the design doc and the Terminology table's Look entry when it lands.
- The material tool (separate backlog entry) would produce variants.
