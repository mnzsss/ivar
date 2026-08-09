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
    root.rs        every command, one file: the 12 root entries and each group's
                   subcommands. One file because clap derive is declarative and
                   splitting it would only scatter the surface.

  action/          one function per leaf command. The unit of behaviour.
    hall.rs        init · status · doctor · cleanup · migrate
    sync.rs        sync — big enough to own a file: it is the only verb that
                   crosses repos, providers and the hall in one pass
    repo/          add · list · remove · pull · setup · upstream
    feature/       create · list · promote · demote · status · deliver · close ·
                   delete · rebase · review · view · prune
    execute/       feature execute: prepare · replan · ack · reconcile ·
                   approve · tick · guard_check · reply
    session/       start · connect · conversion · stop · prune · relay, plus
                   lookup (shared id-prefix resolution)
    plan/          create · list · show · approve · status
    provider/      add · list
    skill/         create · add · update · remove · detach · sync · status ·
                   list · doctor
                   (a directory per group once it passed a handful of verbs;
                   `hall` and `sync` stayed single files because they did not)

  domain/          pure types and invariants. No I/O, no git, no clap.
    name.rs        validated newtypes: HallName, RepoName, FeatureName, BranchName…
    feature.rs     features, promotion, plans, gates and the execution board
    session.rs     session state and identity
    provider.rs    which harnesses exist, and their capability flags
    health.rs      hall health derivation (uninitialized/operational/stale/degraded)
    skill.rs skill_sync.rs  skills, and the sync plan over them
    mcp.rs         MCP server definitions as data

  store/           on-disk persistence. Owns file layout, nothing else.
    versioned.rs   the version-detect / migrate / refuse-if-newer machine
    manifest.rs    ivar.json — committed, NEVER auto-migrates
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
    read.rs        git2: refs, HEAD, worktree list, ahead/behind, status, blobs
    exec.rs        the git binary: clone --bare, worktree add/rm, branch, fetch,
                   push, rebase, checkout
    credential.rs  git credential-helper protocol, for the token fallback

  harness/         provider adapters
    mod.rs         the Harness trait + closed enum dispatch + capability flags.
                   Claude Code and OpenCode are variants here, not files —
                   they differ by data (config path, argv, capabilities), and a
                   file each would have held a match arm and nothing more.
    config.rs      per-harness config materialisation (CLAUDE.md, AGENTS.md, MCP)
    commands.rs    embedded shipped-workflow catalog, reconciliation, integrity
                   inspection, legacy fingerprint cleanup. Owns only the
                   `ivar-*.md` files in each provider's command directory.
    commands/*.md  provider-neutral workflow sources, compiled into the binary
                   with include_str!. No command content or file reconciliation
                   lives in `bin/ivar.rs` or `action`.

  tui/             ratatui. Sync render, explicit drive.
    screen.rs      the Screen seam over vt100 — the emulator swap point
    widget.rs      pure deterministic projection of a snapshot into a Buffer
    driver.rs      all I/O: pty reads, resize, event folding. Owns no executor.
    key_router.rs  pure reducer: (mode, key) -> (mode, action)
    master_detail.rs feature view layout, and the one event loop in the crate

  infra/           adapters to the outside world
    fs.rs          the filesystem primitive set. Nothing else touches std::fs.
                   Includes the read-only guard: the recursive chmod that clears
                   a non-promoted worktree's write bits (`mode & !0o222`).
    json.rs        write_canonical — the ONLY on-disk JSON writer
    frontmatter.rs split + parse + emit YAML frontmatter. The YAML swap point.
    hash.rs        sha256 of a file, and of a tree
    proc.rs        subprocess spawn, capture, exit codes — and port attribution,
                   by reading /proc for a pid's listening sockets
    github.rs      the GitHub trait: gh -> token -> clean failure. Faked in tests.
    term.rs        colour, NO_COLOR, is-a-tty. Decides *whether* to colour.

  error.rs         Failure · Status · FixAction · Warning · Report · Palette.
                   Palette lives here because the layout of a failure does, and
                   colour must not become a second copy of that layout.
```

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
    setups/<repo>.sh      per-repo setup scripts
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
| `IVAR_HALL` | always | the hall root |
| `IVAR_REPO` | in a repo worktree | the repo name, as it appears in `ivar.json` |
| `IVAR_BRANCH` | in a repo worktree | the branch that worktree is checked out on |
| `IVAR_WORKTREE` | in a repo worktree | the absolute path of the worktree (also the cwd) |
| `IVAR_WORKTREE_KIND` | always | `default` or `feature` |
| `IVAR_FEATURE` | feature worktrees only | the feature name |
| `IVAR_SESSION_ID` | inside a session | the session id |
| `IVAR_SESSION_PATH` | inside a session | the view dir |

The three worktree variables were added when slice 2 landed the setup-script
runner: a script's whole job is to bootstrap *this* repo on *this* branch, and
neither fact was derivable. `IVAR_WORKTREE` duplicates the working directory on
purpose — a script that `cd`s somewhere still needs a way back.

The prefix is `IVAR_`, with **no fallback to the old `BIFROST_` names**. Same
reasoning as the manifest: this implementation is new, nobody outside the private
monorepo ever had the old prefix, and being born with compatibility debt it never
incurred is how a tool accumulates two names for everything.

`IVAR_SESSION_ID` is load-bearing beyond bookkeeping — it is what a setup script
derives a per-session database or compose project name from, which is the only
answer `ivar` offers to shared daemon state. See
`docs/reference/limitations.md`.

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
