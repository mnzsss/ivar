# Session Provider Targeting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve each untargeted execution workstream to the current Ivar session's provider, persist that provider in `plan.md` and `board.json` before approval, and prevent `tick` from silently switching to the hall's default provider.

**Architecture:** Keep `tick` as an executor of an already-approved target: provider resolution happens during `execute prepare`, while the plan is still mutable and before its fingerprint is recorded. The CLI passes the caller's session id explicitly; a focused execution-targeting module reads that session's `state.json`, merges session defaults with explicit per-workstream targeting, and synchronizes the canonical `## Operations` metadata in `plan.md`. The board then contains an explicit provider for every workstream, so `tick` selects the recorded harness rather than rediscovering intent at launch time.

**Tech Stack:** Rust 2024, Clap, Serde, Markdown line parsing, existing Ivar `SessionState`/`ExecutionBoard` stores, Rust unit and integration tests.

---

## Agreed behavior

1. `/ivar-execute` runs only from an identifiable Ivar feature session and passes its `IVAR_SESSION_ID` to `execute prepare`.
2. A workstream with no explicit provider inherits the caller session's provider.
3. An explicit per-workstream provider overrides the session provider.
4. The resolved provider is written to the workstream's block in `plan.md` before the plan fingerprint is computed.
5. The same resolved provider is written to `board.json`; the plan and board cannot disagree silently.
6. `model` and `agent` remain separate selectors and are persisted beside `provider`.
7. If any workstream needs a provider default and no readable caller session is supplied, `prepare` fails. It never falls back silently to the hall default.
8. Existing plans with explicit providers remain executable outside a session. Existing serialized boards remain readable because `WorkstreamDef.provider` stays optional at the schema level.

## File responsibility map

- `src/action/execute/plan_ops.rs` — parse and rewrite targeting metadata inside the existing `## Operations` workstream blocks.
- `src/action/execute/targeting.rs` — resolve caller-session provider, merge plan/graph targeting, reject conflicts, and return synchronized workstreams plus plan text.
- `src/action/execute/prepare.rs` — orchestrate graph read, targeting resolution, plan write, fingerprinting, prompt validation, and board creation in that order.
- `src/action/execute/tick/mod.rs` — enforce that newly prepared workstreams arrive with explicit providers; retain legacy fallback only for old boards.
- `src/cli/root.rs` — expose `--session` on `execute prepare` and map it into `PrepareInput`.
- `src/harness/commands/execute.md` — read the current session id, persist targeting in the candidate plan/graph, and pass `--session` to `prepare`.
- Tests stay colocated under `tests/unit/action/execute/` and `tests/unit/harness/commands.rs`; no broad folder move is needed.

---

### Task 1: Extend the plan Operations format with execution targeting

**Files:**
- Modify: `src/action/execute/plan_ops.rs`
- Modify: `tests/unit/action/execute/plan_ops.rs`
- Modify: `src/action/plan/create.rs:37-84`

- [ ] **Step 1: Add failing parser tests for provider, model, and agent**

Extend `REVISED_PLAN` in `tests/unit/action/execute/plan_ops.rs` so one workstream contains scalar targeting lines before its operation bullets:

```rust
const REVISED_PLAN: &str = "# Plan\n\
    \n\
    ## Operations\n\
    \n\
    ### ws-gates\n\
    provider: opencode\n\
    model: deepseek-chat\n\
    agent: implementer-deepseek\n\
    - add-gate-types\n\
    - wire-approve\n\
    write_contract:\n\
    - src/domain/feature.rs\n\
    \n\
    ### ws-board\n\
    provider: claude-code\n\
    - add-board-types\n\
    - store-board\n\
    - tick-board\n\
    write_contract:\n\
    - src/action/execute\n";
```

Add assertions:

```rust
assert_eq!(parsed[0].provider, Some(Provider::OpenCode));
assert_eq!(parsed[0].model.as_deref(), Some("deepseek-chat"));
assert_eq!(parsed[0].agent.as_deref(), Some("implementer-deepseek"));
assert_eq!(parsed[1].provider, Some(Provider::ClaudeCode));
assert_eq!(parsed[1].model, None);
assert_eq!(parsed[1].agent, None);
```

Also add a malformed-provider test:

```rust
#[test]
fn operations_from_plan_refuses_an_unknown_provider() {
    let plan = "# Plan\n\n## Operations\n\n### ws-a\nprovider: deepseek\n- op-a\n";
    let failure = operations_from_plan(plan).unwrap_err();
    assert_eq!(failure.code, "execute.plan_provider_invalid");
    assert!(failure.what.contains("deepseek"));
    assert!(failure.what.contains("claude-code"));
    assert!(failure.what.contains("opencode"));
}
```

- [ ] **Step 2: Run the focused parser tests and verify the expected failure**

Run:

```bash
cargo test --lib action::execute::plan_ops::tests -- --nocapture
```

Expected: compilation fails because `PlanWorkstream` has no targeting fields and `operations_from_plan` does not return a `Result` yet.

- [ ] **Step 3: Make parsing typed and explicit**

In `src/action/execute/plan_ops.rs`, import the existing provider and failure types:

```rust
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
```

Extend `PlanWorkstream`:

```rust
pub(crate) struct PlanWorkstream {
    pub(crate) id: String,
    pub(crate) operations: Vec<String>,
    pub(crate) write_contract: Vec<String>,
    pub(crate) provider: Option<Provider>,
    pub(crate) model: Option<String>,
    pub(crate) agent: Option<String>,
}
```

Change the parser signature:

```rust
pub(crate) fn operations_from_plan(text: &str) -> Result<Vec<PlanWorkstream>, Failure>
```

Initialize the new fields to `None`, recognize only the three exact scalar keys while inside a workstream, and parse providers through `Provider::from_str`:

```rust
if let Some(value) = trimmed.strip_prefix("provider:") {
    let value = value.trim();
    let provider = value.parse::<Provider>().map_err(|_| {
        Failure::blocked(
            "execute.plan_provider_invalid",
            format!("workstream `{}` names unknown provider `{value}`", workstream.id),
        )
        .expected("`claude-code` or `opencode`")
        .actual(value)
        .fix(FixAction::safe(
            "execute.plan_provider_fix",
            "Set `provider:` to `claude-code` or `opencode`.",
        ))
    })?;
    workstream.provider = Some(provider);
    continue;
}
if let Some(value) = trimmed.strip_prefix("model:") {
    workstream.model = non_empty_target("model", &workstream.id, value)?;
    continue;
}
if let Some(value) = trimmed.strip_prefix("agent:") {
    workstream.agent = non_empty_target("agent", &workstream.id, value)?;
    continue;
}
```

Use one helper so empty selectors are rejected consistently:

```rust
fn non_empty_target(
    field: &str,
    workstream: &str,
    value: &str,
) -> Result<Option<String>, Failure> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Failure::blocked(
            "execute.plan_target_empty",
            format!("workstream `{workstream}` has an empty `{field}:` value"),
        )
        .expected(format!("a non-empty {field} selector, or no `{field}:` line"))
        .actual("an empty value"));
    }
    Ok(Some(value.to_owned()))
}
```

Return `Ok(workstreams)` and update every caller (`prompt.rs`, `replan.rs`, and their tests) to propagate `?` rather than treating parsing as infallible.

- [ ] **Step 4: Add a pure plan-targeting rewrite function**

Add a function that replaces or inserts `provider:`, `model:`, and `agent:` directly after the matching `### <workstream>` heading, preserving all unrelated prose and operation details:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedTarget {
    pub(crate) id: String,
    pub(crate) provider: Provider,
    pub(crate) model: Option<String>,
    pub(crate) agent: Option<String>,
}

pub(crate) fn write_targets(
    text: &str,
    targets: &[ResolvedTarget],
) -> Result<String, Failure>
```

The implementation must:

1. Enter only the `## Operations` section.
2. Match workstreams by exact `###` heading text.
3. Remove existing `provider:`, `model:`, and `agent:` lines in that block.
4. Insert the resolved lines after the heading in stable order: provider, model, agent.
5. Preserve the final newline state of the original file.
6. Fail with `execute.plan_workstream_missing` if any target has no matching heading.

Add a round-trip test:

```rust
#[test]
fn write_targets_persists_resolved_targeting_without_changing_operations() {
    let targets = vec![ResolvedTarget {
        id: "ws-gates".to_owned(),
        provider: Provider::OpenCode,
        model: Some("deepseek-chat".to_owned()),
        agent: Some("implementer-deepseek".to_owned()),
    }];

    let rewritten = write_targets(REVISED_PLAN, &targets).unwrap();
    let parsed = operations_from_plan(&rewritten).unwrap();

    assert_eq!(parsed[0].provider, Some(Provider::OpenCode));
    assert_eq!(parsed[0].operations, vec!["add-gate-types", "wire-approve"]);
    assert!(rewritten.contains(
        "### ws-gates\nprovider: opencode\nmodel: deepseek-chat\nagent: implementer-deepseek"
    ));
}
```

- [ ] **Step 5: Update the plan scaffold**

In `src/action/plan/create.rs`, change the example workstream block to document the new metadata:

```md
### <workstream-id>
provider: opencode
model: provider/model
agent: agent-name
- OP-<SLUG>
write_contract:
- path/it/may/write.rs
```

Explain in the surrounding template text that `provider` becomes explicit before approval, while `model` and `agent` may be omitted to use provider defaults.

- [ ] **Step 6: Run parser and plan tests**

Run:

```bash
cargo test --lib action::execute::plan_ops::tests action::plan::create::tests
```

Expected: all selected tests pass.

- [ ] **Step 7: Commit the plan-format slice**

```bash
git add src/action/execute/plan_ops.rs src/action/plan/create.rs tests/unit/action/execute/plan_ops.rs
git commit -m "feat(execute): persist workstream targeting in plans"
```

---

### Task 2: Resolve targeting from the caller's session during prepare

**Files:**
- Create: `src/action/execute/targeting.rs`
- Modify: `src/action/execute/mod.rs`
- Modify: `src/action/execute/prepare.rs`
- Modify: `tests/unit/action/execute/prepare.rs`
- Modify: every test constructing `PrepareInput` under `tests/unit/action/`

- [ ] **Step 1: Add a reusable feature-session fixture**

In `tests/unit/action/execute/prepare.rs`, add a helper that creates a real session record without spawning a provider:

```rust
fn feature_session(root: &Utf8PathBuf, provider: Provider) -> String {
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let id = SessionId::new(uuid::Uuid::new_v4().to_string()).unwrap();
    let view_dir = layout.feature_session(&feature, &id);
    fs::ensure_dir(&view_dir).unwrap();

    let mut state = SessionState::new(provider, "2026-08-14T00:00:00.000000000Z");
    state.bind(feature, "2026-08-14T00:00:00.000000000Z");
    state.write(&view_dir).unwrap();
    id.to_string()
}
```

- [ ] **Step 2: Write failing prepare tests for resolution and persistence**

Add these cases:

```rust
#[test]
fn prepare_inherits_the_caller_sessions_provider_and_persists_it() {
    let (_guard, root) = seeded_hall();
    let graph = graph_file(&root);
    let session = feature_session(&root, Provider::OpenCode);

    let report = prepare(
        &Ctx::new(root.clone()),
        PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
            session: Some(session),
        },
    )
    .unwrap();

    assert!(report
        .value
        .board
        .graph
        .workstreams
        .iter()
        .all(|ws| ws.provider == Some(Provider::OpenCode)));
    let plan = fs::read_text(&root.join("plans/checkout/plan.md"))
        .unwrap()
        .unwrap();
    assert!(plan.contains("provider: opencode"));
    assert_eq!(report.value.board.graph.plan_fingerprint, hash::file(
        &root.join("plans/checkout/plan.md")
    ).unwrap());
}
```

```rust
#[test]
fn an_explicit_workstream_provider_overrides_the_session_provider() {
    let (_guard, root) = seeded_hall();
    let plan_path = root.join("plans/checkout/plan.md");
    let plan = fs::read_text(&plan_path).unwrap().unwrap().replacen(
        "### ws-gates\n",
        "### ws-gates\nprovider: claude-code\n",
        1,
    );
    fs::write_text(&plan_path, &plan).unwrap();
    let graph = root.join("graph.json");
    let graph_text = GRAPH_JSON.replacen(
        "\"write_contract\": [\"src/domain/feature.rs\"]",
        "\"write_contract\": [\"src/domain/feature.rs\"],\n            \"provider\": \"claude-code\"",
        1,
    );
    fs::write_text(&graph, &graph_text).unwrap();
    let session = feature_session(&root, Provider::OpenCode);

    let report = prepare(
        &Ctx::new(root.clone()),
        PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
            session: Some(session),
        },
    )
    .unwrap();

    let board = report.value.board;
    let persisted_plan = fs::read_text(&plan_path).unwrap().unwrap();
    assert_eq!(
        board.graph.workstreams[0].provider,
        Some(Provider::ClaudeCode)
    );
    assert!(persisted_plan.contains("### ws-gates\nprovider: claude-code"));
    assert_eq!(board.graph.workstreams[1].provider, Some(Provider::OpenCode));
}
```

Add missing-context coverage:

```rust
#[test]
fn prepare_refuses_an_unresolved_provider_without_a_caller_session() {
    let (_guard, root) = seeded_hall();
    let graph = graph_file(&root);
    let failure = prepare(
        &Ctx::new(root.clone()),
        PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
            session: None,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "execute.provider_context_missing");
    assert!(failure.what.contains("caller session"));
}
```

Add disagreement coverage:

```rust
#[test]
fn prepare_refuses_provider_drift_between_plan_and_graph() {
    let (_guard, root) = seeded_hall();
    let plan_path = root.join("plans/checkout/plan.md");
    let plan = fs::read_text(&plan_path).unwrap().unwrap().replacen(
        "### ws-gates\n",
        "### ws-gates\nprovider: opencode\n",
        1,
    );
    fs::write_text(&plan_path, &plan).unwrap();
    let graph = root.join("graph.json");
    let graph_text = GRAPH_JSON.replacen(
        "\"write_contract\": [\"src/domain/feature.rs\"]",
        "\"write_contract\": [\"src/domain/feature.rs\"],\n            \"provider\": \"claude-code\"",
        1,
    );
    fs::write_text(&graph, &graph_text).unwrap();
    let session = feature_session(&root, Provider::OpenCode);

    let failure = prepare(
        &Ctx::new(root),
        PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
            session: Some(session),
        },
    )
    .unwrap_err();
    assert_eq!(failure.code, "execute.targeting_conflict");
    assert!(failure.what.contains("ws-gates"));
    assert!(failure.what.contains("opencode"));
    assert!(failure.what.contains("claude-code"));
}
```

- [ ] **Step 3: Run prepare tests and verify they fail**

Run:

```bash
cargo test --lib action::execute::prepare::tests -- --nocapture
```

Expected: compilation fails because `PrepareInput.session` and the targeting resolver do not exist.

- [ ] **Step 4: Add the focused targeting resolver**

Create `src/action/execute/targeting.rs` with a pure result shape and one orchestration function:

```rust
use crate::domain::feature::WorkstreamDef;
use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::store::layout::Layout;

use super::plan_ops::{self, ResolvedTarget};
use super::super::session::lookup;

pub(crate) struct ResolvedExecution {
    pub(crate) workstreams: Vec<WorkstreamDef>,
    pub(crate) plan_text: String,
}

pub(crate) fn resolve(
    layout: &Layout,
    feature: &FeatureName,
    session: Option<&str>,
    plan_text: &str,
    mut workstreams: Vec<WorkstreamDef>,
) -> Result<ResolvedExecution, Failure>
```

Resolution order for every workstream:

1. Parse plan targeting through `plan_ops::operations_from_plan(plan_text)?`.
2. If graph and plan both provide a field and values differ, return `execute.targeting_conflict` naming the workstream, field, and both values.
3. For `model` and `agent`, use the explicit graph/plan value or leave `None`.
4. For `provider`, use the explicit graph/plan value; otherwise use the caller session provider.
5. If no explicit provider exists and no caller session can be resolved, return `execute.provider_context_missing` with a fix action that names `--session`.
6. Produce `ResolvedTarget` values and call `plan_ops::write_targets` once after all workstreams resolve.

Resolve the caller session only when at least one workstream lacks an explicit provider:

```rust
fn caller_provider(
    layout: &Layout,
    feature: &FeatureName,
    session: Option<&str>,
) -> Result<Provider, Failure> {
    let session = session.ok_or_else(|| {
        Failure::blocked(
            "execute.provider_context_missing",
            "one or more workstreams have no provider and no caller session was supplied",
        )
        .expected("an explicit provider per workstream, or `--session <id>`")
        .actual("provider and caller session are both absent")
        .fix(FixAction::safe(
            "execute.pass_session",
            "Run prepare from `/ivar-execute`, or pass the current IVAR_SESSION_ID with `--session`.",
        ))
    })?;
    let session = lookup::resolve(layout, Some(session), Some(feature.as_str()))?;
    let state = session.state.ok_or_else(|| {
        Failure::blocked(
            "execute.session_state_missing",
            format!("session `{}` has no readable state.json", session.id),
        )
        .expected("a session record containing its provider")
        .actual("state.json is missing or unreadable")
    })?;
    Ok(state.provider())
}
```

Expose `targeting` privately from `src/action/execute/mod.rs`:

```rust
mod targeting;
```

- [ ] **Step 5: Reorder prepare so the fingerprint covers persisted targeting**

Add the session field:

```rust
pub struct PrepareInput {
    pub feature: String,
    pub graph_json: String,
    pub session: Option<String>,
}
```

Replace the current `prepare` ordering with:

```rust
let authored = read_workstreams(&graph_path)?;
let plan_path = layout.plan_dir(&feature).join("plan.md");
let plan_text = fs::read_text(&plan_path)?.ok_or_else(|| plan_missing(&feature))?;
let resolved = targeting::resolve(
    &layout,
    &feature,
    input.session.as_deref(),
    &plan_text,
    authored,
)?;
fs::write_text(&plan_path, &resolved.plan_text)?;
let plan_fingerprint = hash::file(&plan_path)?;
require_plan_backs_the_graph(&resolved.plan_text, &resolved.workstreams)?;
```

Change `require_plan_backs_the_graph` to accept the already-read text and resolved workstreams so it does not re-read stale content:

```rust
fn require_plan_backs_the_graph(
    plan_text: &str,
    workstreams: &[WorkstreamDef],
) -> Result<(), Failure> {
    for workstream in workstreams {
        super::prompt::render(plan_text, workstream, &[])?;
    }
    Ok(())
}
```

Construct `ExecutionGraph` from `resolved.workstreams`. Do not mutate `plan.md` after fingerprinting.

- [ ] **Step 6: Update existing PrepareInput constructions deliberately**

For fixtures that test unrelated behavior, make their graphs explicitly target `claude-code` and pass `session: None`. For fixtures intended to exercise inheritance, omit provider and pass a real session id. Update all 20 `PrepareInput` constructions found under:

```text
tests/unit/action/plan/status.rs
tests/unit/action/execute/prepare.rs
tests/unit/action/execute/approve.rs
tests/unit/action/execute/replan.rs
tests/unit/action/execute/ack.rs
tests/unit/action/execute/tick.rs
tests/unit/action/execute/reconcile.rs
src/cli/root.rs
```

This avoids manufacturing fake session context in tests whose subject is approval, replanning, acknowledgement, reconciliation, or status.

- [ ] **Step 7: Run prepare and dependent action tests**

Run:

```bash
cargo test --lib action::execute action::plan::status::tests
```

Expected: all selected tests pass; no test reaches a real provider binary.

- [ ] **Step 8: Commit the resolution slice**

```bash
git add src/action/execute tests/unit/action/execute tests/unit/action/plan/status.rs
git commit -m "feat(execute): resolve provider from caller session"
```

---

### Task 3: Pass the caller session through the CLI and shipped workflow

**Files:**
- Modify: `src/cli/root.rs:298-307,867-878`
- Modify: `src/harness/commands/execute.md`
- Modify: `tests/unit/harness/commands.rs`
- Modify: `docs/reference/commands.md`

- [ ] **Step 1: Add a failing Clap conversion test**

In the existing CLI parser tests, parse:

```text
ivar feature execute prepare checkout --graph-json /tmp/graph.json --session session-123
```

Assert that conversion yields:

```rust
PrepareInput {
    feature: "checkout".to_owned(),
    graph_json: "/tmp/graph.json".to_owned(),
    session: Some("session-123".to_owned()),
}
```

Run the nearest CLI test module:

```bash
cargo test --lib cli::root::tests -- --nocapture
```

Expected: FAIL because `--session` is not accepted.

- [ ] **Step 2: Add `--session` to prepare**

Extend `FeatureExecuteArgs`:

```rust
/// The current Ivar session whose provider supplies defaults for untargeted workstreams.
#[arg(long)]
pub session: Option<String>,
```

Map it without reading ambient environment inside the action:

```rust
impl From<FeatureExecuteArgs> for prepare::PrepareInput {
    fn from(args: FeatureExecuteArgs) -> Self {
        let FeatureExecuteArgs {
            feature,
            graph_json,
            session,
        } = args;
        Self {
            feature,
            graph_json,
            session,
        }
    }
}
```

- [ ] **Step 3: Make `/ivar-execute` persist and pass the provider**

Update `src/harness/commands/execute.md` so step 1 requires both `IVAR_SESSION_ID` and a readable `$IVAR_SESSION_PATH/state.json`. Before presenting the candidate graph:

1. Read `provider` from the current session's `state.json`.
2. Use it for every workstream without an explicit override.
3. Show the resolved provider, never `provider=—`.
4. Write the same `provider`, `model`, and `agent` values into each corresponding `### <workstream>` block in `plan.md`.
5. Re-run plan status if the workflow requires the changed plan to pass its approval gate.

Change the prepare command example to:

```text
ivar feature execute prepare <feature> --graph-json <path-to-candidate> --session "$IVAR_SESSION_ID"
```

Replace the old statement that omitted provider means the hall default with:

```text
Omitting provider in the initial candidate means “inherit the current Ivar
session provider.” Before approval, the resolved provider is made explicit in
both plan.md and board.json. No approved workstream relies on the hall default.
```

- [ ] **Step 4: Pin the shipped-command contract in tests**

In `tests/unit/harness/commands.rs`, assert that the embedded execute command contains all of:

```rust
assert!(execute.contains("IVAR_SESSION_ID"));
assert!(execute.contains("state.json"));
assert!(execute.contains("provider"));
assert!(execute.contains("--session"));
assert!(execute.contains("plan.md"));
```

Also assert it no longer describes an omitted provider as the hall default.

- [ ] **Step 5: Update generated command reference**

Regenerate or update `docs/reference/commands.md` so `execute prepare` documents:

```text
--session <SESSION>  Current Ivar session used to resolve providers for untargeted workstreams
```

Use the repository's existing reference-generation test rather than hand-maintaining generated output:

```bash
IVAR_UPDATE_DOCS=1 cargo test --test docs_reference
cargo test --test docs_reference
```

Expected: the first command updates the reference; the second passes with no drift.

- [ ] **Step 6: Run command and CLI tests**

```bash
cargo test --lib cli::root::tests harness::commands::tests
cargo test --test shipped_commands
```

Expected: all tests pass for both Claude Code and OpenCode command materialization.

- [ ] **Step 7: Commit the workflow slice**

```bash
git add src/cli/root.rs src/harness/commands/execute.md tests/unit/harness/commands.rs docs/reference/commands.md
git commit -m "feat(execute): carry session provider into preparation"
```

---

### Task 4: Prove tick launches the provider persisted from the session

**Files:**
- Modify: `src/action/execute/tick/mod.rs`
- Modify: `tests/unit/action/execute/tick.rs`
- Modify: `docs/guides/planning-and-execution.md`
- Modify: `docs/glossary.md`

- [ ] **Step 1: Add the end-to-end regression test**

Add a tick fixture whose hall default is Claude Code, caller session is OpenCode, and graph omits provider. Prepare through the new session-aware path, approve, then install only an `opencode` stub:

```rust
#[test]
fn tick_uses_the_provider_persisted_from_the_prepare_session() {
    let (_guard, root) = board_ready_for_session_targeting();
    let ctx = Ctx::new(root.clone());
    let session = feature_session(&root, Provider::OpenCode);

    prepare_action::prepare(
        &ctx,
        PrepareInput {
            feature: "checkout".to_owned(),
            graph_json: root.join("graph.json").to_string(),
            session: Some(session),
        },
    )
    .unwrap();
    approve_action::approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let (_sentinel_guard, sentinel_dir) = crate::test_support::utf8_temp_dir();
    let sentinel = sentinel_dir.join("opencode-ran");
    let _stub = PathStub::install("opencode", &format!(
        "cat >/dev/null\ntouch '{sentinel}'\n"
    ));

    tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    assert!(fs::is_file(&sentinel).unwrap());
    let board = persisted(&root);
    assert_eq!(board.graph.workstreams[0].provider, Some(Provider::OpenCode));
    let session_id = board.sessions.keys().next().unwrap();
    let view = Layout::at(root.clone())
        .feature_session(&FeatureName::new("checkout").unwrap(), &SessionId::new(session_id).unwrap());
    assert_eq!(SessionState::read(&view).unwrap().unwrap().provider(), Provider::OpenCode);
}
```

The `opencode` stub must consume stdin because OpenCode prompts are intentionally delivered there.

- [ ] **Step 2: Run the regression test before tightening tick**

```bash
cargo test --lib action::execute::tick::tests::tick_uses_the_provider_persisted_from_the_prepare_session -- --nocapture
```

Expected: PASS once Tasks 1–3 are complete. This proves the root-cause path independently of the next diagnostic guard.

- [ ] **Step 3: Make legacy fallback visible rather than silent**

Keep old boards readable, but when `tick` encounters `ws.provider == None`, add one warning per affected workstream before using the hall default:

```rust
warnings.push(Warning::new(
    "execute.legacy_provider_fallback",
    ws.id.clone(),
    format!(
        "workstream `{}` comes from a legacy board with no recorded provider; using hall default `{}`. Re-prepare the board to persist targeting in plan.md and board.json.",
        ws.id,
        manifest.providers().default_provider(),
    ),
));
```

Append this warning inside the existing job-construction loop, using the `warnings` collection that is already created before that loop. Do not move provider resolution into `launch.rs`: `tick/mod.rs` owns launch decisions, while `launch.rs` should continue receiving a resolved `LaunchJob.provider`.

Add a test that manually clears a prepared board's provider, ticks it, and asserts warning code `execute.legacy_provider_fallback`. Add a control asserting newly prepared boards emit no such warning.

- [ ] **Step 4: Update user documentation**

In `docs/guides/planning-and-execution.md`, document:

- session provider inheritance happens before board approval;
- the resolved value appears in both `plan.md` and `board.json`;
- explicit workstream provider overrides session inheritance;
- `model` and `agent` do not select the provider;
- old boards without a provider receive a visible legacy warning.

In `docs/glossary.md`, update the definitions of execution plan, workstream, provider, and session to state which artifact is authoritative at each stage.

- [ ] **Step 5: Run execution and documentation tests**

```bash
cargo test --lib action::execute::tick::tests
cargo test --test docs_reference
```

Expected: all tests pass, including both provider stubs and legacy warning coverage.

- [ ] **Step 6: Commit the regression and documentation slice**

```bash
git add src/action/execute/tick/mod.rs tests/unit/action/execute/tick.rs docs/guides/planning-and-execution.md docs/glossary.md
git commit -m "fix(execute): launch the session-selected provider"
```

---

### Task 5: Verify compatibility and the complete workflow

**Files:**
- Verify only; change files only when a failing check identifies a defect in the preceding tasks.

- [ ] **Step 1: Format and lint**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: both commands exit successfully with no warnings.

- [ ] **Step 2: Run the complete test suite**

```bash
cargo test --all-targets --all-features
```

Expected: all unit, integration, architecture, command-materialization, and documentation tests pass without invoking a real `claude` or `opencode` binary.

- [ ] **Step 3: Exercise the human-visible workflow with stubs**

Using a temporary hall:

1. Initialize the hall with default `claude-code` and add `opencode` as available.
2. Create and promote a feature.
3. Create an OpenCode feature-session record.
4. Author a plan and graph with no provider on one workstream.
5. Run `execute prepare ... --session <opencode-session>`.
6. Inspect `plan.md` and `board.json`; both must say `opencode`.
7. Approve and tick with an `opencode` stub on `PATH` and no `claude` stub.
8. Confirm the spawned session's `state.json` also says `opencode`.

Expected human output: preparation and approval succeed, tick launches the workstream through OpenCode, and no legacy fallback warning appears.

- [ ] **Step 4: Check the final diff for scope and generated drift**

```bash
git status --short
git diff --check
git diff --stat
```

Expected: only the files named in this plan are changed; `git diff --check` reports no whitespace errors.

- [ ] **Step 5: Commit any verification-only corrections as one logical unit**

If verification required a correction, stage only the corrected source file and its matching regression-test file, inspect the staged diff, and commit them with message `fix(execute): complete session provider targeting`. If every check passed without further edits, do not create an empty commit.

---

## Acceptance criteria

- A hall whose default is `claude-code` can prepare from an OpenCode session and subsequently launch `opencode` without an explicit graph provider.
- `plan.md`, `board.json`, and the launched session's `state.json` all record the same provider.
- Explicit provider overrides remain possible and visible before approval.
- `agent=implementer-deepseek` remains an agent selector; it never implicitly chooses a provider.
- Untargeted preparation without a caller session fails with a structured recovery message.
- Legacy boards remain readable, but their hall-default fallback is warned rather than hidden.
- Provider targeting is persisted before fingerprinting, so `tick` does not mutate the approved plan or create a false plan-divergence failure.
