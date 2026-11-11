# Code from others

Code in the engine adapted from other projects, with each one's license.

| Where | From | License |
|---|---|---|
| `crates/client/src/sky.rs` (the time-of-day palette, the sun and moon arcs), `crates/client/src/atmosphere.wgsl` (`sky`, `sky_bodies`: the gradient, sun disc and halo, moon, stars) | Eanpa Sky by SkyeShark (https://github.com/SkyeShark/Eanpa-Sky, commit `a197d3d`), by way of the stylised-sky block by Patch | MIT: `LICENSE-eanpa-sky.txt`, `LICENSE-stylised-sky.txt` |

Ported from JavaScript and three.js's shader nodes to Rust and WGSL; the clouds, weather, rain
and the moon's baked sea texture were left out.
