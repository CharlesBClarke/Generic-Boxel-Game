# Solid creatures

Added 2026-11-03 at the user's request: "I don't like how unsolid many games enemies are and how being able to stand on them would improve that". Chosen as its own toy; the Sub-grid trial stays a rotating platform, the cleaner test of grid collision.

**Today.** Cow herd chose a soft push on purpose: overlapping bodies are nudged apart over a few ticks, so nothing gets stuck or jitters. That is the unsolid feeling: you sink into a cow a little before it shoves you, and you can never stand on one.

**Proposal.**
- A real collider per creature (an upright cylinder, or a box turned with its facing) that the player's collision treats like terrain: you bump into a cow instead of sinking in.
- Standing on one: a creature under the player is the ground, and its movement carries them. The engine already owns what a body stands on for movement controllers (design doc, "Movement is content"), so any mod's creature works with any controller.
- Play that falls out of it: jump on a cow to escape a fight, get bucked off or carried off, jump off a charging cow's back, stand on a stunned cow to reach a ledge, cross a herd on their backs.

**To settle in the plan.**
- Crushing: a cow walking into a player pinned against a wall. A push that can't resolve has to give somewhere (likely why the soft push was chosen).
- Accidental step-ups: the player's step height is 0.6 m, so small creatures may need to block stepping.
- Animation: the collider is the body's rough shape; a grazing head has no collision of its own (like hit boxes today; see Head hit boxes).
- Creature against creature: keep the soft push between cows and make only player-creature contact solid, or both.

**Shares with the Sub-grid trial:** being carried by something that moves, with the rider kept in its frame. Whichever comes first builds it; the other reuses it.
