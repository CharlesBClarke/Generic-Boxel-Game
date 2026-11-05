# Friction log

What slowed the work down, one line each: date, slice, what, rough cost. Added at Close (see `CLAUDE.md`). When the same kind of friction shows up a third time, raise it and suggest a fix (a seam, a script, a recipe, a doc paragraph, a refactor). Delete entries once fixed.

- 2026-11-03, release mode: adding a control action meant reading all of `client/src/controls.rs` to find its four places (struct, default, parser, test), plus remembering the user's controls file and the HUD label. No recipe for changes that span files; `docs/recipes.md` was proposed and left until this comes up again.
- 2026-11-05, local server seam: every test crate (sim tests, client, server) defines its own `base_sim()` that loads the base mod natively; client tests needed a lint exception to do it. A shared test helper (say a `testing` crate) would remove the copies.
