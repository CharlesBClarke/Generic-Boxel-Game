# 3D crafting view

Backlog entry in `SLICES.md`. Feedback recorded 2026-11-06 at the end of Material behavior; not planned yet.

## What the user said

> I like it but I hate it, its kind of fun thinking about how to make a door nice, I often fight the ui, and I really hate having to manually make almost identical sub componets that are just rotations of eachother. better ui might help. It does seem like alot of work just for every component, its like MC but shrunk down, like you spend the time to make the little things work well

> what if a crafting table becomes a truly 3d ui

So: designing components is fun; the tools fight it, and repeating the same work is the pain. A door needs a closed panel and the same panel turned a quarter, each built by hand.

## Ideas raised (none chosen)

- **A 3D editor in a menu:** the boxel in a window you turn around, clicking cells in and out. Little new infrastructure, but still a second UI with its own controls.
- **Build in the world, enlarged:** the table shows the boxel being designed above it, each quarter-cell a full block, built with the normal place, break, ghost and turning controls, then shrunk back into a part. No second control scheme; turned and mirrored copies come naturally. Needs a small separate grid the player can edit and the client can draw at scale (a sub-grid or a pocket world through a world view, both in the design doc). Leaned toward during discussion.
- **Crawl into the table (bigger on the inside):** added 2026-11-06 at the user's request ("you crawl into the ui and its bigger on the inside"). A twist on building in the world, enlarged: instead of the enlarged boxel appearing above the table, the player goes *into* the table, a pocket world where the boxel being designed is room-sized and built with the normal controls. Uses the design doc's multiple world views (a pocket dimension is a sub-grid not placed in the main world; the table could show a portal-style view into it from outside). Would make the table the first toy to need world views.
- **A turned or mirrored copy of a design in one click,** whatever the UI.
- **Saved designs:** a design made once becomes a recipe made again in one step, later shareable.
- **The bigger question:** how much of the game happens at sub-boxel scale. If designing components is the fun, it needs these tools; if it's occasional, most players get ready-made pieces. Feeds the signature feature choice.

Open: resolution (quarters as now, or finer).

## Placing is part of the crafting problem

Recorded 2026-11-07 at the user's request: placing rules are tied closely to the problems with crafting, so proving crafting includes reviewing placement (backlog: Review placement against the checklist, Speed build). How a part goes into the world decides how it has to be built at the table, and the other way round.
