# More materials

Batches B and C of Experimental materials, deferred when that toy closed with only batch A (2026-11-09). The user: "deffer other batches" and keep them as one entry, not separate future plans. The toy's goal still holds: find which set of materials balances trivial use for players who don't care against combining for those who do, and compare boxel-wide against touching signals.

## B. Signals
- *Sensor:* a body (player or cow) touching a piece of it signals it, once per touch.
- *Delay:* passes a signal it gets on after `delay` seconds (slider). Signals still waiting are dropped on save.
- *Trigger scope switch* (a tuning toggle): boxel-wide (today: a signal reaches a boxel and every piece in it answers; copper carries boxel to boxel) or touching (a signal reaches only the pieces touching the piece that sends it; copper carries only through copper pieces that touch). Crystals always use texel touching.

## C. Feel
- *Bounce:* landing on it springs you up (jump pads); the movement API sees it like friction.
- *Falls:* unsupported pieces drop, like sand. (Climb was left out.)

## Throughout
The crosshair hint lists the aimed material's properties (already built, `crafting::property_words`); a demo spot in the lab (`base::LAB_SPOT`) per batch; every new material is endless.

## Done when (as planned)
- A sensor fires a trap when a cow or the player steps on it; a loop of delays makes a clock.
- The trigger scope switch changes what the knob example does (a knob not touching a phasite strip in its boxel flips it boxel-wide, not touching).
- A bounce pad springs the player; a falls material drops when what holds it up is broken or hidden.

## Also carried over from batch A
- **Beam tuning** ("maybe deffer tuneing it"). Today every beam is alike and one fires per lens piece and direction, so several separately placed green pieces in a boxel stack beams (more damage, a longer tunnel) while one green gadget, however layered, fires once: a hidden rule. Candidates: strength and damage from the green volume used up, and the beam's width from the lens's outline down the beam.
- **Setting crystals off from a distance.** With the grab at 1 texel and the table working in quarters, a player can't make a hidden blue crystal: antiphasite only cuts a quarter-thick blue, and the kept part still touches the green. The lab's trigger is texel-exact, built by the base mod (`shapes::crystal_trigger`). With `grab` at 4 the quarter version works. Decide with the user how a player sets crystals off from a distance.
- **Bloom** for glowing materials: needs Bevy's HDR path, which changes the tone mapping of everything. The lamp "felt pointless wihtouut lighting", so it waits for Stored light.
