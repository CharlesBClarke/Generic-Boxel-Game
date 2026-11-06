# Sound

**Done 2026-11-05** (see `docs/done-toys.md`). What's left from these notes is the backlog's *Robot voice and jump sound*:
- *Robot voice:* espeak-ng (installed, 1.52, GPL-3) renders lines to files offline through a `tools/` script, so the game never links GPL code. Ideas: knocked out, back up after a knock-out, low-health warning, grunts on hard hits. They'd fill `PlayerSounds` slots like `knocked_out`, or new ones.
- *Jump:* detected in the client like landings (the body leaves the ground going up faster than about 3 m/s; walking off a ledge stays silent, a block-jump counts). Two layers: the surface's footstep, quieter, as a push-off, and a pickable `jump_` sound (a generated pneumatic hiss, a printer chirp, a thruster slice, a cloth swish, a light metal tap).

The original pitch, kept for reference:

Moved from the `SLICES.md` backlog on 2026-11-03; the backlog keeps a short pitch.

**Sound.** Added 2026-11-03 at the user's request. The game is nearly silent: one sound, the cow's moo (`mods/base/sounds/moo.ogg`), played flat (not positional) on hits, slams and hurts with a little pitch spread (`client::combat::play_sound`). Mods register sounds (`SoundDef`), but only creatures can name one (`hit_sound`). Ideas, by payoff:
1. *Positional sound*: Bevy's spatial audio, a listener on the camera and sounds where they happen, converted to Y-up at the render boundary.
2. *Movement*: footsteps by the block's material (faster when sprinting), a landing thud by speed; the robot's shock absorber hisses and bottoms out with a clunk on landings that hurt, like the camera.
3. *Blocks*: break and place sounds per material.
4. *Combat*: swing whoosh, hit thump, a sharper head-hit crack, slam crunch, guard clang, the charge's hoofbeats, the player's hurt.
5. *Volumes* in the tuning panel (master and groups), kept by Save.
- *API*: block definitions gain step, break and place sounds per material; tools gain swing and hit sounds. The client plays them from events the simulation already reports (strikes, slams, hurts, landings), so the simulation stays silent and headless.
- *Sourcing (open)*: a CC0 pack (such as Kenney's impact and footstep packs, credited like the cow), generated sounds from data recipes like the looks, or a mix. Recommended: the mix, CC0 for natural sounds (steps, impacts, cows), generated for the robot's own sounds. Suggested first scope: 1, 2 and 4.
- *Servo on quick pans* (the user's idea): faint, varying in length and strength, so not a one-shot file but a steady loop always playing, its volume and pitch driven each frame by the look's turn speed: silent below a threshold so normal aiming is quiet, rising to a faint maximum on fast pans, pitch up a little with speed, fading in quickly (about 30 ms) and out a bit slower (about 120 ms). Optional spin-up and stop one-shots. Not positional. Every value in the panel. Recommended: generate it (a buzzy motor tone whose pitch follows speed, a few gear harmonics, a little rattle; the `fundsp` crate), falling back to a recorded seamless loop: freesound.org with the CC0 filter ("servo", "RC servo", "small motor whir", "actuator", "stepper motor", "camera lens zoom"), or a recording of an RC servo, printer head or toothbrush cut to a steady stretch and looped at zero crossings in Audacity.
- *Audio library (decide first)*: Bevy's built-in audio changes volume and speed live but has no filters or reverb, which Acoustics needs. Pick `kira` (through `bevy_kira_audio`; check it supports Bevy 0.18) or a custom pipeline with `fundsp` at the start, so nothing is built on Bevy audio and moved later.
