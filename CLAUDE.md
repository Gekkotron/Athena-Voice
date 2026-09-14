# PLAN.md format contract

This project is driven by the **Athena Orchestrator**
(`~/superconductor/projects/Athena-Orchestrator`). A launchd tick fires every
15 min, calls `firstBacklogTask()` in the orchestrator's `src/plan.ts` against
this repo's `PLAN.md`, and dispatches the first task to a fresh Claude session.

If `PLAN.md` doesn't obey the shape below, **nothing runs** — the tick logs
`skipped-empty` and moves on silently.

## The shape

- Section headers are `##` **only** — level 2, spelled exactly.
- The parser recognizes exactly three sections: `## Backlog`, `## In progress`,
  `## Done`. Every other `##` is invisible (useful — see "Escape hatches").
- Tasks are top-level `- [ ]` (open) or `- [x]` (done) bullets **directly under**
  one of the three sections. No nesting, no leading `### Subheader` between the
  section and the checkbox.
- A task's body is any lines **indented** under the checkbox, contiguous. The
  body stops at the first blank line, a new `- ` top-level bullet, or a `## `
  header — so no blank lines and no nested `- ` bullets inside a body.

## Required example

```markdown
## Backlog

- [ ] Concrete task title — one line, imperative
      First body line, indented, no blank line before it.
      Second body line — describes what the headless worker must do.
      Success criteria: (a) …; (b) …; (c) ….

- [ ] Next task title
      …
```

## Anti-patterns (all silently invisible to the tick)

- `### Subheader` + prose under `## Backlog` describing a task — parser walks
  past `###` lines, so no `- [ ]` is registered.
- Nested `- ` bullets in the body — the body collector stops on the first `- `,
  everything after is dropped.
- Blank lines inside a body — same reason.
- Renamed section headers (`## backlog`, `## To do`, `### Backlog`) —
  case-sensitive, level-2 only.

## Escape hatches

Any `##` header that isn't `Backlog`/`In progress`/`Done` is **ignored by the
parser** — safe for content you don't want dispatched:

- `## Notes` — risks, strategy notes, architectural context.
- `## Manual checklist (human, not the orchestrator)` — `- [ ]` items you
  intend to complete by hand.

## Session metadata

When the tick dispatches a Backlog task it moves it to `## In progress` and
appends an HTML comment on the checkbox line:

```
- [ ] Task title     <!-- session: conv:claude:… -->
```

You can also add `<!-- blocked: reason -->` on an in-progress task to have the
planner surface it in the next planning session. Both comments are stripped
from the task's title before dispatch.

# Disk hygiene

This workspace accumulates build artifacts unusually fast. The runtime's build
script emits a ~470 MB compiled whisper.cpp tree into a new
`target/debug/build/athena-voice-runtime-<hash>/out/` whenever its inputs
change, and cargo never reclaims the superseded copies — left alone, `target/`
reached 38 GB in four days of ordinary work.

**The rule: when disk is tight, run `./scripts/clean-stale.sh`, never
`cargo clean`.** The script drops superseded build-hash directories (keeping a
few generations per package) and incremental caches; `--all` adds
`satellite-esp32/target` and finished review scratch under `.superpowers/sdd/`.
`cargo clean` frees more but forces a full whisper.cpp rebuild, which is a poor
trade in a headless orchestrator session.

There is deliberately no scheduler — cleanup is manual, because a background job
that silently invalidates a build cache mid-session is worse than a full disk.
Do not add one without asking.

Two things the script must keep, and does: `models/` (downloaded whisper and
piper weights, 680 MB) and `satellite-esp32/.embuild/` (the ESP-IDF toolchain,
3.8 GB and slow to re-fetch).
