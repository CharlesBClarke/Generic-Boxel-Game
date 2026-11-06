# Credits

Media in this mod made by others. The cow and its moo are from TenPlus1's `mobs_animal` mod for Luanti
(https://codeberg.org/tenplus1/mobs_animal), as its `license.txt` lists them.

| File | Original | Author | License |
|---|---|---|---|
| `models/cow.glb` | `mobs_cow.b3d` | sirrobzerrone | CC0 |
| `textures/cow.png` | `mobs_cow.png` | sirrobzerrone | CC0 |
| `sounds/moo.ogg` | `mobs_cow.ogg` | from freesound.org | CC0 |

`cow.glb` was converted from the b3d model with its bones and animation by `tools/b3d_to_glb.py`,
cutting clips at the frame ranges and speeds in the original mod's `cow.lua`:

    python3 -I tools/b3d_to_glb.py mobs_cow.b3d mods/base/models/cow.glb \
        eat:0-30@20 stand:35-75@20 walk:85-114@20 run:120-140@40 punch:145-160@20 die:165-185@25

## Sounds

Every sound file here is used by `sounds.toml`. More to try are in `~/Projects/boxel-sound-packs/`
(not in the repo): the packs as downloaded, and converted copies of the ones tried. Downloaded
files were made mono where they weren't, and those that started late (up to 280 ms of dead air)
were cut by `tools/trim_starts.sh` to start 2 ms before their sound.

| Files | From | License |
|---|---|---|
| `footstep_*`, `impact*` | Kenney (https://kenney.nl), Impact Sounds | CC0 |
| `scifi_*` | Kenney, Sci-fi Sounds | CC0 |
| `step_lth*` | https://opengameart.org/content/footsteps-leather-cloth-armor | CC0 |
| `gravel_snow_*`, `gravel_ice_*` | recorded by Corsica_S on freesound (28661, 28662), cut by Iwan Gabovitch: https://opengameart.org/content/42-snow-and-gravel-footsteps | CC0 |
| `hit_mid_*` | independent.nu ljudbank, hits and punches (OpenGameArt) | CC0 |
| `kill_splat_*` | independent.nu ljudbank, wet squish and slurp impacts (OpenGameArt) | CC0 |
| `swing_rpg_*`, `pickup_beads` | RPG Sound Pack (OpenGameArt; `battle/swing*`, `inventory/beads`) | CC0 |

The OpenGameArt licenses were checked by the user (2026-11-05); the site refuses automated
fetches.

## Generated

`powerdown.ogg` (a falling buzz) and `wind.wav` (filtered noise, looped) are made by
`tools/make_sounds.sh`.

## Temporary: replace before release

`printer.wav` is cut by `tools/make_sounds.sh` from a downloaded 3D printer recording
(`hammy01-3d-printer-283745.mp3`, in the packs folder; apparently from Pixabay, whose license
isn't CC0). It's here to try the sound out only. If it stays, replace it with our own recording.
