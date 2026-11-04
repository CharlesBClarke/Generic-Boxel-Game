# Friction log

What slowed the work down, one line each: date, slice, what, rough cost. Added at Close (see `CLAUDE.md`). When the same kind of friction shows up a third time, raise it and suggest a fix (a seam, a script, a recipe, a doc paragraph, a refactor). Delete entries once fixed.

- 2026-11-03, release mode: adding a control action meant reading all of `client/src/controls.rs` to find its four places (struct, default, parser, test), plus remembering the user's controls file and the HUD label. No recipe for changes that span files; `docs/recipes.md` was proposed and left until this comes up again.
- 2026-11-03, release mode: finding where the HUD, tuning panel, AO and controls loader live took several searches. Fix: `tools/map.sh`.
