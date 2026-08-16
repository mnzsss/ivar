# Architecture

How `ivar` is put together, and the rules that keep it that way. For *why* each
dependency was chosen, see [ADR-0001](docs/adr/0001-stack-and-tooling.md).

## The one-sentence model

`ivar` assembles a directory. Everything else serves that: a **Hall** owns N
**Repos** as bare clones; a **Feature** is one branch across the repos it has
**Promoted**; a **Session** materialises a **View Dir** of symlinks into exactly
those worktrees and opens a harness in it. The repos a feature has not promoted
have their worktree root held read-only by the kernel.

Two properties fall out of that and constrain every module below.

**The work does not live inside a vendor.** Hall, feature, branch, promoted repos,
view dir and plan are files on disk and commits in git. A session dying loses the
conversation and nothing else. So: no state may exist only in a running process,
and no verb may require a live session to be useful.

**Read-only is a filesystem guarantee, not a harness one.** Non-promoted worktrees
have their write bits cleared (`mode & ~0o222`) on the worktree root — one path,
never the tree below it, for a reason `docs/reference/limitations.md` spells out.
Harness hooks are the *error message* that names the way out — `ivar feature promote` — never the barrier. So:
supporting a new harness is never blocked on whether it exposes a pre-tool hook.

## Module map

Describes the tree as it is. When a file moves, this moves with it — a map that
has drifted is worse than none, because it is the thing a newcomer trusts.

```
src/
  bin/ivar.rs      entrypoint: parse argv, dispatch, render, set exit code. No logic.
  lib.rs           crate attrs, module tree, the layering rule as a doc comment.

  cli/             clap derive types ONLY — structs, enums, doc comments.
    root.rs        every command, one file: the root entries and each group's
                   subcommands. One file because clap derive is declarative and
                   splitting it would only scatter the surface.

  action/          one function per leaf command. The unit of behaviour.
    hall/          init · status · doctor · migrate · cleanup — one file per
                   verb (init.rs, status.rs, doctor.rs, migrate.rs, cleanup.rs);
                   the facade owns the shared discovery/read/prompt helpers.
    sync/          sync — big enough to own a directory: the public verb and
                   report types live in mod.rs, repo materialisation in
                   repo.rs, provider/config reconciliation in providers.rs,
                   and setup-script execution in setup.rs.
    repo/          add · list · remove · pull · setup · upstream
    feature/       create · list · promote · demote · status · reparent ·
                   close · delete · rebase · review · view · prune, plus
                   deliver/ — the preview fingerprint, push, and pull-request
                   phases split across mod.rs, repos.rs, and preview.rs — and
                   integrate/ — the receipt-driven orchestration (preflight,
                   reuse/re-verify/resume, close-on-integrated) in mod.rs,
                   with the git-and-forge plumbing (local candidate staging,
                   the PR path, and receipt persistence) in apply.rs. The
                   nested-subfeature machinery lives in private focused
                   modules: relations.rs (the child-derived tree, receipt
                   freshness, and descendant blockers), lifecycle.rs (the
                   shared plan-frontmatter close seam), mutation.rs (the
                   scoped whole-child/per-promotion mutation guards),
                   verification.rs (the ordered executable checks),
                   reparent.rs, and pull_requests.rs (the shared PR operations
                   delivery and integration both use).
    execute/       feature execute: prepare · replan · ack · reconcile ·
                   approve · guard_check · reply, plus inbox (both ends of the
                   human-reply channel, written by reply and read back into
                   the prompt by tick) and tick/ — the launch orchestration in
                   mod.rs, the per-workstream worker thread (session
                   materialisation + spawn + stream drain) in launch.rs, and
                   the event-folding onto the board in events.rs. Shared
                   internals: plan_ops (the Operations parser both prompt and
                   replan use) and targeting (session-provider resolution at
                   prepare)
    session/       start · connect · conversion · stop · prune · relay, plus
                   view (the shared View Dir materialisation: repo symlinks,
                   per-session harness config, the projected plan, the
                   bootstrap instructions) and
                   lookup (shared id-prefix resolution)
    plan/          create · list · show · approve · status
    provider/      add · list
    skill/         create · add · update · remove · detach · sync · status ·
                   list · doctor

  domain/          pure types and invariants. No I/O, no git, no clap.
    name.rs        validated newtypes: HallName, RepoName, FeatureName, BranchName…
    feature/       a facade over seven focused files: feature.rs (the
                   promotion record and FeatureBoard — declared here as
                   `promotion`, via `#[path]`, because a module cannot share
                   the name of the directory that contains it), delivery.rs
                   (guards and the delivery preview), approval.rs (the SPDD
                   gates), execution.rs (the execution board and workstream
                   graph), write_contract.rs (the glob-matching write
                   contract each workstream must respect, split out of
                   execution.rs because it touches no board, status or
                   journal), integration.rs (the pure nested-integration
                   vocabulary: via/strategy/policy resolution, receipts and
                   verification evidence, and the derived integration-state
                   classifier), and base.rs (effective_base, the
                   declared-base-or-default-branch fallback a promotion
                   resolves against). Children are derived by scanning
                   `Feature.parent` — no feature stores a child list, and no
                   lifecycle field is persisted.
    session.rs     session state and identity
    provider.rs    which harnesses exist, and their capability flags
    health.rs      hall health derivation (uninitialized/operational/stale/degraded)
    skill.rs skill_sync.rs  skills, and the sync plan over them
    mcp.rs         MCP server definitions as data

  store/           on-disk persistence. Owns file layout, nothing else.
    versioned/     the version-detect / migrate / refuse-if-newer machine in
                   mod.rs, with its Error and Failure conversion in error.rs
    manifest/      ivar.json — committed, NEVER auto-migrates. model.rs owns
                   the data and invariants, persistence.rs the read/write/plan/
                   migrate, error.rs the Error and its Failure conversion.
    layout.rs      every path under a hall is computed here, nowhere else —
                   including the canonical `HALL.md` and each provider's
                   root alias (`CLAUDE.md` / `AGENTS.md`)
    gitignore.rs   the hall's .gitignore: append the needed lines, never clobber
    setup_receipt.rs  what a worktree's setup script did last time. The one file
                   NOT under layout: it lives in git's admin dir, so it dies
                   with the worktree it describes.
    feature.rs session.rs  the per-feature and per-session state files
    skill.rs       the lockfile and skill state
    render.rs      materialising a skill: symlink or copy, and verify

  git/             the only module that knows git exists
    mod.rs         the Git trait, and the real implementation over the two below
    error.rs       git::Error and its Failure conversion
    read.rs        git2: refs, HEAD, worktree list, ahead/behind, status, blobs
    exec.rs        the git binary: clone --bare, worktree add/rm, branch, fetch,
                   push, rebase, checkout — plus six pure local reads
                   (worktree_dirty, changed_paths, head_commit,
                   paths_committed_since, diff_worktree, commits_ahead) that
                   stay here rather than in read.rs. The ADR-0001 §3 split
                   ("reads go through git2, writes and network go through the
                   binary") is a rule with this one named exception: each of
                   these six parses `git`'s porcelain -z output, which is far
                   cheaper than the equivalent git2 walk for a plain local
                   read, and none of them touches a remote.
    credential.rs  git credential-helper protocol, for the token fallback

  harness/         provider adapters
    mod.rs         the Harness trait + closed enum dispatch + capability flags.
                   Claude Code and OpenCode are variants here, not files —
                   they differ by data (config path, argv, capabilities), and a
                   file each would have held a match arm and nothing more.
    commands.rs    the shipped-command reconciliation: materialise/remove/
                   inspect against a session's harness config directory. It
                   is the module file *for* `commands/` — a sibling of the
                   directory, not a file inside it — where `commands/catalog.rs`
                   owns ShippedCommand + the COMMANDS data and the *.md
                   sources live, compiled in with include_str!. No command
                   content or file reconciliation lives in `bin/ivar.rs` or
                   `action`.
    config/        per-harness config materialisation: instructions.rs owns
                   the canonical `HALL.md` managed block and the provider
                   root aliases (relative symlinks to `HALL.md`); mcp.rs owns
                   the MCP document construction and the Claude/OpenCode
                   translation; session.rs builds the session bootstrap block.
    guard/         the per-session execution guard materialisation: the
                   dispatch and shared constants in mod.rs, the Claude Code
                   hook script + settings.json merge in claude.rs, and the
                   OpenCode plugin in opencode.rs
    stream.rs      provider-shaped JSON in, ExecutorEvent out: the one place
                   a `claude -p --output-format stream-json` or `opencode
                   run` line is parsed, so a provider's envelope shape can
                   change without anything above this file noticing.

  tui/             ratatui. Sync render, explicit drive.
    screen.rs      the Screen seam over vt100 — the emulator swap point
    widget.rs      pure deterministic projection of a snapshot into a Buffer
    driver.rs      all I/O: pty reads, resize, event folding. Owns no executor.
    scrollback.rs  the plain-text scrollback decoder driver.rs feeds: escape
                   sequences stripped from PTY bytes, kept off the emulator
                   seam so a vt100 swap never has to think about it
    pty.rs         the concrete PtsPty adapter over portable-pty, behind the
                   Pty trait the driver is generic over
    key_router.rs  pure reducer: (mode, key) -> (mode, action)
    master_detail.rs feature view layout, and the one event loop in the crate

  infra/           adapters to the outside world
    fs/            the filesystem primitive set. Nothing else touches std::fs.
                   io.rs owns reads/writes/directories/metadata/removal,
                   symlink.rs owns create/replace/read plus SymlinkTarget,
                   guard.rs owns the read-only guard (chmod, mode, write bits),
                   and the facade owns the shared Error.
    json.rs        write_canonical — the ONLY on-disk JSON writer
    frontmatter.rs split + parse + emit YAML frontmatter. The YAML swap point.
    hash.rs        sha256 of a file, and of a tree
    proc/          subprocess spawn: `capture` and `inherit` in mod.rs, the
                   incremental line-protocol runner (`stream`/`Stream`) a
                   provider process needs in streaming.rs, and the Linux
                   /proc port attribution in ports.rs.
    github.rs      GitHub token lookup (and the credential-helper wiring that
                   derives it from `gh`/`$GITHUB_TOKEN` on each call). PR
                   operations themselves are not a trait seam: tests fake the
                   `gh` executable on PATH (see tests/support/fake_gh.rs), and
                   the one `gh` construction site is
                   `action/feature/pull_requests.rs`.
    term.rs        colour, NO_COLOR, is-a-tty, width. Decides *whether* to
                   colour.
    progress.rs    the transient stderr line a long verb reports through.
                   Silent by default; bin/ivar.rs is the only thing that
                   builds a live one. Never part of an Outcome.

  error.rs         Failure · Status · FixAction · Warning · Report · Palette.
                   Palette lives here because the layout of a failure does, and
                   colour must not become a second copy of that layout.
```

## Test layout

Every Rust test body lives under `tests/`, never under `src/`. There are two
compilation boundaries, and the layout keeps them distinct:

```
tests/
  architecture.rs           the layering rule and the centralization invariant
  delivery.rs init.rs …     top-level integration targets, discovered by Cargo
                            and compiled as their own crates
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
things, and `tests/architecture.rs` enforces all three: the layering scan walks
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
empty file of its own. `action/sync/`'s `repo.rs`, `providers.rs`, and
`setup.rs`; `action/feature/deliver/`'s `preview.rs` and `repos.rs`;
`action/feature/integrate/`'s `apply.rs`; `action/execute/tick/`'s `launch.rs`
and `events.rs`; `harness/commands/catalog.rs`; `harness/config/mcp.rs`;
`harness/guard/claude.rs` and `opencode.rs`; `infra/fs/`'s `io.rs`,
`symlink.rs`, and `guard.rs`; `infra/proc/`'s `ports.rs` and `streaming.rs`;
and `tui/scrollback.rs` are all tested through the linked file of the module
that declares them (`sync/mod.rs`, `deliver/mod.rs`, `integrate/mod.rs`,
`tick/mod.rs`, `commands.rs`, `config/mod.rs`, `guard/mod.rs`, `fs/mod.rs`,
`proc/mod.rs`, `driver.rs`). `action/hall/`'s five verb files predate this
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
- `store/versioned/mod.rs` — the single documented versioning machine with its
  two policies (its Error was extracted to `error.rs`).
- `store/layout.rs` — every managed path is computed in one place.
- `store/manifest/model.rs` — the manifest's data types and every value
  invariant (`Manifest::validate`); reading/writing and error conversion were
  already split out to `persistence.rs` and `error.rs`.
- `bin/ivar.rs` — parse, dispatch, render, exit code; no domain logic.
- `error.rs` — the single output/error envelope (`Failure`, `Status`,
  `FixAction`, `Warning`, `Report`, `Palette`).
- `domain/name.rs` — one validation vocabulary with a common error model.
- `domain/feature/execution.rs` — the execution board's status/journal
  invariants and the plan-derived workstream graph. It has no child modules
  and no `pub use`, so — unlike the facades below — it is not a facade; it is
  simply a coherent file that stayed over the trigger after `write_contract.rs`
  was carved out of it.
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
- `action/feature/integrate/mod.rs` and `apply.rs` — a facade split, but an
  honest one: unlike `deliver/`, both halves stayed over the trigger, because
  the orchestration state machine (preflight, reuse/re-verify/resume,
  close-on-integrated) and the git-and-forge plumbing it calls (the local and
  PR apply paths, and receipt persistence) are each real weight, not a thin
  dispatcher over thin children.
- `action/feature/relations.rs` — the derived child-feature tree: parent-cycle
  validation, receipt freshness against live git, and the descendant blockers
  — one scan, not a helper reimplemented at each call site.
- `action/feature/pull_requests.rs` — the one `gh` construction site delivery
  and nested integration both call; splitting it would risk a second PR
  command shape drifting from this one.
- `action/feature/mutation.rs` — the three scoped mutation guards (whole-child,
  structure, per-promotion) that keep a partial integration's plan and board
  mutable without freezing more than the receipt actually froze.
- `action/plan/status.rs`, `action/plan/approve.rs`,
  `action/repo/remove.rs`, `action/session/conversion.rs`,
  `action/feature/delete.rs`, `action/feature/promote.rs` — coherent
  command-level behaviors only modestly over the trigger.
- `action/execute/tick/mod.rs` — the tick orchestration: the module doc's
  concurrency contract, the public inputs/outcomes, and the single `tick()`
  that fans out and folds; its worker and event-folding halves were extracted
  to launch.rs and events.rs.
- `action/execute/tick/launch.rs` — the worker half `tick()` fans out to: one
  workstream's session materialisation, spawn, and stream drain, kept whole
  because a worker thread's steps do not compose usefully split across files.
- `action/execute/plan_ops.rs` — the one `## Operations` parser `prompt` and
  `replan` both call; a copy in each would let the two forks drift on what
  counts as a heading, a bullet, or the write-contract marker.
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


## Layering

Dependencies point downward only. A test over `use` statements enforces this —
a convention nobody remembers is not a boundary.

```
cli  ─────────────► action ─────────────► domain
                      │                     ▲
                      ├──► store ───────────┤
                      ├──► git ─────────────┤
                      ├──► harness ─────────┤
                      ├──► tui ─────────────┤
                      └──► infra ◄──────────┘   (store/git/harness/tui use infra)
```

| module | may import | may **not** import |
| --- | --- | --- |
| `cli` | `action`, `error` | everything else |
| `action` | anything below | `cli` |
| `domain` | `error` only | `store`, `git`, `harness`, `tui`, `infra` |
| `store` | `domain`, `infra`, `error` | `action`, `git`, `harness`, `tui` |
| `git` | `infra`, `error` | `action`, `store`, `domain`, `harness`, `tui` |
| `harness` | `domain`, `infra`, `error` | `action`, `store`, `git` |
| `tui` | `domain`, `infra`, `error` | `action`, `store`, `git`, `harness` |
| `infra` | `error` | everything else |

Two of these earn their strictness:

**`domain` is pure.** It holds the types and the invariants — what a valid
promotion is, how hall health is derived, which branch a repo resolves to. Being
pure is what makes those testable without a temp directory, and what stops the
rules from scattering into the verbs.

Two clocks are the named exception: `domain::feature::execution` and
`domain::session` each call `std::time::SystemTime::now()` directly, because
`JournalEntry::timestamp` (and its session equivalent) is a plain `String` for
exactly this reason — the value is written once, at construction, and nothing
in `domain` reads it back as a clock, so routing it through `store` and back
would cost a conversion at both ends for no invariant gained. `tests/architecture.rs`'s
layering scan only walks `use` statements, so a fully-qualified
`std::time::SystemTime::now()` call is invisible to it; this exception is
enforced by review, not by the test.

**`tui` cannot reach `action` or `store`.** State is pushed *into* the driver by
the host loop; the driver never fetches. This is what makes `widget.rs` a
referentially transparent projection: two renders of the same snapshot produce
byte-identical cells, so it can be tested headless against a `TestBackend` with no
clock and no I/O.

## The seams that carry the design

### 1. `action` is the unit, and it has one output shape

Every leaf command is one function:

```rust
fn promote(ctx: &Ctx, input: PromoteInput) -> Result<Report<PromoteOutcome>, Failure>
```

`Outcome` types are `Serialize`. `--json` prints the outcome; the human surface
formats the *same value*; the TUI renders the *same value*. There is no second
code path that computes what to show, so the surfaces cannot drift.

The one thing an action emits that is *not* in its outcome is a progress line,
and the shape is what keeps that from becoming a second output path: it is
written through a `Progress` sink carried on `Ctx`, it is transient (erased
before the outcome is rendered), it never appears under `--json`, and it
defaults to `Silent` — which is what every test sees, so an action is still
observed only through what it returns. A verb that costs a network round trip
per repo (`repo pull`, and the Smart Fetch inside `session start` and `execute
tick`) says which one it is on; nothing else does.

`Report<T>` carries `Vec<Warning>` alongside the value. A verb crossing eight
repos where one has uncommitted changes returns seven successes and one warning —
it does not abort the batch, and it does not swallow the problem either.

`Failure` is the envelope from ADR-0001: `Blocked` (refused before starting,
nothing happened) versus `Failed` (broke mid-flight), plus ordered `fix_actions`
each marked `safe` or not. That flag is the whole point — it is what lets an agent
recover on its own without being handed permission to force-push.

### 2. `store::versioned` — one machine, two policies

Every aggregate state file carries a schema version. One generic store does
detection (absent means v0), ordered migrations, and a hard refusal when the data
is *newer* than the binary understands, telling the user to upgrade.

The policy split is not cosmetic:

| file | policy |
| --- | --- |
| local state (`.ivar/state.json`, lockfiles) | migrates on read, silently. Nobody sees it. |
| **`ivar.json`** | **never rewrites itself.** Reads old versions fine; writing the new format requires an explicit `ivar migrate`. |

Because `ivar.json` is committed and team-shared. If upgrading `ivar` silently
rewrote it, that becomes a commit, and a teammate's older binary refuses the
commit. One person's upgrade would break someone else's repository.

The published promise is exactly one sentence: *there will never be a hall you
cannot open.* Not "we won't break the format" — at `0.x` we will. It is that every
format change ships its migration and the chain is never pruned.

A chain begins at the *earliest version its format supports*, not necessarily at
v0: `ivar.json`'s first public version is 1, so its chain is `[1→2]` and a v0
(unversioned) file is refused as unreachable rather than adopted. The generic
store's `has_migration_path` is the single answer to "can this file reach the
current version?".

`feature.json` is local state and migrates itself on read (now v3, adding
`parent`, the feature's `integration` override, and per-promotion
`integration_receipt`); `ivar.json` is committed and explicit (now v2, adding
hall `integration` defaults and each repo's ordered `checks`).

### 3. `infra::json::write_canonical` — the single writer

Sorted keys, two-space indent, LF, trailing newline. Nothing else writes JSON to
disk.

This exists because the spike's first differential run failed 13 of 13 cases with
semantically identical output — TypeScript emitted keys in spread order, `serde`
in struct-field order. Two consequences, both real: golden vectors shared with the
surviving TypeScript package can only be compared byte-for-byte if both sides
canonicalise, and a user's hall would otherwise churn in git every time a
different writer touched a file.

### 4. `git::Git` — one trait, two backends

Reads go through `git2`; mutations and anything touching a remote shell out to
`git`. Callers see one trait and cannot tell. The rule and its reasoning are in
ADR-0001 §3.

Tests never mock this. `tempfile::TempDir` plus a real `git init` is fast,
hermetic, and tests the thing that actually ships.

### 5. `harness::Harness` — trait plus closed enum

The set of harnesses is known at compile time, so dispatch is a match over a
closed enum, not a vtable. Each variant owns its command construction, its log
normalisation, and its config file shape — MCP configuration differs per harness
and is mapped centrally.

Capabilities are **explicit flags**, not inferred: `supports_resume`,
`supports_review`, and so on. A harness that cannot do something declares that,
rather than pretending and failing at spawn time. There is a `raw` escape hatch at
both ends.

### 6. `tui`: pure widget, explicit driver

`widget.rs` never awaits, never opens anything, never reads the clock. `driver.rs`
owns every byte of I/O and exposes explicit step methods the host loop calls —
`refresh`, `apply_event`, `apply_output_chunk` — and spawns no background tasks.

The spike found that the ANSI serialisation layer the TypeScript needed (162
lines) exists only because `ink` accepts strings. In `ratatui` you own the cells,
so a `vt100` cell maps straight to a `Style` and the entire layer disappears. Do
not reintroduce it.

### 7. Lifecycle is derived from the gates, never stored

A feature has no `lifecycle` field, and will not get one. Where it is in the SPDD
cycle is **read from the approvals artifact** — `ApprovalState` in
`features/<feature>/planning/approvals.json` — plus its promotions and the state
of its branches. `feature deliver` conditions apply on `Gate::Plan` being
`Approved`, reading it at the moment it needs it.

The alternative was a stored enum with explicit transitions, which is what the
TypeScript predecessor does: eleven states, a `transitionLifecycle` call at each
edge, and `deliver` refusing anything but `delivery_approved`. It shipped with a
state nothing wrote. The approve commands recorded their gate in the approvals
artifact and never touched the enum, so every feature stayed in `draft` — the
three gates green, delivery unreachable, and the only way through was hand-editing
the JSON.

The defect is not that someone forgot a call. It is that a stored lifecycle is a
second copy of a fact the gates already hold, and two copies of a fact drift.
Derivation removes the class: there is no transition to omit when there is no
transition, and no state that exists on disk but is reachable by nothing.

What this costs, stated plainly:

- **Reading the state costs a file read**, at every point that wants it. Cheap,
  local, and the artifact was already being read by `plan status`.
- **There is no history.** Derived state answers *where is this now*, never *when
  did it get here*. The execution board's journal is where anything append-only
  belongs; do not smuggle history into the gates.
- **A derived value must be recomputed, not cached across a mutation.** The gate
  is read inside `deliver`, after the feature is read, and is not threaded in
  from a caller who might be holding a stale copy.

`plan_gate` rides inside the fingerprinted delivery preview rather than being
checked beside it, so crossing the gate after a preview reads as drift like any
other change. A preview taken before approval cannot be applied after it — the
human approves one state, and that state includes whether the plan was approved.

### 7b. Nested subfeatures: derived trees, immediate parents, and durable receipts

A child stores exactly one fact about the lineage — `parent` — in its
`feature.json`. Everything else is derived by `action::feature::relations`
scanning every feature record: children, depth, the derived integration state,
and the descendants that block a parent's integration or delivery. The tree is
validated on every read (a missing parent or a cycle is a hard, non-mutating
refusal), and no parent ever stores a child list.

The direction of integration is always **up to the immediate parent's branch**:
`feature integrate <child>` refuses roots, refuses anything with a blocking
descendant (leaves first), and moves each promoted repo onto the parent's branch
via `local` (a detached candidate is built and checked before the parent moves)
or `pr` (push, create/reuse a PR against the parent's branch, required checks,
explicit merge with `--match-head-commit`, observed to `MERGED`). The one `gh`
construction site is `action/feature/pull_requests.rs`; tests fake the `gh`
executable, never a trait.

Multi-repo integration is **partial, durable, and resumable**: each repo's
result is persisted as an `integration_receipt` the moment it lands — success and
failed post-parent evidence alike — and a rerun reuses fresh receipts,
re-verifies unchanged failed ones, and resumes the rest. The first receipt of any
kind freezes the child's relationship/base/policy and promotion membership;
a successful receipt freezes its promotion individually (even when it later goes
stale). The scoped guards in `action::feature::mutation` enforce these
boundaries — plan/board mutations stay legal during a partial integration, an
unrestricted session cannot coexist with a successful receipt, and a fully
`integrated` close freezes the whole child with no reopen.

Pristine lineage changes (before any promotion, plan, execution, session,
receipt, close record, or descendant) go through the one explicit
`feature reparent` action, which rewrites `parent` and the derived `base`
together in a single canonical write. Child refs and worktrees are retained
after integration so receipt validation stays exact; cleanup remains explicit.

### 8. `cli` converts by destructuring, so a dropped flag will not compile

Every `*Args` struct in `cli::root` has exactly one `From<XArgs> for XInput`
impl, and every one of them opens with `let XArgs { .. } = args;` naming each
field. `bin/ivar.rs` is then pure dispatch: `action::verb(&ctx, args.into())`,
with no field list of its own.

The rule exists because the two failure directions are not symmetric:

- **An `Input` field the CLI never supplies** cannot happen. A struct literal in
  Rust must be exhaustive, so the compiler already refuses it.
- **An arg the parser declares and nothing reads** compiles fine. The field is
  simply never touched, `--help` promises the flag, and passing it does nothing.

That second one is the whole bug class. Exhaustive destructuring converts it into
`error[E0027]: pattern does not mention field`, naming the field, at the one site
that was supposed to forward it. No test can do this as well as the compiler, so
there is no test — do not replace the `let XArgs { … } = args;` lines with field
access to shorten them, or the guarantee silently goes away.

Nothing is validated here. Turning a `String` into a `FeatureName` needs
`domain`, which `cli` may not import; that stays the action's job.

## On-disk layout

One dotdir, one manifest, one name everywhere.

```
<hall>/
  ivar.json               the manifest. Committed — it is the identity file and
                          must be visible in review.
  HALL.md                 the canonical standing instructions. Committed, and
                          the sole editable source; the managed block is the
                          only part `ivar` owns.
  CLAUDE.md AGENTS.md     provider root aliases. Committed relative symlinks
                          to `HALL.md` — never sources, never edit targets.
  .ivar/                  everything the tool manages
    state.json            local hall state (gitignored)
    repos/<name>/.bare/   the bare clone; every checkout is a worktree off it
    repos/<name>/<branch>/
    features/<name>/      promotion records, execution board, session view dirs
    sessions/<uuid>/      discovery session view dirs
    secrets/              hand-maintained secret material (gitignored)
    setups/<repo>.sh      per-repo setup scripts
    setups/<repo>.session.sh  per-repo session hooks
    skills/               hall-scoped skills (committed)
  plans/<feature>/        requirements.md · analysis.md · plan.md (committed)
  .claude/ .opencode/     harness-dictated, and the TARGET of symlinks, not the source
  .claude/commands/ivar-*.md   derived workflow commands (gitignored)
  .opencode/commands/ivar-*.md derived workflow commands (gitignored)
```

A feature-session view dir is a real directory at
`<hall>/.ivar/features/<feature>/sessions/<uuid>/` containing: one symlink per
registered repo (feature worktree if promoted, read-only default otherwise), a
real harness config dir for the session's own provider (`.claude/` or
`.opencode/`, with `commands/` symlinked back to the hall), the feature's plan
projected in (`plans/<feature>/` → `<hall>/plans/<feature>/`, so the agent
confined to the view dir can read and edit the artifacts), and the provider's
instruction file (`CLAUDE.md` / `AGENTS.md`) **derived from the canonical
`HALL.md`** — the session bootstrap block followed by the hall's standing
instructions, or the canonical content alone for a discovery session. The
plan link and the instruction file are per-session views, never copies — they
die with the view dir, and plan edits land in the hall. Every view dir
receives the instruction file, whether or not it is feature-bound, and the
file never comes from the root alias.

Two traps, both learned the hard way:

- `.gitignore` must be `.ivar/*` plus `!.ivar/skills/` plus `!.ivar/setups/`,
  never `.ivar/`. Both of those children are committed — the skills a team shares
  and the setup script each repo carries. Git does
  not re-include a child of an excluded directory, and the failure is silent.
- **No legacy manifest-name fallback.** The manifest has already been renamed once.
  A fresh implementation should not be born carrying compatibility debt it never
  had.

## Environment contract

Setup scripts and anything else `ivar` spawns get these, and they are a **public
contract** — a user's committed `.ivar/setups/<repo>.sh` breaks if they move.

| variable | set when | value |
| --- | --- | --- |
| variable | setup script | session hook | value |
| --- | --- | --- | --- |
| `IVAR_HALL` | ✓ | ✓ | the hall root |
| `IVAR_REPO` | ✓ | ✓ | the repo name, as it appears in `ivar.json` |
| `IVAR_BRANCH` | ✓ | ✓ | the branch that worktree is checked out on |
| `IVAR_WORKTREE` | ✓ | ✓ | the absolute path of the worktree (also the cwd) |
| `IVAR_WORKTREE_KIND` | ✓ | ✓ | `default` or `feature` |
| `IVAR_SECRETS_DIR` | ✓ | ✓ | `.ivar/secrets/` — hand-maintained, never written by `ivar` |
| `IVAR_FEATURE` | feature worktrees only | ✓ | the feature name |
| `IVAR_SESSION_ID` | — | ✓ | the session id |
| `IVAR_SESSION_PATH` | — | ✓ | the view dir |

The three worktree variables were added when slice 2 landed the setup-script
runner: a script's whole job is to bootstrap *this* repo on *this* branch, and
neither fact was derivable. `IVAR_WORKTREE` duplicates the working directory on
purpose — a script that `cd`s somewhere still needs a way back.

`IVAR_FEATURE` is set on the promote path and absent on the sync path, because
`sync` runs against the default worktree where there is no feature to name.
`IVAR_WORKTREE_KIND` is what a script branches on to know which it is in; a
script that reads `IVAR_FEATURE` unguarded should fail loudly on the default
worktree rather than quietly bootstrap the wrong thing.

`IVAR_SECRETS_DIR` points at a directory `ivar` creates and never writes into.
The tool holds no secrets — the same posture `domain::mcp` takes for MCP server
definitions. It lives under `.ivar/`, so the hall's `.gitignore` (`.ivar/*` plus
exactly two negations) covers it without a line of its own; that is the reason
for the location, not a coincidence of it.

The prefix is `IVAR_`, with **no fallback to the old `BIFROST_` names**. Same
reasoning as the manifest: this implementation is new, nobody outside the private
monorepo ever had the old prefix, and being born with compatibility debt it never
incurred is how a tool accumulates two names for everything.

### Two scripts, two lifetimes

`IVAR_SESSION_ID` is load-bearing beyond bookkeeping — it is what a hook derives
a per-session database or compose project name from, which is the only answer
`ivar` offers to shared daemon state. See `docs/reference/limitations.md`.

It reaches the **session hook**, not the setup script, and the split is forced
by the receipt. `.ivar/setups/<repo>.sh` bootstraps a *worktree*: a receipt in
the worktree's git admin directory makes it run about once, which is the only
reason `ivar sync` stays cheap enough that people keep running it. A session's
database has to come up every time a session opens, and several sessions can
share one promoted worktree — so a receipt keyed to the worktree would skip
exactly the runs that matter.

| | setup script | session hook |
| --- | --- | --- |
| file | `.ivar/setups/<repo>.sh` | `.ivar/setups/<repo>.session.sh` |
| runs on | `sync`, `promote`, `repo setup` | `session start` |
| how often | once per worktree, receipt-gated | once per session, ungated |
| failure | refuses on `sync`, warns on `promote` | warns; the session still opens |

Both are committed, both are per-repo, and both run under `bash` with the
worktree as cwd. Only the hook sees the session.

## Build order

Vertical slices — one full journey at a time, end to end — not layer by layer.
This is what the spike did, and it is why 51 of 52 differential cases passed on
first assembly instead of surfacing at integration.

```
1  ivar init                 store, manifest, layout, canonical json, error envelope
2  ivar sync                 harness config materialisation, setup scripts, git clone
3  ivar repo …               the Git trait both ways, worktrees, ro_guard
4  ivar feature …            promotion, branch resolution, typestate worktree creation
5  ivar session start        view dir, harness spawn, TUI, PTY, emulator
6  ivar plan …               SPDD artifacts, approval gates
7  ivar skill …              mostly ported already by the spike
8  status · doctor · cleanup · migrate   reconciliation and hygiene, informed
                             by everything above
```

Slices 1–2 are where the foundations get set, so they are the expensive ones:
after those, the error envelope, the warning discipline, the canonical writer and
the versioned store all exist and every later slice is cheaper than the last.

Three items are launch gates rather than polish, and they attach to specific
slices: port attribution (slice 5 — two repos and two dev servers collide on the
*first* session otherwise), the per-repo setup script (slice 2 — a git worktree
shares history but not untracked files, so a fresh worktree has no `.env` and no
`node_modules`), and testing the view dir against a third-party git TUI (slice 3 —
the closest prior art broke `lazygit` outright with output that was perfectly
legal).

## What is deliberately not here

- **No async runtime, no daemon, no server, no socket, no telemetry.** The
  local-only claim is verifiable by `rg` over this repo, and it should stay that
  way.
- **No MCP surface yet.** Deferred with the cost named: the closest prior art
  shipped 67 MCP tools and had to cut to 15 after finding the agent spent its
  context orchestrating instead of reading code. Marking a verb costs an
  annotation; unmarking 67 tools costs a month.
- **No container per environment.** `ivar` isolates the filesystem and ports. It
  does **not** isolate a daemon with shared state — a shared Postgres stays
  shared. The mechanism offered instead is the setup script hook plus a documented
  recipe, and the limitation is named on a page users find before they hit it.
- **No Windows.** The view dir is built entirely from symlinks, which need
  Developer Mode or admin rights. The answer is WSL, which consumes the Linux
  build unchanged.
