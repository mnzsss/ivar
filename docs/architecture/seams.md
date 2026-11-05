# The seams that carry the design

Part of the [architecture overview](../../ARCHITECTURE.md).

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
per repo (`repo pull` and the Smart Fetch inside `session start`) says which one
it is on; nothing else does. Delivery events are the second non-outcome output: they go
through a delivery event sink carried on `Ctx`, `Silent` by default, and never print into
the outcome.

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

### 5. `providers` facade — closed provider-first dispatch with shared orchestration

The set of harnesses is known at compile time (`Provider::SELECTABLE`; `Provider::ALL`
is the subset that owns a config surface, and a hosted provider such as `t3` stays out
of it), so dispatch is a match over a closed enum via the `providers` facade
(`src/providers/mod.rs`), not an open vtable or direct cross-module coupling. Action
modules never import concrete provider modules (`claude_code`, `opencode`, `omp`, `t3`)
directly; all provider-native facts (launch argv, MCP schemas, hook scripts, guard
encodings, and auth broker interactions) are private to their respective provider
modules.

Capabilities are **explicit flags**, not inferred: `supports_resume`,
`supports_review`, and so on. A harness that cannot do something declares that,
rather than pretending and failing at spawn time.

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
  did it get here*. Run Receipts hold append-only execution evidence; do not
  smuggle history into the gates.
- **A derived value must be recomputed, not cached across a mutation.** The gate
  is read inside `deliver`, after the feature is read, and is not threaded in
  from a caller who might be holding a stale copy.

`plan_gate` rides inside the fingerprinted delivery preview rather than being
checked beside it, so crossing the gate after a preview reads as drift like any
other change. A preview taken before approval cannot be applied after it — the
human approves one state, and that state includes whether the plan was approved.

The three gates — requirements, analysis, and plan — authorize execution in that
order. An approved Plan's content fingerprint is the authorization that `execute
start` pins into the Run Receipt; editing any approved artifact invalidates its
downstream gates. This is planning state, not a fourth execution approval.

### 7b. Run Receipts: local evidence, provider-native coordination

`domain/feature/run.rs` is the provider-neutral Run Receipt aggregate; the
`action/execute/` files validate and transition it, and `store/feature/run.rs`
owns its paths and archive. This direction is intentional: domain types define
state and invariants without I/O, actions combine those types with session, plan,
and snapshot facts, and the store persists only the resulting receipt. No action
joins execution filenames itself.

A receipt pins an approved plan fingerprint and immutable baseline, then records
coordinator session/provider lineage, checkpoints, structured report, and exact
final snapshot evidence. `active`, `blocked`, and `diverged` hold the Feature's
single-Run lock; `succeeded`, `failed`, and `interrupted` are terminal and move
whole into the immutable archive. `feature close` refuses while that lock is
held, but preserves all receipt history.

The provider boundary is deliberately narrow. The interactive provider decides
how to decompose the approved Plan and coordinate native subagents. Ivar neither
spawns headless provider children nor parses their events, stores their
transcripts, maintains their task graph, or enforces per-subagent write
contracts. A later session may resume the same logical Run under another provider:
the receipt records ordered lineage, not an invented continuity of an opaque
provider conversation.

Run Receipts are local state and migrate on read. Historical execution records
are imported into receipts before receipt-aware actions read or mutate them. A
known completed outcome is preserved; every non-terminal historical record becomes
`interrupted`. This compatibility path is historical input, not an active
execution architecture.

### 7c. Nested subfeatures: derived trees, immediate parents, and durable receipts

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
explicit merge with `--match-head-commit`, observed to `MERGED`). Shared `gh`
execution and response decoding live in `action/feature/pull_requests/mod.rs`;
focused operation modules build the commands. Tests fake the executable,
never a trait.

Multi-repo integration is **partial, durable, and resumable**: each repo's
result is persisted as an `integration_receipt` the moment it lands — success and
failed post-parent evidence alike — and a rerun reuses fresh receipts,
re-verifies unchanged failed ones, and resumes the rest. The first receipt of any
kind freezes the child's relationship/base/policy and promotion membership;
a successful receipt freezes its promotion individually (even when it later goes
stale). The scoped guards in `action::feature::mutation` enforce these
boundaries — plan and receipt mutations stay legal during a partial integration, an
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

### 9. Memory pages: pure model, derived index

A page is a file, and the index in `memory.db` is a cache of it. Three layers keep
that true:

- `domain::memory_page` is pure: `PageKind`, `Tier`, `PageMeta`, the path rule and the
  validation. It reads no file and no YAML.
- `store::memory::page_source` parses front matter through `infra::frontmatter` and calls
  the domain; `store::memory::pages` writes and searches the `pages`, `pages_fts` and
  `page_links` rows. `ivar doctor` and the reindex share `validate_page`, so the two
  cannot disagree on what a valid page is.
- `action::memory::{reindex,query}` own the behaviour: the reindex refreshes the index from
  disk (before `query` and `status`, in `sync`, and on the MCP leader's tick) and never
  touches `observations`; the query fuses page FTS and one-hop link neighbours with the
  ranking constants of `domain::memory_rank`.

Because the index is derived, a corrupt or stale one is repaired by
`ivar memory reindex`, not migrated. See [ADR-0010](../adr/0010-pages-in-hall-docs.md).
