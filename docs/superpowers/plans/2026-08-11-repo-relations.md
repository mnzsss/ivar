# Repository Relations Journey Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a human-confirmed repository-relations workflow backed by canonical `HALL.md` instructions, safely maintained aliases, evidence-driven lifecycle reminders, and no machine-readable relation graph.

**Architecture:** Put canonical instruction and root-alias reconciliation in a focused `harness::config::instructions` module; `Layout` names canonical and provider-native paths, while actions only orchestrate and report. Keep relationship semantics entirely in shipped Markdown workflows: Rust maintains `HALL.md`'s existing managed block and aliases but never parses the separate relation region. Session view materialization reads `HALL.md` directly and writes provider-native ephemeral files.

**Tech Stack:** Rust 2024, existing `camino`, `serde`, `clap`, `infra::fs`, shipped Markdown command catalog, `rstest`, `tempfile`, real-Git integration helpers.

**Specification:** `docs/superpowers/specs/2026-08-11-repo-relations.md`

---

## Agreed behavior

1. `repo add` succeeds with structured `next_action: "/ivar-relations <repo>"`; no review state is persisted.
2. `/ivar-relations` is a provider-neutral, one-question-at-a-time workflow. Evidence produces proposals; only humans confirm prose.
3. `HALL.md` is the only editable root source. Enabled providers use relative `CLAUDE.md`/`AGENTS.md` symlinks.
4. Rust owns only the existing managed block; `/ivar-relations` owns its separate region. Rust never parses relation prose.
5. Enabled regular aliases are preserved for human adoption. Disabled alias paths are removed even when regular.
6. Init and provider add attempt instruction materialization immediately; sync is authoritative repair; doctor is read-only inspection.
7. Sessions read `HALL.md` directly and write real ephemeral native files. Missing canonical content warns but does not block.
8. Plan, execute, and deliver inspect relation context at defined checkpoints and prompt only with cited evidence.
9. `feature promote`, manifest schema, and delivery fingerprints do not change.

## File structure

### New files

- `docs/superpowers/specs/2026-08-11-repo-relations.md` — approved behavioral specification.
- `src/harness/config/instructions.rs` — canonical managed-block materialization, root-alias inspection, and reconciliation.
- `src/harness/commands/relations.md` — guided human-confirmed relation authoring and review.
- `tests/unit/harness/config/instructions.rs` — topology and byte-preservation unit tests.
- `tests/repo_relations.rs` — compiled-binary lifecycle coverage.

### Modified files

- `src/store/layout.rs` — distinct canonical and alias accessors.
- `src/harness/config/mod.rs` — expose the focused instructions module and retain MCP/session responsibilities.
- `src/harness/commands/catalog.rs` — fifteenth command and optional legacy fingerprint.
- `src/harness/commands.rs` — handle commands without a legacy fingerprint.
- `src/action/hall/init.rs` — immediate best-effort instruction bootstrap.
- `src/action/sync/providers.rs` — invoke canonical reconciliation once and keep MCP/commands per provider.
- `src/action/provider/add.rs` — immediate best-effort alias bootstrap.
- `src/action/hall/doctor.rs` — shared read-only instruction inspection.
- `src/action/repo/add.rs` — structured relations next action.
- `src/action/session/view.rs` — derive discovery and feature instruction files from `HALL.md`.
- `src/harness/commands/plan.md` — Analysis checkpoint.
- `src/harness/commands/execute.md` — terminal-board checkpoint.
- `src/harness/commands/deliver.md` — preview-to-apply checkpoint.
- `tests/unit/store/layout.rs`, `tests/unit/action/hall.rs`, `tests/unit/action/sync.rs`, `tests/unit/action/provider/add.rs`, `tests/unit/action/repo/add.rs`, `tests/unit/action/session/start.rs`, `tests/unit/action/session/connect.rs`, `tests/unit/action/session/conversion.rs`, `tests/unit/action/session/relay.rs`, `tests/unit/harness/commands.rs`, and `tests/unit/harness/config/` — focused TDD coverage.
- `ARCHITECTURE.md`, `docs/concepts.md`, `docs/getting-started.md`, `docs/glossary.md`, `docs/reference/on-disk-format.md` — public contracts.

---

### Task 1: Separate canonical and provider-native instruction paths

**Files:**
- Modify: `src/store/layout.rs`
- Modify: `tests/unit/store/layout.rs`

- [x] **Step 1: Write failing accessor tests**

Add tests beside the existing provider path coverage:

```rust
#[test]
fn hall_instructions_and_provider_aliases_are_distinct() {
    let layout = Layout::at(Utf8PathBuf::from("/hall"));

    assert_eq!(
        layout.hall_instructions(),
        Utf8PathBuf::from("/hall/HALL.md")
    );
    assert_eq!(
        layout.instruction_alias(&Provider::ClaudeCode),
        Utf8PathBuf::from("/hall/CLAUDE.md")
    );
    assert_eq!(
        layout.instruction_alias(&Provider::OpenCode),
        Utf8PathBuf::from("/hall/AGENTS.md")
    );
}
```

- [x] **Step 2: Run the focused test and verify failure**

Run:

```bash
cargo test --lib store::layout::tests::hall_instructions_and_provider_aliases_are_distinct
```

Expected: compile failure because the two new accessors do not exist.

- [x] **Step 3: Add exact accessors and remove the ambiguous API**

Add beside `commands_dir`:

```rust
/// `<hall>/HALL.md` — the sole editable source of shared hall instructions.
#[must_use]
pub fn hall_instructions(&self) -> Utf8PathBuf {
    self.root.join("HALL.md")
}

/// The provider-native root alias that must point relatively to `HALL.md`.
#[must_use]
pub fn instruction_alias(&self, provider: &Provider) -> Utf8PathBuf {
    self.root.join(provider.instruction_file())
}
```

Remove `instruction_file`. Let compilation failures enumerate every caller; do
not retain a compatibility alias that preserves the source/alias ambiguity.

- [x] **Step 4: Run layout tests**

Run: `cargo test --lib store::layout`

Expected: accessor tests pass; remaining compile errors name production callers
that Tasks 2 and 4 deliberately update.

- [x] **Step 5: Commit**

```bash
git add src/store/layout.rs tests/unit/store/layout.rs
git commit -m "refactor(layout): distinguish hall instructions from aliases"
```

### Task 2: Build one root instruction reconciler

**Files:**
- Create: `src/harness/config/instructions.rs`
- Create: `tests/unit/harness/config/instructions.rs`
- Modify: `src/harness/config/mod.rs`

- [x] **Step 1: Write failing canonical-content tests**

Cover these exact cases with a temporary hall root:

1. absent `HALL.md` becomes a regular file containing only the managed block;
2. user bytes without markers survive byte-for-byte after the prepended block;
3. an existing block is replaced without changing bytes before or after it;
4. a second call is `Unchanged` and does not rewrite the file;
5. non-regular `HALL.md` returns a typed conflict without touching its target.

Use the existing `build_block` output as the expected bytes; do not duplicate the
block text in the test.

- [x] **Step 2: Write failing alias topology tests**

For each provider, assert:

```text
enabled + absent                 -> Created relative symlink to HALL.md
enabled + correct symlink        -> Unchanged
enabled + broken/wrong symlink   -> Updated to HALL.md
enabled + regular file           -> Conflict and exact bytes preserved
disabled + absent                -> Unchanged
disabled + any symlink           -> Removed
disabled + regular file          -> Removed
```

Also assert that no result removes or rewrites `HALL.md`.

- [x] **Step 3: Run tests and verify failure**

Run: `cargo test --lib harness::config::instructions::tests`

Expected: compile failure because `instructions` and its result types do not
exist.

- [x] **Step 4: Define the focused API**

Move the managed-block content functions out of `config/mod.rs` and define:

```rust
pub const MANAGED_START: &str = "<!-- ivar:managed:start -->";
pub const MANAGED_END: &str = "<!-- ivar:managed:end -->";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Created,
    Updated,
    Removed,
    Unchanged,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: Utf8PathBuf,
    pub change: Change,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub path: Utf8PathBuf,
    pub integrity: Integrity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Integrity {
    Current,
    Missing,
    NotRegular,
    ManagedBlockMissing,
    ManagedBlockStale,
    AliasIsRegular,
    AliasBroken,
    AliasWrongTarget,
    DisabledAliasPresent,
}

pub fn build_block(hall: &HallName, repos: &[RepoName]) -> String;

pub fn reconcile(
    layout: &Layout,
    manifest: &Manifest,
) -> Result<Vec<Entry>, Error>;

pub fn inspect(
    layout: &Layout,
    manifest: &Manifest,
) -> Result<Vec<Inspection>, Error>;
```

`reconcile` must process `HALL.md` once, then `Provider::ALL`. Pass
`Utf8Path::new("HALL.md")` as the relative symlink target. Use
`replace_symlink_if_changed`, compare bytes before writes, preserve enabled
regular aliases, and call the existing filesystem removal primitive for every
disabled alias entry regardless of type.

- [x] **Step 5: Keep module responsibilities explicit**

In `config/mod.rs`, expose `pub mod instructions;`, keep `mcp` and `session` in
their current files, and re-export only names already relied on broadly. Root
alias filesystem behavior must live only in `instructions.rs`.

- [x] **Step 6: Run focused tests**

Run:

```bash
cargo test --lib harness::config::instructions
cargo test --lib harness::config
```

Expected: all managed-content and topology cases pass, including destructive
disabled-provider cleanup and enabled regular-file preservation.

- [x] **Step 7: Commit**

```bash
git add src/harness/config/mod.rs src/harness/config/instructions.rs tests/unit/harness/config/instructions.rs
git commit -m "feat(config): reconcile canonical hall instructions"
```

### Task 3: Wire init, sync, provider add, and doctor

**Files:**
- Modify: `src/action/hall/init.rs`
- Modify: `src/action/sync/providers.rs`
- Modify: `src/action/provider/add.rs`
- Modify: `src/action/hall/doctor.rs`
- Modify: `tests/unit/action/hall.rs`
- Modify: `tests/unit/action/sync.rs`
- Modify: `tests/unit/action/provider/add.rs`

- [x] **Step 1: Add failing init and provider-add tests**

Add assertions that:

- Claude init immediately creates regular `HALL.md` and relative `CLAUDE.md`;
- OpenCode init creates `AGENTS.md` and not `CLAUDE.md`;
- an occupied alias produces a warning while the manifest remains valid;
- adding OpenCode immediately creates `AGENTS.md` through the shared reconciler;
- provider-add conflict warns and keeps the provider persisted.

- [x] **Step 2: Add failing sync matrix tests**

Prove in separate tests:

1. sync repairs absent and wrong enabled symlinks;
2. sync preserves exact bytes of an enabled regular alias and reports conflict;
3. sync still reconciles repos, MCP, and commands after that conflict;
4. removing OpenCode manually from `providers.available` then syncing deletes a
   regular `AGENTS.md`;
5. repeated healthy sync reports unchanged and leaves file mtimes unchanged.

- [x] **Step 3: Add failing doctor tests**

Assert stable findings for canonical absence/non-regular state, stale block,
enabled alias absence/regular/broken/wrong target, and disabled alias presence.
One run must return every applicable finding.

Use these stable codes:

```text
instructions.canonical_missing
instructions.canonical_not_regular
instructions.managed_block_missing
instructions.managed_block_stale
instructions.alias_missing
instructions.alias_regular
instructions.alias_broken
instructions.alias_wrong_target
instructions.disabled_alias_present
```

- [x] **Step 4: Run tests and verify failure**

Run:

```bash
cargo test --lib action::hall
cargo test --lib action::sync
cargo test --lib action::provider::add
```

Expected: instruction artifacts and findings are absent.

- [x] **Step 5: Add one action-level warning adapter**

Create a small `pub(crate)` adapter beside provider sync orchestration that calls
`instructions::reconcile`, converts conflicts/errors to existing sync entries and
warnings, and is reused by init and provider add. Do not call the complete sync
action from either command.

Use warning codes:

```text
instructions.not_materialised
instructions.adoption_required
```

The adoption warning must name the regular alias and instruct the user to
consolidate it into `HALL.md`, remove it, rerun `ivar sync`, and inspect Git diff.

- [x] **Step 6: Wire actions without duplicating reconciliation**

- `init`: invoke after durable manifest/skeleton creation; warn, never rollback.
- `sync`: invoke once before or after the per-provider MCP/command loop; a
  conflict does not abort the loop.
- `provider add`: invoke after the manifest update; warn, never rollback.
- `doctor`: call `instructions::inspect` and map each non-current result to one
  diagnosis. Disabled alias findings explicitly say sync removes regular files.

- [x] **Step 7: Run focused tests**

Run:

```bash
cargo test --lib action::hall
cargo test --lib action::sync
cargo test --lib action::provider::add
```

Expected: all lifecycle, warning, destructive cleanup, and multi-finding tests
pass.

- [x] **Step 8: Commit**

```bash
git add src/action/hall src/action/sync src/action/provider tests/unit/action
git commit -m "feat(actions): maintain hall instruction topology"
```

### Task 4: Derive every session's native file from `HALL.md`

**Files:**
- Modify: `src/action/session/view.rs`
- Modify: `src/harness/config/session.rs`
- Modify: `tests/unit/action/session/start.rs`
- Modify: `tests/unit/action/session/connect.rs`
- Modify: `tests/unit/action/session/conversion.rs`
- Modify: `tests/unit/action/session/relay.rs`
- Modify: `tests/unit/harness/config/session.rs`

- [x] **Step 1: Write failing session materialization tests**

Cover:

1. discovery under Claude writes a real `CLAUDE.md` equal to `HALL.md`;
2. discovery under OpenCode writes a real `AGENTS.md` equal to `HALL.md`;
3. feature session prepends the exact bootstrap then two newlines then HALL bytes;
4. root alias bytes/target are irrelevant because materialization reads canonical;
5. connect-style rematerialization repairs modified ephemeral content;
6. unchanged content is not rewritten;
7. missing `HALL.md` lets discovery open with a warning and no shared content;
8. missing `HALL.md` lets feature open with bootstrap only and the same warning.

- [x] **Step 2: Run and verify failure**

Run: `cargo test --lib action::session::view`

Expected: discovery has no instruction file and production still reads the
provider alias.

- [x] **Step 3: Make materialization report warnings**

Change the shared view materializer's return to carry warnings without failing
the session:

```rust
pub(crate) struct MaterialiseReport {
    pub warnings: Vec<Warning>,
}

pub(crate) fn materialise(
    layout: &Layout,
    manifest: &Manifest,
    feature: Option<&Feature>,
    provider: Provider,
    view_dir: &Utf8Path,
) -> Result<MaterialiseReport, Failure>;
```

Use warning code `instructions.canonical_unavailable`. Thread the report through
start, connect, conversion, relay, and executor callers into their existing
`Report` warning surface. Do not add fallback reads.

- [x] **Step 4: Replace provider-alias reads**

Read `layout.hall_instructions()` once. For discovery, write those bytes to the
provider-native view path. For feature, write
`build_session_block(...) + "\n\n" + hall`. When canonical bytes are absent,
feature writes only the bootstrap and discovery writes no shared content.
Compare bytes before every write.

- [x] **Step 5: Run all session and execute-launch tests**

Run:

```bash
cargo test --lib action::session
cargo test --lib action::execute
```

Expected: all entry points use the same materializer, warnings are surfaced, and
reconnection remains idempotent.

- [x] **Step 6: Commit**

```bash
git add src/action/session src/action/execute src/harness/config/session.rs tests/unit/action/session tests/unit/action/execute tests/unit/harness/config/session.rs
git commit -m "feat(session): derive instructions from HALL.md"
```

### Task 5: Add the relations invitation and shipped workflow

**Files:**
- Modify: `src/action/repo/add.rs`
- Modify: `tests/unit/action/repo/add.rs`
- Create: `src/harness/commands/relations.md`
- Modify: `src/harness/commands/catalog.rs`
- Modify: `src/harness/commands.rs`
- Modify: `tests/unit/harness/commands.rs`

- [x] **Step 1: Write failing repo-add outcome tests**

Assert serialized and human surfaces share the same action:

```rust
assert_eq!(outcome.next_action, "/ivar-relations api");
assert!(human.contains("Next: run `/ivar-relations api`"));
```

Also assert the successful report has no warning or fix action and the manifest
schema remains version 1.

- [x] **Step 2: Add the exact outcome field**

Extend `AddOutcome`:

```rust
/// Provider-neutral guided follow-up for describing this repo in its hall.
pub next_action: String,
```

Set it only on successful completion with
`format!("/ivar-relations {name}")`. Render it from the outcome; do not compute a
second command string in `WriteHuman`.

- [x] **Step 3: Write failing command-catalog tests**

Change catalog expectations from 14 to 15, require unique `relations`, and assert
it has no legacy fingerprint while every prior entry retains its exact hash.

- [x] **Step 4: Make legacy fingerprints optional**

Change the type and all entries consistently:

```rust
pub struct ShippedCommand {
    pub id: &'static str,
    pub content: &'static str,
    pub legacy_sha256: Option<&'static str>,
}
```

Wrap all fourteen existing hashes in `Some(...)`; define `relations` with
`legacy_sha256: None`. In reconciliation, run legacy cleanup only inside
`if let Some(expected) = command.legacy_sha256`. Do not use an empty-string
sentinel.

- [x] **Step 5: Author `relations.md` from the specification**

The file must include:

```yaml
---
description: Review and maintain human-confirmed relationships between repositories in HALL.md.
argument-hint: [repo-name]
---
```

Then encode the exact entry modes, evidence rules, one-proposal-at-a-time flow,
soft limit of five, open question, final confirmation, canonical markers,
deterministic ordering, optional topic numbering, orphan handling, concurrent
re-read, and the prohibition on editing aliases/session files. The command must
say Rust does not validate the region and rejected proposals are not persisted.

- [x] **Step 6: Run focused tests**

Run:

```bash
cargo test --lib action::repo::add
cargo test --lib harness::commands
```

Expected: repo-add output includes the next action; all fifteen commands
materialize; legacy cleanup behavior for the original fourteen is unchanged.

- [x] **Step 7: Commit**

```bash
git add src/action/repo/add.rs src/harness/commands tests/unit/action/repo tests/unit/harness/commands
git commit -m "feat(relations): add guided repository context workflow"
```

### Task 6: Add evidence-driven lifecycle checkpoints

**Files:**
- Modify: `src/harness/commands/plan.md`
- Modify: `src/harness/commands/execute.md`
- Modify: `src/harness/commands/deliver.md`
- Modify: `tests/unit/harness/commands.rs`

- [x] **Step 1: Add failing content-contract tests**

Assert each embedded command contains its checkpoint and the common safeguards:

```text
plan     -> beginning of Analysis; read HALL.md and linked topics
execute  -> only after every workstream is terminal
deliver  -> after preview and before apply
all      -> cited evidence; offer /ivar-relations; never block or write directly
```

Also assert plan/execute/deliver Rust outcomes and schemas are unchanged.

- [x] **Step 2: Run and verify failure**

Run: `cargo test --lib harness::commands`

Expected: checkpoint phrases are absent.

- [x] **Step 3: Update `plan.md`**

At Analysis start, require reading `HALL.md`, selecting entries for potentially
affected repos, following only linked topics, and recording relevant context in
`analysis.md`. Offer `/ivar-relations` only when cited code evidence contradicts,
extends, or obsoletes the prose. Deferral cannot block approval.

- [x] **Step 4: Update `execute.md`**

After the existing terminal-state rule, inspect journal and produced changes once
after every workstream is succeeded or failed. Offer only with evidence; state
that relation review does not change board completion and is not replan/reconcile.

- [x] **Step 5: Update `deliver.md`**

Between preview and apply, focus on preview repos and compare HALL context with
Analysis and final journal. Offer only for unreflected evidence; state that
deferral neither blocks apply nor invalidates the fingerprint.

- [x] **Step 6: Run command tests and commit**

Run: `cargo test --lib harness::commands`

Expected: all fifteen embedded commands satisfy content contracts.

```bash
git add src/harness/commands/plan.md src/harness/commands/execute.md src/harness/commands/deliver.md tests/unit/harness/commands
git commit -m "feat(workflows): keep relation context alive"
```

### Task 7: Update domain and on-disk documentation

**Files:**
- Modify: `ARCHITECTURE.md`
- Modify: `docs/concepts.md`
- Modify: `docs/getting-started.md`
- Modify: `docs/glossary.md`
- Modify: `docs/reference/on-disk-format.md`
- Modify: `docs/guides/day-to-day.md`
- Modify: `docs/guides/planning-and-execution.md`

- [x] **Step 1: Update the module map and on-disk layout**

Document `harness/config/instructions.rs`, regular committed `HALL.md`, relative
committed aliases for enabled providers, the destructive disabled-provider rule,
and real ephemeral session-native files. Replace claims that root provider files
are independent sources.

- [x] **Step 2: Update the domain glossary**

Ensure the **Repo relation** entry says:

```markdown
**Repo relation** — a human-authored directed sentence from one registered Repo
to another, maintained by `/ivar-relations` in `HALL.md`. It expresses
co-belonging — the same intent as **`part of`** — not dependency, permission,
build order, merge order, or automatic promotion. The workflow keeps at most one
sentence per ordered pair; Rust does not parse or validate it.
```

- [x] **Step 3: Update user journeys**

State that init creates canonical instructions and its first alias, provider add
creates its alias, sync repairs topology, and enabled regular aliases require
human consolidation. Put the destructive disabled-provider behavior next to the
manifest-editing instructions, not in a footnote.

- [x] **Step 4: Update command and session references**

Document `/ivar-relations`, discovery/feature session derivation, and the warning
when canonical content is unavailable. Keep the statement that sessions still
open.

- [x] **Step 5: Run documentation integrity tests**

Run:

```bash
cargo test --test docs_reference
cargo test --test architecture
```

Expected: generated command references and architecture assertions pass; no docs
describe `CLAUDE.md` or `AGENTS.md` as editable sources.

- [x] **Step 6: Commit**

```bash
git add ARCHITECTURE.md docs
git commit -m "docs(relations): document canonical hall context"
```

### Task 8: Add black-box acceptance coverage and run release gates

**Files:**
- Create: `tests/repo_relations.rs`
- Modify: none expected; failures outside `tests/repo_relations.rs` must be reported as follow-up work rather than patched opportunistically

- [x] **Step 1: Write the compiled-binary lifecycle test**

Using existing integration helpers and a temporary Git hall, prove:

1. init creates `HALL.md` and the selected relative alias;
2. repo add JSON and human output expose `/ivar-relations <repo>`;
3. provider add creates the second alias immediately;
4. sync repairs a wrong symlink and preserves an enabled regular alias;
5. after manually disabling a provider, sync deletes its regular alias;
6. doctor reports all canonical/alias drift in one run;
7. discovery and feature session materialization use HALL bytes, not alias bytes;
8. missing HALL warns but session materialization succeeds;
9. all fifteen workflow commands materialize for both providers.

- [x] **Step 2: Run the new integration target**

Run: `cargo test --test repo_relations`

Expected: every lifecycle assertion passes against the compiled binary.

- [x] **Step 3: Format and inspect only intended changes**

Run:

```bash
cargo fmt --all -- --check
git status --short
git diff --check
```

Expected: formatting and whitespace checks pass; the diff contains only this
feature's files and no unrelated working-tree changes.

- [x] **Step 4: Run the full quality gate**

Run:

```bash
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo build --release
```

Expected: all tests pass, clippy is clean, and release build embeds all fifteen
Markdown workflows.

- [x] **Step 5: Perform a disposable smoke test**

In a temporary directory, initialize a hall, add two local bare remotes, add both
providers, run sync and doctor, inspect relative symlinks, then materialize one
discovery and one feature session. Expected: doctor is clean, aliases target
`HALL.md`, session files contain canonical bytes, and repo add advertises the
relations journey.

- [x] **Step 6: Commit final test corrections if any**

```bash
git add tests/repo_relations.rs src tests ARCHITECTURE.md docs
git commit -m "test(relations): cover canonical context lifecycle"
```

Do not create an empty commit when verification requires no corrections.

## Out of scope

- Persisting relation data or review state in `ivar.json`.
- Adding a provider-removal CLI command.
- Parsing the relation region in Rust.
- Automatic relation drift detection or Repo rename/remove edits.
- Changing promotion, delivery ordering, or delivery fingerprints.
- Persisting rejected proposals.
