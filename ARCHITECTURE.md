# Architecture

How `ivar` is put together, and the rules that keep it that way. For *why* each
dependency was chosen, see [ADR-0001](docs/adr/0001-stack-and-tooling.md).

## The one-sentence model

`ivar` assembles a directory. Everything else serves that: a **Hall** owns N
**Repos** as bare clones; a **Feature** is one branch across the repos it has
**Promoted**; a **Session** materialises a **View Dir** of symlinks into exactly
those worktrees and opens a harness in it. The repos a feature has not promoted
are held read-only by the kernel.

Two properties fall out of that and constrain every module below.

**The work does not live inside a vendor.** Hall, feature, branch, promoted repos,
view dir and plan are files on disk and commits in git. A session dying loses the
conversation and nothing else. So: no state may exist only in a running process,
and no verb may require a live session to be useful.

**Read-only is a filesystem guarantee, not a harness one.** Non-promoted worktrees
have their write bits cleared (`mode & ~0o222`). Harness hooks are the *error
message* that names the way out — `ivar feature promote` — never the barrier. So:
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
    feature/       create · list · promote · demote · status · close · delete ·
                   rebase · review · view · prune, plus deliver/ — the preview
                   fingerprint, push, and pull-request phases split across
                   mod.rs, repos.rs, preview.rs, and pull_requests.rs.
    execute/       feature execute: prepare · replan · ack · reconcile ·
                   approve · guard_check · reply, plus tick/ — the launch
                   orchestration in mod.rs, the per-workstream worker thread
                   (session materialisation + spawn + stream drain) in
                   launch.rs, and the event-folding onto the board in events.rs
    session/       start · connect · conversion · stop · prune · relay, plus
                   lookup (shared id-prefix resolution)
    plan/          create · list · show · approve · status
    provider/      add · list
    skill/         create · add · update · remove · detach · sync · status ·
                   list · doctor

  domain/          pure types and invariants. No I/O, no git, no clap.
    name.rs        validated newtypes: HallName, RepoName, FeatureName, BranchName…
    feature/       a facade over four focused files: feature.rs (the promotion
                   record and FeatureBoard), delivery.rs (guards and the
                   delivery preview), approval.rs (the SPDD gates), and
                   execution.rs (the execution board and workstream graph)
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
    layout.rs      every path under a hall is computed here, nowhere else
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
                   push, rebase, checkout
    credential.rs  git credential-helper protocol, for the token fallback

  harness/         provider adapters
    mod.rs         the Harness trait + closed enum dispatch + capability flags.
                   Claude Code and OpenCode are variants here, not files —
                   they differ by data (config path, argv, capabilities), and a
                   file each would have held a match arm and nothing more.
    config/        per-harness config materialisation (CLAUDE.md, AGENTS.md, MCP).
                   mod.rs owns the managed-block behaviour; mcp.rs owns the MCP
                   document construction and the Claude/OpenCode translation.
    commands/      the shipped workflow catalog and reconciliation: catalog.rs
                   owns ShippedCommand + the COMMANDS data, commands.rs owns
                   materialise/remove/inspect. The *.md sources live here and
                   are compiled in with include_str!.
    commands/*.md  provider-neutral workflow sources, compiled into the binary
                   with include_str!. No command content or file reconciliation
                   lives in `bin/ivar.rs` or `action`.
    guard/         the per-session execution guard materialisation: the
                   dispatch and shared constants in mod.rs, the Claude Code
                   hook script + settings.json merge in claude.rs, and the
                   OpenCode plugin in opencode.rs

  tui/             ratatui. Sync render, explicit drive.
    screen.rs      the Screen seam over vt100 — the emulator swap point
    widget.rs      pure deterministic projection of a snapshot into a Buffer
    driver.rs      all I/O: pty reads, resize, event folding. Owns no executor.
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
    proc/          subprocess spawn, capture, exit codes in mod.rs; the Linux
                   /proc port attribution lives in ports.rs.
    github.rs      the GitHub trait: gh -> token -> clean failure. Faked in tests.
    term.rs        colour, NO_COLOR, is-a-tty. Decides *whether* to colour.

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
  unit/                     the unit-test tree, mirrored one-to-one against src/
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
things, and `tests/architecture.rs` enforces both: the layering scan walks
`tests/unit/<module>/` with the same allowed imports as `src/<module>/`, and a
second rule refuses any `#[test]`, `#[rstest]`, or inline `mod tests { … }`
body under `src/`.

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
- `bin/ivar.rs` — parse, dispatch, render, exit code; no domain logic.
- `error.rs` — the single output/error envelope (`Failure`, `Status`,
  `FixAction`, `Warning`, `Report`, `Palette`).
- `domain/name.rs` — one validation vocabulary with a common error model.
- `action/session/start.rs` — the session-start orchestration, kept one file by
  an explicit planning decision.
- `harness/commands.rs` and `harness/config/mod.rs` — the reconciliation halves
  of their pairs; their declarative halves (catalog.rs, mcp.rs) were extracted.
- `action/sync/mod.rs`, `action/feature/deliver/mod.rs`,
  `domain/feature/execution.rs` — module facades whose capabilities live in
  focused child files.
- `action/plan/status.rs`, `action/plan/approve.rs`,
  `action/repo/remove.rs`, `action/session/conversion.rs`,
  `action/feature/delete.rs`, `action/execute/replan.rs` — coherent command-level
  behaviors only modestly over the trigger.
- `action/execute/tick/mod.rs` — the tick orchestration: the module doc's
  concurrency contract, the public inputs/outcomes, and the single `tick()`
  that fans out and folds; its worker and event-folding halves were extracted
  to launch.rs and events.rs.
- `git/exec.rs` — the single home for all mutations performed through the Git
  executable.

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
