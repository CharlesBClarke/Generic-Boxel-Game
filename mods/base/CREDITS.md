# Credits

Media in this mod made by others, all from TenPlus1's `mobs_animal` mod for Luanti
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
