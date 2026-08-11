//! Black-box lifecycle coverage for the repository-relations journey and the
//! canonical hall instructions, driving the compiled binary.
//!
//! The unit tests cover the reconciler, the actions, and the session
//! materialiser against temp directories. These tests exist for what only the
//! real process can prove: init and provider add materialise the canonical
//! `HALL.md` and its aliases immediately, sync repairs and destroys topology,
//! doctor reports every drift in one pass, sessions derive their instruction
//! files from the canonical bytes, and all fifteen workflow commands land for
//! both providers.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/integration.rs"]
mod common;

use camino::Utf8Path;
use common::{hall_root, ivar, seeded_repo};
use predicates::prelude::*;

/// The fifteen shipped command ids.
const SHIPPED_IDS: [&str; 15] = [
    "deliver",
    "discovery",
    "execute",
    "feature-create",
    "feature-status",
    "plan",
    "promote",
    "relations",
    "repo-list",
    "repo-setup",
    "review",
    "session-connect",
    "session-start",
    "session-stop",
    "sync",
];

/// What the root alias at `path` points at, as read by `readlink` — the
/// symlink target itself, not the file it resolves to.
fn symlink_target(root: &Utf8Path, name: &str) -> std::path::PathBuf {
    std::fs::read_link(root.join(name)).unwrap_or_else(|error| {
        panic!("`{name}` should be a symlink at {root}: {error}");
    })
}

/// A hall with both providers registered, so both aliases exist.
fn hall_with_both_providers() -> (tempfile::TempDir, camino::Utf8PathBuf) {
    let (guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["provider", "add", "opencode"])
        .assert()
        .success();
    (guard, root)
}

// -- init -----------------------------------------------------------------

#[test]
fn init_creates_hall_md_and_the_selected_relative_alias() {
    let (_guard, root) = hall_root();

    ivar().current_dir(&root).arg("init").assert().success();

    assert!(root.join("HALL.md").is_file());
    assert_eq!(
        symlink_target(&root, "CLAUDE.md"),
        std::path::Path::new("HALL.md"),
        "the selected provider's alias must be a relative symlink to HALL.md"
    );
    assert!(
        !root.join("AGENTS.md").exists(),
        "an OpenCode alias must not exist for a Claude hall"
    );
}

// -- repo add -------------------------------------------------------------

#[test]
fn repo_add_exposes_the_relations_next_action() {
    // JSON surface: the structured next action.
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
    let json = ivar()
        .current_dir(&root)
        .args(["repo", "add", "api", origin.as_str(), "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(value["next_action"], "/ivar-relations api");

    // Human surface: the same invitation, rendered from the same value.
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let origin = seeded_repo(&root.parent().unwrap().join("origins").join("web"), "main");
    let human = ivar()
        .current_dir(&root)
        .args(["repo", "add", "web", origin.as_str()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(
        String::from_utf8(human)
            .unwrap()
            .contains("Next: run `/ivar-relations web`"),
        "the human surface must carry the same next action"
    );
}

// -- provider add ---------------------------------------------------------

#[test]
fn provider_add_creates_the_second_alias_immediately() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    ivar()
        .current_dir(&root)
        .args(["provider", "add", "opencode"])
        .assert()
        .success();

    assert_eq!(
        symlink_target(&root, "AGENTS.md"),
        std::path::Path::new("HALL.md"),
        "adding OpenCode must immediately create its relative alias"
    );
    assert_eq!(
        symlink_target(&root, "CLAUDE.md"),
        std::path::Path::new("HALL.md"),
        "the existing alias must be untouched"
    );
}

// -- sync -----------------------------------------------------------------

#[test]
fn sync_repairs_a_wrong_symlink_and_preserves_an_enabled_regular_alias() {
    let (_guard, root) = hall_with_both_providers();
    // Wrong-target symlink for Claude; a regular file for OpenCode.
    std::fs::write(root.join("other.md"), "x").unwrap();
    std::fs::remove_file(root.join("CLAUDE.md")).unwrap();
    std::os::unix::fs::symlink("other.md", root.join("CLAUDE.md")).unwrap();
    std::fs::remove_file(root.join("AGENTS.md")).unwrap();
    std::fs::write(root.join("AGENTS.md"), "legacy, precious\n").unwrap();

    ivar()
        .current_dir(&root)
        .arg("sync")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("AGENTS.md alias needs a decision"));

    assert_eq!(
        symlink_target(&root, "CLAUDE.md"),
        std::path::Path::new("HALL.md"),
        "sync must repair a wrong-target enabled symlink"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("AGENTS.md")).unwrap(),
        "legacy, precious\n",
        "sync must preserve an enabled regular alias byte for byte"
    );
}

#[test]
fn disabling_a_provider_by_hand_makes_sync_delete_its_regular_alias() {
    let (_guard, root) = hall_with_both_providers();
    ivar().current_dir(&root).arg("sync").assert().success();
    // Drop OpenCode from the manifest by hand.
    std::fs::write(
        root.join("ivar.json"),
        r#"{"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[],"version":1}"#,
    )
    .unwrap();
    std::fs::remove_file(root.join("AGENTS.md")).unwrap();
    std::fs::write(root.join("AGENTS.md"), "regular file\n").unwrap();

    ivar().current_dir(&root).arg("sync").assert().success();

    assert!(
        !root.join("AGENTS.md").exists(),
        "a disabled provider's alias path is entirely ivar-managed"
    );
    assert!(
        root.join("HALL.md").is_file(),
        "the canonical file must always survive"
    );
}

#[test]
fn all_fifteen_workflow_commands_materialise_for_both_providers() {
    let (_guard, root) = hall_with_both_providers();

    for provider in ["claude-code", "opencode"] {
        let commands_dir = if provider == "claude-code" {
            root.join(".claude/commands")
        } else {
            root.join(".opencode/commands")
        };
        for id in SHIPPED_IDS {
            assert!(
                commands_dir.join(format!("ivar-{id}.md")).is_file(),
                "{provider}: ivar-{id}.md must be materialised"
            );
        }
    }
}

// -- doctor ---------------------------------------------------------------

#[test]
fn doctor_reports_every_instruction_drift_in_one_run() {
    let (_guard, root) = hall_with_both_providers();
    ivar().current_dir(&root).arg("sync").assert().success();
    // Drift every coexisting shape at once: canonical non-regular, one
    // enabled regular alias, one broken enabled alias.
    std::fs::remove_file(root.join("HALL.md")).unwrap();
    std::fs::create_dir(root.join("HALL.md")).unwrap();
    std::fs::remove_file(root.join("CLAUDE.md")).unwrap();
    std::fs::write(root.join("CLAUDE.md"), "legacy, precious\n").unwrap();
    std::fs::remove_file(root.join("AGENTS.md")).unwrap();
    std::os::unix::fs::symlink("vanished.md", root.join("AGENTS.md")).unwrap();

    ivar()
        .current_dir(&root)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "instructions.canonical_not_regular",
        ))
        .stdout(predicate::str::contains("instructions.alias_regular"))
        .stdout(predicate::str::contains("instructions.alias_broken"));
}

// -- sessions -------------------------------------------------------------

/// The view dir a detached session produced, from its `--json` output. The
/// exit code is deliberately not asserted — the tests decide whether the run
/// should be clean or carry the canonical-unavailable warning.
fn detached_session_view_dir(
    root: &Utf8Path,
    args: &[&str],
) -> (serde_json::Value, camino::Utf8PathBuf) {
    let mut command = ivar();
    command.current_dir(root).arg("session").arg("start");
    for arg in args {
        command.arg(arg);
    }
    let output = command
        .args(["--detached", "--provider", "claude-code", "--json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let view_dir = camino::Utf8PathBuf::from(value["view_dir"].as_str().unwrap());
    (value, view_dir)
}

#[test]
fn discovery_and_feature_sessions_use_hall_bytes_not_alias_bytes() {
    // A hall whose root alias is broken: the session file must still come
    // from HALL.md, never from the alias.
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "checkout"])
        .assert()
        .success();
    let hall = std::fs::read_to_string(root.join("HALL.md")).unwrap();
    std::fs::remove_file(root.join("CLAUDE.md")).unwrap();
    std::fs::write(root.join("CLAUDE.md"), "alias bytes that must not leak\n").unwrap();

    // Discovery session: its CLAUDE.md equals HALL.md.
    let (_value, view_dir) = detached_session_view_dir(&root, &[]);
    assert_eq!(
        std::fs::read_to_string(view_dir.join("CLAUDE.md")).unwrap(),
        hall,
        "a discovery session's file must be the canonical content"
    );

    // Feature session: bootstrap + HALL.md, still no alias bytes.
    let (_value, view_dir) = detached_session_view_dir(&root, &["checkout"]);
    let instructions = std::fs::read_to_string(view_dir.join("CLAUDE.md")).unwrap();
    assert!(
        instructions.contains("ivar session — feature `checkout`"),
        "the bootstrap must lead the feature session file"
    );
    assert!(
        instructions.contains("managed:start"),
        "the canonical content must follow the bootstrap"
    );
    assert!(
        !instructions.contains("alias bytes"),
        "alias bytes must never leak into a session file"
    );
}

#[test]
fn missing_hall_warns_but_session_materialisation_succeeds() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "checkout"])
        .assert()
        .success();
    std::fs::remove_file(root.join("HALL.md")).unwrap();

    // Discovery: warns, opens, and carries no shared content.
    let (value, view_dir) = detached_session_view_dir(&root, &[]);
    let warnings = value["warnings"].as_array().unwrap();
    assert_eq!(warnings[0]["code"], "instructions.canonical_unavailable");
    assert!(
        !view_dir.join("CLAUDE.md").exists(),
        "a discovery session with no canonical content writes nothing"
    );

    // Feature: the same warning, and bootstrap only.
    let (value, view_dir) = detached_session_view_dir(&root, &["checkout"]);
    let warnings = value["warnings"].as_array().unwrap();
    assert_eq!(warnings[0]["code"], "instructions.canonical_unavailable");
    let instructions = std::fs::read_to_string(view_dir.join("CLAUDE.md")).unwrap();
    assert!(
        instructions.contains("ivar session — feature `checkout`"),
        "the bootstrap must still land"
    );
    assert!(
        !instructions.contains("managed:start"),
        "without HALL.md there is no canonical content"
    );
}
