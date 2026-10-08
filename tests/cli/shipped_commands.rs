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

use crate::common::{hall_root, ivar};
use camino::Utf8Path;
use predicates::prelude::*;

/// Every shipped command id, as `/ivar-<id>`.
const SHIPPED_IDS: [&str; 12] = [
    "connect",
    "discovery",
    "feature-cleanup",
    "feature-create",
    "feature-status",
    "promote",
    "relations",
    "repo-list",
    "repo-setup",
    "review",
    "sync",
    "workspace",
];

/// The exact bytes of the Bifrost-era `repo-list` command — its SHA-256 is the
/// catalog's legacy fingerprint for `repo-list`, which is what lets `sync`
/// remove the unprefixed file (and only the file whose digest matches).
const LEGACY_REPO_LIST: &str = "# Repo List\n\
        \n\
        List all repositories registered in the hall manifest, along with active sessions\n\
        and promoted repos.\n\
        \n\
        ## Usage\n\
        \n\
        ```bash\n\
        bifrost hall status\n\
        ```\n\
        \n\
        ## Output\n\
        \n\
        Shows all repos with their name, default branch, and URL. Also shows features,\n\
        sessions, lifecycle state, and promoted repos per feature.\n";

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

    let plan_dir = root.join(".claude/skills/ivar-plan");
    let plan = collapsed(
        [
            "SKILL.md",
            "references/plan-template.md",
            "references/task-template.md",
        ]
        .iter()
        .map(|file| std::fs::read_to_string(plan_dir.join(file)).unwrap())
        .collect::<Vec<_>>()
        .join("\n"),
    );
    assert!(plan.contains("three planning phases"));
    assert!(plan.contains("[approve plan] → Execution"));
    assert!(!plan.contains("approve graph"));
    assert!(plan.contains("Step 1 carries the test's literal source"));
    assert!(plan.contains("**Sketch:**"));
    assert!(plan.contains("Literal Code"));
    assert!(!root.join(".claude/commands/ivar-plan.md").exists());
    assert!(!root.join(".claude/commands/ivar-execute.md").exists());
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
        !root.join(".opencode/commands/ivar-review.md").exists(),
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
    std::fs::write(root.join(".claude/commands/ivar-review.md"), "tampered\n").unwrap();

    ivar().current_dir(&root).arg("sync").assert().success();

    let restored = std::fs::read_to_string(root.join(".claude/commands/ivar-review.md")).unwrap();
    assert!(restored.starts_with("---\n"), "was: {restored:?}");
    assert!(restored.contains("description:"), "was: {restored:?}");
}

/// Provider sync materialises shipped skills (e.g. `ivar-execute/SKILL.md`) for available providers,
/// removes them when unavailable, and `ivar doctor` inspects them.
#[test]
fn sync_materialises_shipped_skills_and_doctor_inspects_them() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["provider", "add", "opencode"])
        .assert()
        .success();
    ivar()
        .current_dir(&root)
        .args(["provider", "add", "omp"])
        .assert()
        .success();

    ivar().current_dir(&root).arg("sync").assert().success();

    for dir in [".claude/skills", ".opencode/skills", ".omp/skills"] {
        assert!(
            root.join(dir).join("ivar-execute/SKILL.md").is_file(),
            "expected {dir}/ivar-execute/SKILL.md to exist after sync"
        );
    }

    // Doctor on a healthy setup reports no skill findings
    ivar()
        .current_dir(&root)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("provider.skill_missing").not())
        .stdout(predicate::str::contains("provider.skill_modified").not());

    // Tamper with a skill file and verify doctor catches it
    std::fs::write(
        root.join(".claude/skills/ivar-execute/SKILL.md"),
        "tampered skill content\n",
    )
    .unwrap();
    ivar()
        .current_dir(&root)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("provider.skill_modified"));

    // Sync repairs it
    ivar().current_dir(&root).arg("sync").assert().success();
    let restored =
        std::fs::read_to_string(root.join(".claude/skills/ivar-execute/SKILL.md")).unwrap();
    assert!(restored.starts_with("---\n"));
    assert!(restored.contains("name: ivar-execute"));
}

/// A fingerprint-matching legacy `repo-list.md` is removed by sync; a customised
/// one survives and appears in `ivar doctor`.
#[test]
fn fingerprint_matching_legacy_command_is_removed_and_modified_one_is_diagnosed() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    // The exact official artifact: sync removes it.
    std::fs::write(root.join(".claude/commands/repo-list.md"), LEGACY_REPO_LIST).unwrap();
    ivar().current_dir(&root).arg("sync").assert().success();
    assert!(
        !root.join(".claude/commands/repo-list.md").exists(),
        "a fingerprint-matching legacy command must be removed"
    );

    // A customised one is preserved, and doctor names it.
    std::fs::write(
        root.join(".claude/commands/repo-list.md"),
        format!("{LEGACY_REPO_LIST}x"),
    )
    .unwrap();
    ivar().current_dir(&root).arg("sync").assert().success();
    assert!(
        root.join(".claude/commands/repo-list.md").is_file(),
        "a customised legacy command must survive sync"
    );
    ivar()
        .current_dir(&root)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("provider.legacy_command_modified"));
}

/// A hall synced before `ivar-deliver` became a skill loses the stale command
/// and gains the skill.
#[test]
fn sync_replaces_the_deliver_command_with_the_deliver_skill() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    std::fs::write(root.join(".claude/commands/ivar-deliver.md"), "stale\n").unwrap();

    ivar().current_dir(&root).arg("sync").assert().success();

    assert!(!root.join(".claude/commands/ivar-deliver.md").exists());
    assert!(root.join(".claude/skills/ivar-deliver/SKILL.md").is_file());
}

/// A missing convenience command is not structural degradation: `ivar status`
/// stays `operational`.
#[test]
fn status_stays_operational_when_a_shipped_command_is_missing() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    std::fs::remove_file(root.join(".claude/commands/ivar-review.md")).unwrap();

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

/// `ivar provider remove` strips the provider's `ivar-*` commands and never
/// touches a command file the user wrote.
#[test]
fn provider_remove_strips_ivar_commands_and_keeps_user_commands() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["provider", "add", "opencode"])
        .assert()
        .success();
    std::fs::write(root.join(".opencode/commands/mine.md"), "mine\n").unwrap();

    ivar()
        .current_dir(&root)
        .args(["provider", "remove", "opencode"])
        .assert()
        .success();

    for id in SHIPPED_IDS {
        assert!(
            !root
                .join(".opencode/commands")
                .join(format!("ivar-{id}.md"))
                .exists(),
            "{id} must be removed with the provider"
        );
    }
    assert_eq!(
        std::fs::read_to_string(root.join(".opencode/commands/mine.md")).unwrap(),
        "mine\n"
    );
}

/// Removing the default provider without `--default` is refused.
#[test]
fn provider_remove_of_the_default_requires_a_new_default() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["provider", "add", "opencode"])
        .assert()
        .success();

    ivar()
        .current_dir(&root)
        .args(["provider", "remove", "claude-code"])
        .assert()
        .failure();
    ivar()
        .current_dir(&root)
        .args(["provider", "remove", "claude-code", "--default", "opencode"])
        .assert()
        .success();
    assert!(!root.join(".claude/settings.json").exists());
}
