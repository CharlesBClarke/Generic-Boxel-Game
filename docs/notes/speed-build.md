# Speed build

Added 2026-11-06 at the user's request, while talking about ways to find better rules (see the checklist in `material-behavior.md`).

## The idea

A toy that tests placement rules by timing them instead of arguing about them. The game hands out a set of placing challenges, and the user builds each one as fast as they can.

Example from the user: you start inside a hollow cube and have to put stairs in every inner corner, the vertical ones included (floor-to-wall, wall-to-wall and ceiling-to-wall edges).

## How it's used

- Run the same challenge under different rules, or with two different kinds of stairs, and compare.
- Also try it once placing carefully rather than fast, to see what precision costs.
- After each run Claude writes a short report on what felt good and what didn't, from the user's comments plus the numbers.

## What a report could hold

- Time per challenge and per piece.
- Misplaced pieces: placed, then broken or replaced, and which rule or ghost misled.
- Pieces that needed turning, a mode switch or a second try.
- The user's comments: what felt good, what didn't.
- A verdict on each rule against the checklist (predictable, no hidden tiebreaks, one sentence).

## Open

- Where challenges come from: hand-built rooms in the test area, or a small data file of target shapes with a ghost of the goal.
- How a run is judged done: the game checks the target, or the user says so.
- Where the drill-like game logic lives; the chase drill already sits in the client as API debt.
- Pairs with Review placement against the checklist and Recheck the crafting rules.
