# Material behavior (next toy after Functional crafting)

*Planned and built 2026-11-06: the vanisher became phasite and antiphasite (see the plan in `SLICES.md`); the checklist below is in the design doc.*

Split out of Functional crafting on 2026-11-06 at the user's request: that toy closes with several materials in one shape; what materials *do* is planned again here.

## What exists (first try, built in Functional crafting)

`api::Property` (`trigger`, `hinge`, `listener`), set per material in `blocks.toml`, worked by `sim::signal`. Brass triggers (interact on it), copper listens to a trigger in a touching boxel, iron hinges: the boxel and its hinge line swing a quarter turn about the hinge piece's long axis. A knob makes the door stick out of its boxel either way, so the swing slides everything back inside; opened pieces are "open" kinds that remember the closed occupant (`BlockRegistry::opened`). The test area has a door and lever at `DOOR_SPOT`.

The user's verdict from play: the hinge needs tiebreaks (which axis when the hinge piece isn't clearly long, which way when both fit: counterclockwise) and a slide, so swings look arbitrary and move too far. Copper only reaching one block surprised them: they expected wire.

## Agreed for this toy

- **Bundles break whole.** Breaking any piece of a bundle takes off all of that bundle's pieces in the boxel (open or not) and gives the bundle back as one item; other occupants stay. World building gets a way to place a bundle, so the test door breaks like a crafted one.
- **Copper conducts.** A signal runs through every face-touching boxel holding copper, instantly, and every boxel reached is signaled (capped at a few thousand boxels). Rename `listener` to `conductor`. Two doors on one wire open together; a knob can be wired to a door.

## A checklist for rules (proposed; goes in the design doc)

The user: "we need to be more disciplined about how we choose rules… we fix little annoyances greedily". Check every rule before building it:
1. Predictable from what you see; if not, show it first (like the placement ghost).
2. No hidden tiebreaks: a rule that needs one is wrong; let the player choose, or refuse with a reason.
3. Fits in one sentence (the crosshair hint). "Unless" means it's too fussy.
4. Local and stateless where possible.
5. Undoable: doing it again gets back exactly.
6. Restrictions only stop the player making things they can't use or place.
7. Small verbs that combine, over clever ones with cases.

The hinge fails 1 to 3. Placement likely fails 2 and 3 in places (see the backlog's Review placement against the checklist).

## Candidates to replace the hinge

- **A vanisher (the user's idea).** "When signaled, a vanisher hides itself and everything within reach of it, any material, any boxel; the next signal brings it all back." Players insulate what should stay (a handle) with a gap: sub-boxel play. Open:
  - Reach: one sixteenth (as the user said) or one quarter (the crafting grid's unit, so an insulating gap is one empty cell on the table; recommended).
  - Touching vanishers act as one (a two-high door goes together); plain materials never pass it on.
  - Pieces partly within reach are cut, the rest stays; each boxel's originals are kept in a record in the simulation and restored exactly. Refused if a body or something built since is in the way.
  - A signal to any boxel the vanishing touched brings it back (the surviving handle, or a lever over copper).
  - The frame loses its strip within reach while open unless the door is built with a gap.
  - Which material: iron, or a new one.
- **The hinge as a sub-grid** (the user, 2026-11-06). A swinging door is a small grid of its own turning about its real hinge line, sticking out of its boxel while open, so there's no slide and no in-boxel limit. A chance to do the backlog's Sub-grid trial with a real use. Bigger: collision with moving, rotated grids.
