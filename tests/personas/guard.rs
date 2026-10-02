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
