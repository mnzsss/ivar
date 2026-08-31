//! Black-box lifecycle tests for the shipped workflow commands, driving the
//! compiled binary.
//!
//! The reconciliation behaviour itself is unit-tested in
//! `src/harness/commands.rs` (with the catalog in
//! `src/harness/commands/catalog.rs`) against temp directories. These tests exist for
//! what only the real process can prove: `init` and `provider add` bootstrap
//! commands without a follow-up sync, sync repairs and removes them at the
//! hall level, and the commands never disturb hall health.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/integration.rs"]
mod common;

use camino::Utf8Path;
use common::{hall_root, ivar};
use predicates::prelude::*;

/// Every shipped command id, as `/ivar-<id>`.
const SHIPPED_IDS: [&str; 14] = [
    "connect",
    "deliver",
    "discovery",
    "execute",
    "feature-cleanup",
    "feature-create",
    "feature-status",
    "plan",
    "promote",
    "relations",
    "repo-list",
    "repo-setup",
    "review",
    "sync",
];

/// The exact bytes of the Bifrost-era `plan` command — its SHA-256 is the
/// catalog's legacy fingerprint for `plan`, which is what lets `sync` remove
/// the unprefixed file (and only the file whose digest matches).
const LEGACY_PLAN: &str = r#"---
description: Conduct the SPDD planning process — Requirements, Analysis, Plan, and approval gates.
argument-hint: <feature-name>
---

Run the full SPDD planning process for a feature. This skill conducts three planning phases with human approval gates between each.

## Prerequisites

- You must be inside a **Feature Session** (`BIFROST_FEATURE` must be set).
- The feature must exist (`bifrost hall feature list`).
- Start a new SPDD flow with `bifrost hall plan init --feature <name>` to scaffold the planning artifacts.

## Process Overview

The SPDD planning lifecycle has three artifacts with four approval gates:

```
Requirements → [approve-requirements] → Analysis → [approve-analysis] → Plan → [approve-plan] → Graph → [approve-graph] → Execution
```

Each artifact lives committed under `<hall>/plans/<feature>/`. Once an artifact is approved, changing it cascades invalidation to downstream artifacts.

## Phase 1: Requirements

1. Research the feature and its context (repos, existing code, user needs).

2. Write the Requirements artifact to `plans/<feature>/requirements.md`. Include:
   - Functional requirements (R-* IDs: R-LOGIN, R-AUTH, etc.)
   - Non-functional requirements (performance, security)
   - Constraints

3. Call `bifrost hall plan submit --feature <name> --artifact requirements`.

4. **Pause for human approval.** Show the requirements to the user. Only proceed after they approve.

5. Call `bifrost hall plan approve-requirements --feature <name>`.

## Phase 2: Analysis

1. With approved Requirements as context, analyze the codebase to determine:
   - Affected modules (repo + path + impact level)
   - Trade-offs between approaches
   - Risks and mitigations
   - Recommendations

2. Write the Analysis artifact to `plans/<feature>/analysis.md`.

3. Call `bifrost hall plan submit --feature <name> --artifact analysis`.

4. **Pause for human approval.** Show the analysis to the user. Only proceed after they approve.

5. Call `bifrost hall plan approve-analysis --feature <name>`.

## Phase 3: Plan

1. Synthesize the Requirements and Analysis into a structured plan. Include:
   - **Requirements** section referencing the artifact
   - **Entities** — domain model (delta only; reference CONTEXT.md)
   - **Approach** — the chosen design approach
   - **Structure** — file/module organization
   - **Operations** — concrete, testable steps with OP-* IDs:
     - Each operation has: id, title, description, dependsOn, touches, tests, doneWhen
     - Operation IDs follow the format `OP-<SLUG>` (e.g., `OP-API-CONTRACT`)
     - Touch sets are file paths identifying what files are affected
   - **Norms** — coding conventions to follow
   - **Safeguards** — things to watch out for

2. Write the Plan artifact to `plans/<feature>/plan.md`.

3. Call `bifrost hall plan submit --feature <name> --artifact plan`.

4. **Pause for human approval.** Show the plan to the user. Only proceed after they approve.

5. Call `bifrost hall plan approve-plan --feature <name>`.

## Phase 4: Execution Graph

After the plan is approved, the execution graph must be approved separately:

1. Call `bifrost hall feature execute prepare --feature <name> --plan <plan-path> --graph-json <path>`.
2. When `status=awaiting_approval`, show the generated graph to the user.
3. After approval, call `bifrost hall plan approve-graph --feature <name>` or `bifrost hall feature execute approve --feature <name>`.

## Checking Status

At any point, check approval gate status:
`bifrost hall plan status --feature <name>`

## Important

- **Never hand-edit** approvals in `.features/<name>/planning/approvals.json`. Always use the CLI commands.
- Changing an upstream artifact (Requirements → Analysis → Plan) automatically marks downstream gates as `needs_revision`.
- Behavior-changing plan edits (Operations or Approach changes) require re-approval of affected gates.
- The plan skill respects the **REASONS Canvas** format: design sections reference standing sources and record only the feature's delta.
- **Replan mode**: If execution is in-flight and the Plan needs structural changes, use `bifrost hall plan submit --artifact plan` to produce a new revision. Behavior-changing revisions pause affected workstreams until each acknowledges via `bifrost hall feature execute ack-revision --feature <name> --workstream <id>`. Execution resumes only after all affected workstreams acknowledge.
- **Reconcile mode**: For local code divergence confined to an operation's implementation, record the deviation in the execution journal and update the Plan via submit. Requires user acceptance before writing.
"#;

/// Rewrite a hall's `ivar.json` to list exactly `available` providers, no
/// repos. Hand-written because the manifest being hand-editable is the
/// contract, and there is no provider-removal verb.
fn rewrite_manifest(root: &Utf8Path, available: &[&str]) {
    let list = available
        .iter()
        .map(|provider| format!("\"{provider}\""))
        .collect::<Vec<_>>()
        .join(",");
    std::fs::write(
        root.join("ivar.json"),
        format!(
            r#"{{"name":"acme","providers":{{"available":[{list}],"default":"claude-code"}},"repos":[],"version":1}}"#
        ),
    )
    .unwrap();
}

/// The `.claude/commands/ivar-*.md` files exist after `ivar init`.
#[test]
fn init_materialises_the_selected_providers_commands() {
    let (_guard, root) = hall_root();
    ivar()
        .current_dir(&root)
        .args(["init", "--provider", "claude-code"])
        .assert()
        .success();

    for id in SHIPPED_IDS {
        assert!(
            root.join(".claude/commands")
                .join(format!("ivar-{id}.md"))
                .is_file(),
            "{id} should be materialised by init"
        );
    }
    assert!(
        !root.join(".opencode").exists(),
        "a claude-code hall must not create an opencode command directory"
    );
}

/// The shipped bytes make the provider the coordinator while marking each wave
/// complete in the plan and isolating new scope in a child feature.
#[test]
fn shipped_commands_encode_wave_completion_and_native_coordination() {
    let (_guard, root) = hall_root();
    ivar()
        .current_dir(&root)
        .args(["init", "--provider", "claude-code"])
        .assert()
        .success();

    let read = |id: &str| {
        std::fs::read_to_string(root.join(".claude/commands").join(format!("ivar-{id}.md")))
            .unwrap_or_default()
    };
    let collapsed = |text: String| text.split_whitespace().collect::<Vec<_>>().join(" ");

    let feature_create = collapsed(read("feature-create"));
    assert!(feature_create.contains("`ivar feature create <child> --parent <current>`"));
    assert!(feature_create.contains("announce"));
    assert!(feature_create.contains("do not ask permission"));

    let plan = collapsed(read("plan"));
    assert!(plan.contains("three planning phases"));
    assert!(plan.contains("[approve plan] → Execution"));
    assert!(!plan.contains("approve graph"));

    let execute = collapsed(read("execute"));
    assert!(execute.contains("active provider coordinates its own native subagents"));
    assert!(execute.contains("wave checkpoint"));
    assert!(execute.contains("mark the wave complete"));
    assert!(execute.contains("child Feature"));
    assert!(!execute.contains("workstream"));
    assert!(!execute.contains("execute tick"));
    assert!(!execute.contains("ivar feature execute start"));
    assert!(!execute.contains("ivar feature execute finish"));
    assert!(execute.contains("If newly discovered work is outside the approved plan"));
    assert!(execute.contains("create a child Feature"));
}

/// `ivar provider add` materialises the new provider's commands immediately —
/// no follow-up sync.
#[test]
fn provider_add_materialises_the_new_providers_commands_immediately() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["provider", "add", "opencode"])
        .assert()
        .success();

    for id in SHIPPED_IDS {
        assert!(
            root.join(".opencode/commands")
                .join(format!("ivar-{id}.md"))
                .is_file(),
            "{id} should be materialised without a follow-up sync"
        );
    }
}

/// A user's command file survives sync, and survives the provider being
/// dropped from the manifest (which removes only the `ivar-*` files).
#[test]
fn a_user_command_survives_sync_and_provider_removal() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["provider", "add", "opencode"])
        .assert()
        .success();
    let custom = root.join(".opencode/commands/custom.md");
    std::fs::write(&custom, "my own command\n").unwrap();

    ivar().current_dir(&root).arg("sync").assert().success();
    assert_eq!(
        std::fs::read_to_string(&custom).unwrap(),
        "my own command\n",
        "sync must not touch a user command"
    );

    rewrite_manifest(&root, &["claude-code"]);
    ivar().current_dir(&root).arg("sync").assert().success();
    assert!(
        !root.join(".opencode/commands/ivar-plan.md").exists(),
        "a dropped provider's shipped commands must be removed"
    );
    assert_eq!(
        std::fs::read_to_string(&custom).unwrap(),
        "my own command\n",
        "provider removal must not touch a user command"
    );
}

/// A modified shipped command is restored by sync.
#[test]
fn sync_restores_a_modified_shipped_command() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    std::fs::write(root.join(".claude/commands/ivar-plan.md"), "tampered\n").unwrap();

    ivar().current_dir(&root).arg("sync").assert().success();

    let restored = std::fs::read_to_string(root.join(".claude/commands/ivar-plan.md")).unwrap();
    assert!(restored.starts_with("---\n"), "was: {restored:?}");
    assert!(restored.contains("description:"), "was: {restored:?}");
}

/// A fingerprint-matching legacy `plan.md` is removed by sync; a customised
/// one survives and appears in `ivar doctor`.
#[test]
fn fingerprint_matching_legacy_command_is_removed_and_modified_one_is_diagnosed() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    // The exact official artifact: sync removes it.
    std::fs::write(root.join(".claude/commands/plan.md"), LEGACY_PLAN).unwrap();
    ivar().current_dir(&root).arg("sync").assert().success();
    assert!(
        !root.join(".claude/commands/plan.md").exists(),
        "a fingerprint-matching legacy command must be removed"
    );

    // A customised one is preserved, and doctor names it.
    std::fs::write(
        root.join(".claude/commands/plan.md"),
        format!("{LEGACY_PLAN}x"),
    )
    .unwrap();
    ivar().current_dir(&root).arg("sync").assert().success();
    assert!(
        root.join(".claude/commands/plan.md").is_file(),
        "a customised legacy command must survive sync"
    );
    ivar()
        .current_dir(&root)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("provider.legacy_command_modified"));
}

/// A missing convenience command is not structural degradation: `ivar status`
/// stays `operational`.
#[test]
fn status_stays_operational_when_a_shipped_command_is_missing() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    std::fs::remove_file(root.join(".claude/commands/ivar-plan.md")).unwrap();

    ivar()
        .current_dir(&root)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("operational"));
}

/// The shipped execution workflow names only the Run Receipt lifecycle verbs.
#[test]
fn execute_help_exposes_the_receipt_lifecycle_without_legacy_verbs() {
    let (_guard, root) = hall_root();
    ivar()
        .current_dir(&root)
        .args(["feature", "execute", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("start"))
        .stdout(predicate::str::contains("finish"))
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("accept-revision"))
        .stdout(predicate::str::contains("replan").not())
        .stdout(predicate::str::contains("prepare").not())
        .stdout(predicate::str::contains("tick").not());
}

#[test]
fn no_shipped_command_tells_the_agent_to_export_ivar_vars() {
    for id in SHIPPED_IDS {
        let source = format!(
            "{}/src/harness/commands/{id}.md",
            env!("CARGO_MANIFEST_DIR")
        );
        let body = std::fs::read_to_string(source).unwrap();
        assert!(
            !body.contains("export IVAR_"),
            "/ivar-{id} still tells the agent to export IVAR_* vars"
        );
    }
}
