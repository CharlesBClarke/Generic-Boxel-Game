# Boxel Game

A moddable boxel survival game. The Rust engine ships with no content; the base game is a mod on the public API.

- `Boxel Game Design Document.md` is the source of truth for design decisions. Read the sections relevant to the current slice before planning it.
- `SLICES.md` is the roadmap: the ordered vertical slices, each with its current stage. Keep it short: it's read every session.
- `docs/done-toys.md` archives each finished slice's full write-up. `docs/notes/<topic>.md` holds long design notes for backlog entries; read only the one for the slice being planned.

## Workflow

We work one vertical slice at a time. Each slice goes through these stages in order:

1. **Plan.** Read the slice in `SLICES.md`, its notes file if it has one, and the relevant design doc sections. Propose an approach: crates and modules touched, key types, how it will be tested, and open questions. No code until the user approves the plan. Write the approved plan under the slice in `SLICES.md`.
2. **Implement.** Build the approved plan in small steps. Keep `cargo check` and `cargo clippy` clean as you go. If the plan turns out wrong, stop and say so instead of quietly changing course.
3. **Test.** Write tests for the slice's behavior: unit tests for logic, plus integration tests where pieces meet. Run `cargo test` and report the real output.
4. **Verify.** Check the slice's "Done when" criteria one by one. Run the game when the slice is visual or interactive, and have the user try it. Report each criterion as met or not met. Every toy also runs the quick bench, `tools/bench.sh --quick master work` (a few minutes, unattended; tell the user first, as its window takes the screen), and puts the comparison table in the verify notes. It's a check for surprises, not a pass/fail: explain any row flagged slower, and glance at the screenshots in `~/.local/share/boxel/bench/shots/`.
5. **Close.** Mark the slice done and record decisions, anything deferred and any new API debt. Then move its full write-up to the top of `docs/done-toys.md` and leave one line under Done toys in `SLICES.md` (name, date, branch, what it gave the game). Decisions that still bind later work go in the design doc or the Phase 2 decisions list, not only in the archive. Update the design doc if a decision changed, and offer to commit. Then add a line to `docs/friction.md` for anything that slowed the work down (a missing recipe, a hard-to-find module, a confusing API, a slow test loop). If the same friction shows up a third time, point it out and suggest a fix. Delete entries once fixed.

Planning the next slice always happens in a new session, with a fresh context. After closing a slice, don't start planning the next one: at most, leave notes for it (in its notes file, or under its backlog entry if short) for the next session's planner to pick up, then tell the user to start a new session.

### The go word: "engage"

Don't start writing code or changing project files until the user says **"engage"**. Only that word counts; a casual "go ahead" in conversation does not. Approving a plan, answering a question or agreeing with an idea is not an engage. After proposing something, stop and wait.

- "Engage" covers the work that was just agreed, such as implementing an approved plan or making a requested change. Once going, keep going without checking in, unless something is needed from the user or the plan turns out wrong.
- Discussion and questions never start work. Answer them and stop.
- Without an engage, the only file changes allowed are keeping the stage marker in `SLICES.md` current and writing down something the user explicitly asked to have recorded.

### Bugs found later

When a problem turns up while working on something else, first decide which kind it is:

1. **A bug in code the current slice wrote or changed:** fix it in the slice.
2. **A bug in earlier, already-merged code:** fix it on its own branch (below), not inside the current slice.
3. **Working as built, but the user doesn't like it:** a design change, not a bug. Add it to the backlog in `SLICES.md`.

Backlog entries are short pitches, two or three lines. Longer design notes go in `docs/notes/<topic>.md`, linked from the pitch.

Fix branch steps:

1. Commit or close the current work first, so nothing half-done comes along.
2. Branch off `master` as `fix-<name>`.
3. Write a failing test that shows the bug.
4. Fix it, then run the tests and clippy. If the bug is visible in play, have the user check it.
5. Merge into `master` with `--no-ff`, then merge `master` into the current slice branch.
6. Add one line to the Fixes list in `SLICES.md`: date, branch, what was wrong.

A fix skips the full plan stage: describe the bug and the fix in a sentence or two, then wait for "engage". If the fix would change how the game plays, it is a toy, not a fix.

### Git

Each slice or toy gets its own feature branch off `master` (for example `slice-1-minimum-engine`, `toy-movement`). Commit there, and when the slice is closed, merge it into `master` with `--no-ff` so each slice stays visible in the history. Never commit project code (anything under `crates/` or `mods/`) directly to `master`. Commits that only touch bookkeeping (`SLICES.md`, `docs/`, the design doc, `CLAUDE.md`) or helper scripts in `tools/` can go straight to `master`.

### At the start of every session

1. Read `SLICES.md` and find the current slice: the one in progress, or in phase 2 the current toy.
   Run `tools/map.sh` when you need to find where something lives: it lists every crate and module with its one-line doc.
2. Tell the user which slice and stage we're on, and what the next step is. If nothing is in progress in phase 2, suggest a next toy from the backlog, picking one that needs little new infrastructure.
3. Don't start work until the user confirms.

### Keeping the order

Keep the stage marker in `SLICES.md` up to date as we move between stages.

If the user asks for something that breaks the normal order, say so before doing it. Examples:

- Writing code before the plan is approved
- Skipping tests or verification
- Starting a new slice, or work from a later slice, before the current one is closed
- Work that isn't in the current slice at all
- Building infrastructure (world generation, LOD, saving and the like) before a toy needs it
- Reaching around the mod API without adding the shortcut to the API debt list
- Changing an approved plan mid-implementation

Name the step being skipped and what it risks in one or two sentences, then ask whether to go ahead. This is a reminder, not a refusal. If the user says to proceed, do it, and note the deviation in `SLICES.md` so it isn't lost. Small fixes and questions that don't touch the slice don't need a warning.

## Conventions

- Coordinates: +Z up, right-handed, +X east, +Y north. Convert to Bevy's Y-up only at the render and import boundaries; engine and mod code only see Z-up.
- Positions are an integer cell plus a small float offset, never one large float. Only the renderer converts to floats, relative to its world view's origin.
- Use "boxel", not "voxel", in code and docs.
- Use the words in the design doc's Terminology table, the same way in conversation, code and docs. When the user uses a word that isn't there, or uses one differently, check what they mean with a concrete example before going on, then add it to the table.
- Base game content lives in the `base` crate and uses only the `api` crate, never engine internals. If the API can't express something, extend the API. A shortcut is allowed only if it's logged in the API debt list in `SLICES.md`.
- When closing a toy, if the API debt list is getting long, suggest a seam slice next.
- Registries and saves use names, not raw numeric IDs.
- Simulation code must not depend on rendering. Keep them in separate crates so a headless server is always possible.
