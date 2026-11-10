//! Text checks over the shipped workflow commands and the env vars they name.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::shipped::shipped;

/// Every shipped command id, as `/ivar-<id>`.
const SHIPPED_IDS: [&str; 11] = [
    "connect",
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

#[test]
fn shipped_commands_do_not_reference_old_slash_execute() {
    for id in SHIPPED_IDS {
        let source = format!(
            "{}/src/harness/commands/{id}.md",
            env!("CARGO_MANIFEST_DIR")
        );
        let body = std::fs::read_to_string(source).unwrap();
        assert!(
            !body.contains("/ivar-execute"),
            "/ivar-{id} still references /ivar-execute"
        );
    }
}

#[test]
fn shipped_text_names_only_env_vars_ivar_exports() {
    for path in [
        "commands/feature-cleanup.md",
        "skills/ivar-explore/SKILL.md",
        "commands/connect.md",
    ] {
        assert!(
            !shipped(path).contains("IVAR_SESSION_TYPE"),
            "{path} uses an unset variable"
        );
    }
    assert!(
        !shipped("skills/ivar-explore/SKILL.md").contains("look in `.claude/skills/`"),
        "a view dir never holds the hall's ivar-plan skill"
    );
    assert!(
        shipped("commands/connect.md").contains("Claude Code"),
        "workdir is opencode/omp only"
    );
    assert!(shipped("commands/sync.md").contains(".omp/commands/"));
}

#[test]
fn connect_continues_in_the_current_t3_thread_with_absolute_paths() {
    let connect = shipped("commands/connect.md");
    assert!(!connect.contains("t3_project_create"));
    assert!(!connect.contains("t3_thread_launch"));
    assert!(!connect.contains("outside the write guard"));
    assert!(!connect.contains("(a) Continue here"));
    assert!(connect.contains("absolute paths under `$IVAR_SESSION_PATH`"));
    assert!(connect.contains("Never create a project or open a new thread"));
}
