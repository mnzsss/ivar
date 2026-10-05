//! The session write guard, driven through `ivar guard` as a provider hook.

use camino::Utf8PathBuf;

use super::support::{home, isolated_ivar, one_repo_hall};
use crate::common::hall_root;

#[test]
fn the_guard_hook_denies_a_dotdot_escape_through_a_missing_directory() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    let view = root.join(".ivar/sessions/6f0c9d5f-0000-4000-8000-000000000030");
    std::fs::create_dir_all(&view).unwrap();
    std::fs::write(
        view.join("state.json"),
        r#"{"version":1,"provider":"claude-code","started_at":"2026-10-02T00:00:00Z"}"#,
    )
    .unwrap();
    let decide = |file: Utf8PathBuf| {
        let payload = serde_json::json!({
            "tool_name": "Write",
            "tool_input": { "file_path": file },
            "cwd": view,
        });
        let output = isolated_ivar(&home(&root))
            .current_dir(&root)
            .args(["guard", "--provider", "claude-code"])
            .write_stdin(payload.to_string())
            .output()
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        body["hookSpecificOutput"]["permissionDecision"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    assert_eq!(decide(view.join("notes.md")), "allow");
    assert_eq!(
        decide(view.join("missing/../../../../.git/hooks/pre-commit")),
        "deny"
    );
}

#[test]
fn omp_hashline_edit_inside_promoted_worktree_from_hall_root_cwd_is_allowed() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    let view = root.join(".ivar/sessions/6f0c9d5f-0000-4000-8000-000000000030");
    std::fs::create_dir_all(&view).unwrap();
    std::fs::write(
        view.join("state.json"),
        r#"{"version":1,"provider":"omp","started_at":"2026-10-02T00:00:00Z"}"#,
    )
    .unwrap();

    let target_file = view.join("notes.md");
    let payload = serde_json::json!({
        "tool": "edit",
        "args": {
            "input": format!("[{target_file}#ABCD]\nPUT 1:\n+new content\n")
        },
        "cwd": root,
    });
    let output = isolated_ivar(&home(&root))
        .current_dir(&root)
        .args(["guard", "--provider", "omp"])
        .write_stdin(payload.to_string())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "hashline edit targeting session view must be allowed"
    );
}

#[test]
fn omp_multi_header_hashline_denies_if_one_target_is_in_unpromoted_repo() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    let view = root.join(".ivar/sessions/6f0c9d5f-0000-4000-8000-000000000030");
    std::fs::create_dir_all(&view).unwrap();
    std::fs::write(
        view.join("state.json"),
        r#"{"version":1,"provider":"omp","started_at":"2026-10-02T00:00:00Z"}"#,
    )
    .unwrap();

    let payload = serde_json::json!({
        "tool": "edit",
        "args": {
            "input": format!("[{}/notes.md#ABCD]\nPUT 1:\n+ok\n[/etc/passwd#1234]\nPUT 1:\n+bad\n", view)
        },
        "cwd": view,
    });
    let output = isolated_ivar(&home(&root))
        .current_dir(&root)
        .args(["guard", "--provider", "omp"])
        .write_stdin(payload.to_string())
        .output()
        .unwrap();

    assert!(!output.status.success(), "exit non-zero on denial");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("/etc/passwd"),
        "denial should name the disallowed target: {stdout}"
    );
}

#[test]
fn omp_mv_outside_writable_set_is_denied() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    let view = root.join(".ivar/sessions/6f0c9d5f-0000-4000-8000-000000000030");
    std::fs::create_dir_all(&view).unwrap();
    std::fs::write(
        view.join("state.json"),
        r#"{"version":1,"provider":"omp","started_at":"2026-10-02T00:00:00Z"}"#,
    )
    .unwrap();

    let payload = serde_json::json!({
        "tool": "edit",
        "args": {
            "input": format!("[{}/notes.md#ABCD]\nMV \"/etc/shadow\"\n", view)
        },
        "cwd": view,
    });
    let output = isolated_ivar(&home(&root))
        .current_dir(&root)
        .args(["guard", "--provider", "omp"])
        .write_stdin(payload.to_string())
        .output()
        .unwrap();

    assert!(!output.status.success());
}
