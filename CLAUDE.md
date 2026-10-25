# Boxel Game

A moddable boxel survival game. The Rust engine ships with no content; the base game is a mod on the public API.

- `Boxel Game Design Document.md` is the source of truth for design decisions. Read the sections relevant to the current slice before planning it.
- `SLICES.md` is the roadmap: the ordered vertical slices, each with its current stage.

## Workflow

We work one vertical slice at a time. Each slice goes through these stages in order:

1. **Plan.** Read the slice in `SLICES.md` and the relevant design doc sections. Propose an approach: crates and modules touched, key types, how it will be tested, and open questions. No code until the user approves the plan. Write the approved plan under the slice in `SLICES.md`.
2. **Implement.** Build the approved plan in small steps. Keep `cargo check` and `cargo clippy` clean as you go. If the plan turns out wrong, stop and say so instead of quietly changing course.
3. **Test.** Write tests for the slice's behavior: unit tests for logic, plus integration tests where pieces meet. Run `cargo test` and report the real output.
4. **Verify.** Check the slice's "Done when" criteria one by one. Run the game when the slice is visual or interactive, and have the user try it. Report each criterion as met or not met.
5. **Close.** Update `SLICES.md` (mark the slice done, record decisions, anything deferred and any new API debt), update the design doc if a decision changed, and offer to commit.

### Git

Each slice or toy gets its own feature branch off `master` (for example `slice-1-minimum-engine`, `toy-movement`). Commit there, and when the slice is closed, merge it into `master` with `--no-ff` so each slice stays visible in the history. Never commit directly to `master`.

### At the start of every session

1. Read `SLICES.md` and find the current slice: the one in progress, or in phase 2 the current toy.
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
- Base game content lives in the `base` crate and uses only the `api` crate, never engine internals. If the API can't express something, extend the API. A shortcut is allowed only if it's logged in the API debt list in `SLICES.md`.
- When closing a toy, if the API debt list is getting long, suggest a seam slice next.
- Registries and saves use names, not raw numeric IDs.
- Simulation code must not depend on rendering. Keep them in separate crates so a headless server is always possible.
