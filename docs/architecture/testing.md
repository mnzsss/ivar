# Test layout and retained large files

Part of the [architecture overview](../../ARCHITECTURE.md).

## Test layout

Every Rust test body lives under `tests/`, never under `src/`. There are two
compilation boundaries, and the layout keeps them distinct:

```
tests/
  static_checks.rs          checks that read the repository: the layering rule,
                            the centralization invariant, the command reference
  cli.rs delivery.rs …      top-level integration targets, discovered by Cargo
                            and compiled as their own crates; each links its
                            modules from the directory of the same name
  support/
    shared.rs               the one implementation of temp-dir and real-Git
                            helpers
    unit.rs                 linked from src/lib.rs as crate::test_support
    integration.rs          linked from each integration target as `common`,
                            adding the assert_cmd binary and manifest helpers
  unit/                     the unit-test tree, mirrored against the module
                            that owns each #[path] link — not wall-to-wall
                            against src/, since a facade's children may
                            share the facade's linked test file
```

A production file keeps only a path-linked declaration; the body lives in the
mirrored file under `tests/unit/`:

```rust
#[cfg(test)]
#[path = "../../tests/unit/error.rs"]
mod tests;
```

The `#[path]` link means the module is still compiled as a child of its owning
production module inside the library test crate — `use super::*` and access to
private parent items keep working, so relocating a test never widens production
visibility. Physical location and compilation home are deliberately different
things, and `tests/static_checks/architecture.rs` enforces all three: the layering scan walks
`tests/unit/<module>/` with the same allowed imports as `src/<module>/`, a
second rule refuses any `#[test]`, `#[rstest]`, or inline `mod tests { … }`
body under `src/`, and a third walks the physical `tests/unit/` tree and every
`#[path = "…tests/unit/…"]` link in `src/` and asserts the two sets match
exactly. An orphaned test file — physically present, linked from nowhere —
compiles into nothing, and its assertions silently never run; that gap is what
the third rule exists to catch.

The mirror is against the *linked* module, not the physical src/ tree, and
that difference is deliberate in roughly twenty places, not drift: a facade's
own verb is tested where the facade's link says it is, and a focused child
with no independent surface is exercised alongside it rather than carrying an
empty file of its own. `action/repo/pull/`'s `diagnosis.rs`, whose blockers are
observed through the sweep that reports them; `action/sync/`'s `repo.rs`,
`providers.rs`, and
`setup.rs`; `action/mcp/auth/`'s `preregister.rs` and `dispatch.rs`;
`action/feature/deliver/`'s `preview.rs` and `repos.rs`;
`action/feature/integrate/`'s focused children; `action/execute/`'s lifecycle files;
`harness/commands/catalog.rs`; `harness/config/mcp.rs`; `infra/fs/`'s `io.rs`,
`symlink.rs`, and `guard.rs`; `infra/proc/`'s `cwd.rs`; and `tui/scrollback.rs`
are all tested through the linked file of the module that declares them
(`sync/mod.rs`, `auth/mod.rs`, `deliver/mod.rs`, `integrate/mod.rs`,
`execute/mod.rs`, `commands.rs`, `config/mod.rs`, `fs/mod.rs`, `proc/mod.rs`,
`driver.rs`).
`action/hall/`'s five verb files predate this
pattern and share one file, `tests/unit/action/hall.rs`, for a different
reason: the facade there holds the shared discovery/read/prompt helpers, not
a re-export shell, so no per-verb split ever happened. A plain `mod.rs` that
is only `mod x; pub use x::*;` — most of the crate's — carries no test file at
all, because it has nothing in it that its children's tests do not already
cover.

`tests/unit/` is not a Cargo integration target: Cargo only auto-discovers
top-level `tests/*.rs` files, and there is deliberately no top-level
`tests/unit.rs`.

## Retained large files

The 300-line production review is a review trigger, not a rule — splitting a
coherent file to satisfy a number would optimise for counting rather than
responsibility. Files that remain above it are coherent single-responsibility
exceptions:

- `cli/root.rs` — the declarative Clap surface, one searchable command model.
- `cli/feature/lifecycle.rs` — declarative feature-lifecycle arguments beside
  their action-input conversions; delivery metadata and execution commands
  live in separate modules.
- `store/versioned/mod.rs` — the single documented versioning machine with its
  two policies (its Error was extracted to `error.rs`).
- `store/layout.rs` — every managed path is computed in one place.
- `store/manifest/model.rs` — the manifest's data types and every value
  invariant (`Manifest::validate`); reading/writing and error conversion were
  already split out to `persistence.rs` and `error.rs`.
- `bin/ivar.rs` — parse, dispatch, render, exit code; no domain logic.
- `error.rs` — the single output/error envelope (`Failure`, `Status`,
  `FixAction`, `Warning`, `Report`, style roles).
- `domain/name.rs` — one validation vocabulary with a common error model.
- `domain/feature/run.rs` — the Run Receipt vocabulary and lifecycle
  invariants in one coherent module.
- `domain/feature/integration.rs` — the pure nested-integration vocabulary in
  one place: via/strategy/policy resolution, receipts, and the evidence a
  receipt's trust rests on.
- `action/session/start.rs` — the session-start orchestration, kept one file by
  an explicit planning decision.
- `harness/commands.rs` and `harness/config/instructions.rs` — the reconciliation
  halves of their pairs; their declarative halves (`commands/catalog.rs`,
  `config/mcp.rs`) were extracted.
- `action/sync/mod.rs` — the public verb and report types, dispatching into
  `repo.rs`/`providers.rs`/`setup.rs`, which each stayed well under the
  trigger — a module facade whose capabilities genuinely live in focused
  child files.
- `action/feature/deliver/mod.rs` — restated honestly rather than filed under
  the facade above: `deliver()` itself (the preview/push/PR pipeline) stays
  inline here; `preview.rs` and `repos.rs` hold only the fingerprinted preview
  and repo-materialisation pieces it calls into. That inline apply pipeline is
  known debt, not a facade — splitting it further was not attempted in this
  pass.
- `action/feature/relations.rs` — the derived child-feature tree: parent-cycle
  validation, receipt freshness against live git, and the descendant blockers
  — one scan, not a helper reimplemented at each call site.
- `action/feature/mutation.rs` — the three scoped mutation guards (whole-child,
  structure, per-promotion) that keep a partial integration's planning mutable
  without freezing more than the receipt actually froze.
- `action/plan/status.rs`, `action/repo/remove.rs`,
  `action/session/conversion.rs`, `action/feature/delete.rs`, and
  `action/feature/promote.rs` — coherent command-level behaviors only modestly
  over the trigger.
- `action/plan/approve.rs` — no longer "modestly over": one gate verb that must
  hold the whole refusal surface in view. Approving a plan validates the
  frontmatter's declared repos against the manifest and resolves every shell
  command the plan's fenced blocks would run against those repos' real trees.
  Both refusals are the same decision — "this plan cannot be executed as
  written" — and splitting them would put half the answer a step away from the
  other half.
- `action/feature/cleanup.rs` — the preview and the apply of one verb, kept
  together because apply is defined by re-deriving the preview and comparing
  its fingerprint: the two halves must read the same facts the same way, and a
  split invites a second, drifting way to gather them.
- `action/repo/pull/mod.rs` — the one place a repo's refresh policy lives: the
  per-repo fetch step, the default-branch refresh, and the `refresh_all` sweep
  used by session startup. It crossed the trigger when that sweep replaced the
  copies callers previously carried.
- `git/mod.rs` — the `Git` trait and the real implementation dispatching
  across `read.rs`/`exec.rs`; one file so a caller sees one seam and never
  which backend answered.
- `git/exec.rs` — the single home for all mutations performed through the Git
  executable, plus six pure local reads kept here rather than in `read.rs`
  (named in the git/ module map above).
- `infra/proc/mod.rs` — the subprocess boundary's shared reasoning and its two
  stateless calls, `capture` and `inherit`; the one stateful, resumable call
  was carved out to `streaming.rs`.
- `tui/driver.rs` — seam 6: every byte of PTY/event I/O behind explicit step
  methods, spawning no executor of its own; the scrollback decoder was carved
  out to `scrollback.rs`.

There is no permanent lint for 300 lines. Adding one would make generated,
declarative, and inherently cohesive files optimise for counting.
