# Shipped Workflow Commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `ivar` install, repair, diagnose, and safely remove its 14 official workflow commands for Claude Code and OpenCode.

**Architecture:** Keep one provider-neutral Markdown source per workflow beside a focused `harness::commands` module and embed every source in the single Rust binary with `include_str!`. The module owns only the shipped-command catalog and filesystem reconciliation; `Layout` computes target paths, while `init`, `sync`, `provider add`, and `doctor` orchestrate it without duplicating behavior. Official commands use the reserved `ivar-*` namespace, remain derived and gitignored, and never cause unrelated user commands to be overwritten or deleted.

**Tech Stack:** Rust 2024, `camino`, existing `infra::fs` and `infra::hash` adapters, `clap`, `serde`, `rstest`, `tempfile`, existing integration-test harness.

---

## Agreed behavior

1. Ship these workflows: `deliver`, `discovery`, `execute`, `feature-create`, `feature-status`, `plan`, `promote`, `repo-list`, `repo-setup`, `review`, `session-connect`, `session-start`, `session-stop`, and `sync`.
2. Materialize them as `/ivar-<id>` commands, not as hall skills. The word **Skill** remains reserved for `.ivar/skills/<id>/SKILL.md` bundles shared by a hall.
3. Materialize on `ivar init`, `ivar sync`, and `ivar provider add`.
4. Claude Code targets `.claude/commands/ivar-<id>.md`; OpenCode targets `.opencode/commands/ivar-<id>.md`.
5. `ivar-*` is reserved for Ivar-owned commands. Other files in either command directory belong to the user and must survive every operation.
6. A missing or modified official command is restored on sync. A removed provider loses only its `ivar-*` commands.
7. Generated commands are ignored with narrow patterns; user commands remain committable.
8. `ivar doctor` reports command drift with `ivar sync` as the repair, but command drift does not change hall health or block sessions.
9. A failed command write during `init`, `sync`, or `provider add` is a warning and leaves the primary operation complete and repairable.
10. Legacy Bifrost command files are removed only when their SHA-256 matches a known official Bifrost artifact. Modified files are preserved and diagnosed.

## File structure

### New files

- `src/harness/commands.rs` — embedded catalog, reconciliation plan/execution, integrity inspection, and fingerprint-gated legacy cleanup.
- `src/harness/commands/deliver.md`
- `src/harness/commands/discovery.md`
- `src/harness/commands/execute.md`
- `src/harness/commands/feature-create.md`
- `src/harness/commands/feature-status.md`
- `src/harness/commands/plan.md`
- `src/harness/commands/promote.md`
- `src/harness/commands/repo-list.md`
- `src/harness/commands/repo-setup.md`
- `src/harness/commands/review.md`
- `src/harness/commands/session-connect.md`
- `src/harness/commands/session-start.md`
- `src/harness/commands/session-stop.md`
- `src/harness/commands/sync.md`
- `tests/shipped_commands.rs` — black-box lifecycle tests across both providers.

### Modified files

- `src/harness/mod.rs` — explicitly expose `commands`; no broad re-export.
- `src/store/layout.rs` — add the one canonical command-directory path accessor and narrow gitignore patterns.
- `src/store/gitignore.rs` — update exact expected patterns in tests.
- `src/action/sync.rs` — reconcile commands next to instruction and MCP config, with per-provider warnings.
- `src/action/hall.rs` — bootstrap the selected provider during init and add doctor findings.
- `src/action/provider/add.rs` — materialize the newly added provider immediately and return warnings instead of “run sync” prose.
- `src/domain/provider.rs` — correct ownership documentation: Ivar owns the `ivar-*` namespace, not the entire commands directory.
- `ARCHITECTURE.md` — add `harness/commands.rs` and shipped workflow assets to the module map.
- `docs/glossary.md` — define **Workflow Command** separately from **Skill**.
- `docs/getting-started.md` — state that init/sync/provider-add install official commands.
- `docs/guides/skills.md` — explicitly distinguish hall skills from shipped workflow commands.
- `docs/reference/on-disk-format.md` — mark generated `ivar-*` command files as local derived state.
- `docs/reference/commands.md` — regenerated only if clap descriptions change.

## Legacy fingerprints

Use these exact SHA-256 values when deciding whether an unprefixed Bifrost command is safe to remove:

| id | legacy SHA-256 |
| --- | --- |
| `deliver` | `b8402403fba034c85355def2f40ca9cec0e5572f4e67b130ebeac14ceda64c8b` |
| `discovery` | `97fba325393f6eba415a62bb6120d7bdc4cd813872e15d6f6669c910e32c0120` |
| `execute` | `94c2aa9d9617de45cc5d985e752a99d4c6f5899654967d618542f270a5e18a72` |
| `feature-create` | `062a359e6ecf9fa8313d65f478737ee0018ef1c4c17868e2dff3e7abbc3dfe16` |
| `feature-status` | `67d092c2ecf3469a96c17fd8971dd6caa2e0ea97ca404361fea59617d681129c` |
| `plan` | `5b1e361e11d342c022901a41f89de1a8b2463eb63c42e15d4e8fee9498fa188e` |
| `promote` | `eae89c066ce3526b5e7cb3d4cd76f822faec9b3430965d4fdf83ae97e40c084f` |
| `repo-list` | `cd8705d0e972c339ca55607c89e5cf4702123677e1a1c02ea4cf5502d105a8e1` |
| `repo-setup` | `255554048fcf58d7f6d396acc1713bc888d185e00794db47be2965a849bc4068` |
| `review` | `da6d0ad313c366246d0b15fac0e04340af65786486dfaed5f5128770537d4b2d` |
| `session-connect` | `c81e99ac2bbfcea31381e61ead8e2a51cf91c46781e4466025ab11f23bee7b24` |
| `session-start` | `43affb5874c67b0aa2e904c7bca48499401f8d04667cbe6500add74d2c6508e4` |
| `session-stop` | `2e2c6fc76618a19f77dec801dd59d52b6a5b6446f8f048943750534701aa4bbd` |
| `sync` | `e663a6534823dcc7a0699e126d4e32619277e08ea48e657de8f74da0806bf15d` |

---

### Task 1: Add canonical paths and narrow ignore rules

**Files:**
- Modify: `src/store/layout.rs:265-301,345-355,499-630`
- Modify: `src/store/gitignore.rs:92-162`

- [ ] **Step 1: Write failing layout tests**

Add assertions for both providers and for the exact ignore contract:

```rust
#[test]
fn command_dirs_use_each_providers_native_location() {
    let layout = Layout::at(Utf8PathBuf::from("/hall"));
    assert_eq!(
        layout.commands_dir(&Provider::ClaudeCode),
        Utf8PathBuf::from("/hall/.claude/commands")
    );
    assert_eq!(
        layout.commands_dir(&Provider::OpenCode),
        Utf8PathBuf::from("/hall/.opencode/commands")
    );
}

#[test]
fn gitignore_lines_ignore_only_ivar_shipped_commands() {
    assert_eq!(
        Layout::gitignore_lines(),
        vec![
            ".ivar/*",
            "!.ivar/skills/",
            "!.ivar/setups/",
            ".claude/commands/ivar-*.md",
            ".opencode/commands/ivar-*.md",
        ]
    );
}
```

- [ ] **Step 2: Run the focused tests and verify failure**

Run:

```bash
cargo test --lib store::layout::tests::command_dirs_use_each_providers_native_location
cargo test --lib store::layout::tests::gitignore_lines_ignore_only_ivar_shipped_commands
```

Expected: compile failure because `Layout::commands_dir` does not exist, followed by an assertion mismatch for the old three-line gitignore contract.

- [ ] **Step 3: Implement the path accessor and ignore lines**

Add to `impl Layout` beside `harness_dir`:

```rust
/// The provider-native directory containing project workflow commands.
/// Ivar owns only files matching `ivar-*.md` inside it.
#[must_use]
pub fn commands_dir(&self, provider: &Provider) -> Utf8PathBuf {
    self.root().join(provider.commands_dir())
}
```

Change `gitignore_lines` to return the five exact entries shown in the test. Update `gitignore.rs` expectations so existing user content is still preserved and repeated `ensure` calls remain byte-idempotent.

- [ ] **Step 4: Run focused tests**

Run: `cargo test --lib store::layout store::gitignore`

Expected: all layout and gitignore tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/store/layout.rs src/store/gitignore.rs
git commit -m "feat(commands): add provider command paths"
```

### Task 2: Create the embedded workflow catalog

**Files:**
- Create: `src/harness/commands.rs`
- Create: all 14 `src/harness/commands/*.md` files listed above
- Modify: `src/harness/mod.rs`

- [ ] **Step 1: Write catalog tests before exposing filesystem behavior**

Start `commands.rs` with tests that require 14 unique definitions, prefixed filenames, non-empty descriptions, no legacy product name, and at least one current Ivar invocation:

```rust
#[test]
fn catalog_is_complete_unique_and_current() {
    let commands = catalog();
    assert_eq!(commands.len(), 14);

    let ids = commands.iter().map(|command| command.id).collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), commands.len());

    for command in commands {
        assert_eq!(command.file_name(), format!("ivar-{}.md", command.id));
        assert!(command.content.starts_with("---\n"));
        assert!(command.content.contains("description:"));
        assert!(command.content.contains("`ivar "));
        assert!(!command.content.contains("bifrost"));
        assert!(!command.content.contains("BIFROST_"));
    }
}
```

- [ ] **Step 2: Run the test and verify failure**

Run: `cargo test --lib harness::commands::tests::catalog_is_complete_unique_and_current`

Expected: compile failure because the module/catalog does not exist.

- [ ] **Step 3: Add the focused catalog types**

Use an explicit static catalog—no `build.rs`, directory scanning, proc macro, or runtime asset dependency:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShippedCommand {
    pub id: &'static str,
    pub content: &'static str,
    pub legacy_sha256: &'static str,
}

impl ShippedCommand {
    #[must_use]
    pub fn file_name(self) -> String {
        format!("ivar-{}.md", self.id)
    }

    #[must_use]
    pub fn legacy_file_name(self) -> String {
        format!("{}.md", self.id)
    }
}

pub const fn catalog() -> &'static [ShippedCommand] {
    &COMMANDS
}
```

Define `COMMANDS` explicitly with one `include_str!` for each of the 14 exact Markdown paths listed in **New files**, paired with the matching legacy hash table entry in this plan. Expose the module from `harness/mod.rs` with `pub mod commands;`; do not re-export its members from `harness`.

- [ ] **Step 4: Author the 14 provider-neutral Markdown sources**

Use the corresponding `packages/bifrost/skills/<id>/SKILL.md` file as editorial input, not as code to copy blindly. Every new file must:

1. Keep YAML frontmatter with `description` and, where applicable, `argument-hint`.
2. Name the user-facing command `/ivar-<id>`.
3. Use only `IVAR_SESSION_ID`, `IVAR_FEATURE`, `IVAR_SESSION_PATH`, and current Ivar paths.
4. Replace the legacy noun surface according to this map:

```text
bifrost hall sync                         -> ivar sync
bifrost hall feature create              -> ivar feature create
bifrost hall feature list                -> ivar feature list
bifrost hall feature promote             -> ivar feature promote <feature> <repo>
bifrost hall feature status              -> ivar feature status <feature>
bifrost hall session start               -> ivar session start [feature]
bifrost hall session connect             -> ivar session connect
bifrost hall session stop                -> ivar session stop
bifrost hall repo list                    -> ivar repo list
bifrost hall repo setup                   -> ivar repo setup [repo]
bifrost hall review                       -> ivar feature review <feature>
bifrost hall deliver                      -> ivar feature deliver <feature>
```

For every flag and positional argument, consult `docs/reference/commands.md`, which is generated from clap. Do not preserve a Bifrost flag merely because its old workflow mentions it.

- [ ] **Step 5: Run catalog and documentation-reference tests**

Run:

```bash
cargo test --lib harness::commands
cargo test --test docs_reference
```

Expected: both pass; no shipped command contains `bifrost` or `BIFROST_`.

- [ ] **Step 6: Commit**

```bash
git add src/harness/mod.rs src/harness/commands.rs src/harness/commands
git commit -m "feat(commands): embed official workflows"
```

### Task 3: Implement safe command reconciliation

**Files:**
- Modify: `src/harness/commands.rs`

- [ ] **Step 1: Add failing reconciliation tests**

Use a temporary command directory for these six exact cases:

1. `materialise_creates_repairs_and_then_becomes_idempotent`: first call returns 14 `Created` entries; after replacing `ivar-plan.md` with `changed`, the next call returns `Updated` for plan; a third call returns 14 `Unchanged` entries and leaves bytes unchanged.
2. `materialise_preserves_unrelated_user_commands`: write `custom.md` before materializing and assert its exact bytes survive.
3. `materialise_removes_unknown_files_in_reserved_ivar_namespace`: write `ivar-retired.md`, materialize, and assert the file is absent with a matching `Removed` entry.
4. `remove_deletes_only_reserved_ivar_commands`: materialize, add `custom.md`, remove, and assert all 14 official files are absent while `custom.md` remains byte-identical.
5. `matching_legacy_command_is_removed`: write a real legacy fixture whose digest equals its catalog constant and assert materialization deletes the unprefixed file.
6. `modified_legacy_command_is_preserved_and_reported`: append one byte to that fixture and assert the file survives and inspection returns `LegacyModified`.

The legacy tests should use one checked-in test byte string whose digest is one of the constants in the catalog; do not fake the digest comparison by passing an expected hash into the production function.

- [ ] **Step 2: Run and verify the tests fail**

Run: `cargo test --lib harness::commands::tests`

Expected: compile failures for the missing reconciliation API.

- [ ] **Step 3: Add explicit result types**

Use result values that callers can render without recomputing state:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Created,
    Updated,
    Removed,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandChange {
    pub id: String,
    pub file_name: String,
    pub change: Change,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Integrity {
    Current,
    Missing,
    Modified,
    LegacyModified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub id: String,
    pub path: Utf8PathBuf,
    pub integrity: Integrity,
}
```

- [ ] **Step 4: Implement `materialise`, `remove`, and `inspect`**

Required signatures:

```rust
pub fn materialise(commands_dir: &Utf8Path) -> Result<Vec<CommandChange>, Error>;
pub fn remove(commands_dir: &Utf8Path) -> Result<Vec<CommandChange>, Error>;
pub fn inspect(commands_dir: &Utf8Path, enabled: bool) -> Result<Vec<Inspection>, Error>;
```

Implementation rules:

- Ensure the directory only when materializing.
- Compare bytes before every write; use `infra::fs::write_atomic` only when content differs.
- Enumerate existing `.md` files once.
- Delete unknown `ivar-*.md` files because that prefix is reserved.
- Never delete a non-prefixed file unless its filename and SHA-256 both match a catalog legacy entry.
- Preserve a modified legacy file and return `Integrity::LegacyModified` from inspection.
- When removing a provider, delete all `ivar-*.md`, preserve every other file, and remove the now-empty directory only if the filesystem adapter can prove it is empty.
- Map `infra::fs::Error` into one module-local `Error` carrying the affected path; callers convert that to `Failure`.

- [ ] **Step 5: Run focused tests and clippy**

Run:

```bash
cargo test --lib harness::commands
cargo clippy --all-targets -- -D warnings
```

Expected: all command tests pass and clippy reports no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/harness/commands.rs
git commit -m "feat(commands): reconcile managed workflow files"
```

### Task 4: Wire commands into sync

**Files:**
- Modify: `src/action/sync.rs:20-44,268-334,639-709,1144-1310`

- [ ] **Step 1: Add failing sync tests**

Add five tests beside the existing provider tests:

1. `sync_materialises_shipped_commands_for_available_providers`: a manifest with both providers produces 14 official files in each native command directory.
2. `second_sync_reports_commands_unchanged_without_rewriting`: capture every official file's bytes and modified time, sync again, and assert all are unchanged.
3. `sync_repairs_modified_shipped_command_and_preserves_custom_command`: replace `ivar-plan.md`, add `custom.md`, sync, and assert plan equals the embedded source while custom remains byte-identical.
4. `sync_removes_only_shipped_commands_for_unavailable_provider`: remove OpenCode from the manifest, sync, and assert OpenCode's `ivar-*` files are gone while its `custom.md` survives.
5. `command_write_failure_warns_and_other_provider_steps_continue`: place a regular file where one provider's command-directory parent must exist, sync, and assert command entries fail with warnings while the other provider's commands and config complete. Use an occupied path rather than permission bits so the test is deterministic even under a privileged CI user.

- [ ] **Step 2: Run the focused test and verify failure**

Run: `cargo test --lib action::sync::tests::sync_materialises_shipped_commands_for_available_providers`

Expected: failure because sync does not create command files.

- [ ] **Step 3: Extend provider reconciliation without enlarging `sync_provider`**

Keep instruction blocks, MCP, and commands as separate concerns:

```rust
for provider in Provider::ALL {
    sync_provider(layout, manifest, provider, &block, entries, warnings);
    sync_mcp(layout, manifest, provider, entries, warnings);
    sync_commands(layout, manifest, provider, entries, warnings);
}
```

`sync_commands` must call `commands::materialise` when available and `commands::remove` otherwise. Convert each returned `CommandChange` into an existing sync `Entry` with surface `provider.id()` and label `command <filename>`. On error, call `record_failure`; do not abort other repos/providers.

- [ ] **Step 4: Update sync module documentation**

The ordered behavior must now say provider reconciliation includes instruction block, MCP config, and official workflow commands. The “deliberately does not do” section must remain unchanged.

- [ ] **Step 5: Run sync tests**

Run: `cargo test --lib action::sync`

Expected: all existing and new sync tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/action/sync.rs
git commit -m "feat(sync): materialise official workflow commands"
```

### Task 5: Bootstrap commands during init and provider add

**Files:**
- Modify: `src/action/hall.rs:127-167` and init tests
- Modify: `src/action/provider/add.rs:1-109,111-240`

- [ ] **Step 1: Add failing action tests**

Add four tests with these exact assertions:

1. `init_materialises_commands_for_its_selected_provider`: OpenCode init creates 14 OpenCode files and no Claude command directory.
2. `init_returns_warning_when_command_materialisation_fails`: a regular file occupying OpenCode's command parent yields warning code `provider.commands_not_materialised`, while `ivar.json` remains valid and selects OpenCode.
3. `provider_add_materialises_the_new_providers_commands`: adding OpenCode to a Claude hall creates all 14 OpenCode files without a follow-up sync.
4. `provider_add_returns_warning_when_commands_cannot_be_written`: a regular file occupying OpenCode's command parent yields the same warning while OpenCode remains persisted in `providers.available`.

- [ ] **Step 2: Run tests and verify failure**

Run:

```bash
cargo test --lib action::hall::tests::init_materialises_commands_for_its_selected_provider
cargo test --lib action::provider::add::tests::provider_add_materialises_the_new_providers_commands
```

Expected: target command files are absent.

- [ ] **Step 3: Add one small action-level adapter**

Do not call the `sync` action from `init` or `provider add`. Add a focused helper in `action/sync.rs` or a new private helper in the immediate caller that accepts `&Layout` and `Provider`, calls `harness::commands::materialise`, and converts failure to this warning shape:

```rust
Warning::new(
    "provider.commands_not_materialised",
    provider.id(),
    format!("official commands could not be written: {error}; run `ivar sync` to repair"),
)
```

Prefer a shared `pub(crate)` adapter only if it avoids duplicate error mapping in both callers; the actual filesystem behavior remains in `harness::commands`.

- [ ] **Step 4: Wire init and provider add**

After durable manifest/skeleton writes succeed, attempt command materialization. Return `Report::with_warnings` on failure. Do not roll back `ivar.json`, remove the provider, or convert a repairable command problem into exit code 2.

Update `provider/add.rs` module docs and human output: remove “Run `ivar sync` to materialise its config.” A successful add must say the provider is registered and ready; a warning already carries the repair command when needed.

- [ ] **Step 5: Run action tests**

Run:

```bash
cargo test --lib action::hall
cargo test --lib action::provider::add
```

Expected: all tests pass; warning cases keep their manifest changes.

- [ ] **Step 6: Commit**

```bash
git add src/action/hall.rs src/action/provider/add.rs
git commit -m "feat(commands): bootstrap workflows during setup"
```

### Task 6: Diagnose command drift without blocking sessions

**Files:**
- Modify: `src/action/hall.rs:367-455` and doctor tests
- Test: `tests/shipped_commands.rs`

- [ ] **Step 1: Add failing doctor tests**

Cover missing, modified, legacy-modified, unavailable-provider cleanup, and healthy states. Stable diagnosis codes:

```text
provider.command_missing
provider.command_modified
provider.legacy_command_modified
provider.command_stale
```

Every finding must say `Run ivar sync` except `legacy_command_modified`, which must tell the user to rename/remove the preserved customized file after review.

- [ ] **Step 2: Verify tests fail**

Run: `cargo test --lib action::hall::tests::doctor_reports_missing_shipped_command`

Expected: no command diagnosis is emitted.

- [ ] **Step 3: Extend doctor only, not health derivation**

For each `Provider::ALL`, call `commands::inspect(layout.commands_dir(&provider), enabled)`. Map every non-`Current` inspection to a `Diagnosis`. Do not change `domain::health`, `StatusOutcome.health`, or session-start gates.

- [ ] **Step 4: Add black-box lifecycle coverage**

In `tests/shipped_commands.rs`, invoke the compiled binary against temporary halls and prove:

1. `ivar init --provider claude-code` creates 14 `.claude/commands/ivar-*.md` files.
2. `ivar provider add opencode` creates 14 `.opencode/commands/ivar-*.md` files immediately.
3. A user `custom.md` survives sync and provider removal behavior.
4. A modified `ivar-plan.md` is restored by sync.
5. A fingerprint-matching Bifrost `plan.md` is removed; a modified one remains and appears in doctor output.
6. `ivar status` stays `operational` when only a shipped command is missing.

- [ ] **Step 5: Run doctor and integration tests**

Run:

```bash
cargo test --lib action::hall
cargo test --test shipped_commands
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/action/hall.rs tests/shipped_commands.rs
git commit -m "feat(doctor): diagnose workflow command drift"
```

### Task 7: Reconcile ownership documentation and user guides

**Files:**
- Modify: `src/domain/provider.rs:20-32,157-176`
- Modify: `ARCHITECTURE.md:26-116`
- Modify: `docs/glossary.md:192-213`
- Modify: `docs/getting-started.md`
- Modify: `docs/guides/skills.md`
- Modify: `docs/reference/on-disk-format.md`

- [ ] **Step 1: Correct the provider ownership contract**

Replace the current claim that the entire commands directory is owned by Ivar. The precise contract is:

```text
Ivar owns files named ivar-*.md in each provider's project command directory.
Other command files belong to the user and are never changed or removed by Ivar.
```

- [ ] **Step 2: Update the architecture map**

Document `harness/commands.rs` as “embedded shipped-workflow catalog, reconciliation, integrity inspection, legacy fingerprint cleanup” and `harness/commands/*.md` as provider-neutral source assets compiled into the binary. Keep `bin/ivar.rs` and `action` thin; no command content or file reconciliation belongs there.

- [ ] **Step 3: Add the glossary distinction**

Add:

```markdown
**Workflow Command** — an instruction workflow shipped inside the `ivar` binary
and materialised into each available Provider as `/ivar-<name>`. Workflow Commands
are local derived state: `ivar init`, `ivar provider add`, and `ivar sync` create
or repair them. They are not **Skills** and are not shared through `.ivar/skills/`.
```

Update **Sync** to include official workflow commands among per-harness config.

- [ ] **Step 4: Update guides and on-disk reference**

Document the two independent surfaces:

```text
.ivar/skills/<id>/SKILL.md                 committed hall-owned source
.claude/skills/<id>/...                   derived hall skill target
.opencode/skills/<id>/...                 derived hall skill target
.claude/commands/ivar-<id>.md              derived Ivar workflow command
.opencode/commands/ivar-<id>.md            derived Ivar workflow command
```

State that custom command names should not use the reserved `ivar-*` prefix.

- [ ] **Step 5: Run docs integrity tests**

Run:

```bash
cargo test --test docs_reference
cargo test --test architecture
```

Expected: documentation reference and layering tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/domain/provider.rs ARCHITECTURE.md docs
git commit -m "docs(commands): document shipped workflow ownership"
```

### Task 8: Final verification and release readiness

**Files:**
- Modify only files required by failures found below.

- [ ] **Step 1: Format and inspect the diff**

Run:

```bash
cargo fmt --all -- --check
```

Expected: no formatting or whitespace errors. If rustfmt changes are required, run `cargo fmt --all`, inspect them, and include only intended formatting.

- [ ] **Step 2: Run the full quality gate**

Run:

```bash
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo build --release
```

Expected: all tests pass (including the existing 862-test baseline plus new tests), clippy is clean, and the release binary builds with every Markdown asset embedded.

- [ ] **Step 3: Perform a disposable smoke test with the release binary**

In a temporary directory:

```bash
git init demo-hall
cd demo-hall
path/to/target/release/ivar init --provider claude-code
path/to/target/release/ivar provider add opencode
path/to/target/release/ivar sync
path/to/target/release/ivar doctor
```

Expected:

- Both providers have 14 `ivar-*.md` command files.
- `.gitignore` contains only the two narrow generated-command patterns.
- `git status --short` does not list generated commands.
- A manually created `.claude/commands/custom.md` remains after another sync.
- Doctor reports no problems.

- [ ] **Step 4: Confirm the binary has no runtime source dependency**

Move or copy only the release binary outside the source tree and repeat `ivar init` in another temporary directory. Expected: commands still materialize because `include_str!` embedded them at compile time.

- [ ] **Step 5: Commit any final test-only corrections**

```bash
git add src/harness src/store src/action src/domain tests/shipped_commands.rs ARCHITECTURE.md docs
git commit -m "test(commands): complete workflow bootstrap coverage"
```

Do not create an empty commit when verification requires no changes.

## Out of scope

- Installing official workflows as `.claude/skills`, `.opencode/skills`, or `.agents/skills`.
- Supporting `/ivar:plan`; project-command portability is proven for `/ivar-plan` instead.
- Adding a third provider or a provider plugin system.
- Allowing user overrides of official `ivar-*` files in place.
- Changing hall health or blocking session start because a convenience command is missing.
- Moving hall skills out of `.ivar/skills/` or changing their existing sync semantics.
- Introducing a runtime asset directory, `build.rs`, async runtime, or new dependency.
