//! Repository instructions reach the agent through `ivar guard`: the compiled
//! binary, a real session view dir, and a promotion in the middle of the session.

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::{Value, json};

use super::support::{home, isolated_ivar, run_ok};
use crate::common::{declare_repos, git, hall_root, seeded_repo};

const MAIN_CLAUDE: &str = "# app\n\n- Run app locally only with `pnpm serve:alpha` [MAIN-41].\n";
const MAIN_AGENTS: &str =
    "# app (agents)\n\n- Run app locally only with `pnpm serve:alpha` [MAIN-AGENTS-41].\n";
const FEATURE_CLAUDE: &str = "# app\n\n- Run app locally only with `pnpm serve:bravo` [FEAT-92].\n";

/// A synced hall whose one repo `app` has root `CLAUDE.md` + `AGENTS.md` and
/// `src/lib.rs` committed on `main`, with feature `checkout` created (not promoted).
fn hall_with_instructed_repo(root: &Utf8Path) {
    run_ok(root, &["init"]);
    let origin = seeded_repo(&root.parent().unwrap().join("origins").join("app"), "main");
    std::fs::create_dir_all(origin.join("src")).unwrap();
    std::fs::write(origin.join("src/lib.rs"), "pub fn app() {}\n").unwrap();
    std::fs::write(origin.join("CLAUDE.md"), MAIN_CLAUDE).unwrap();
    std::fs::write(origin.join("AGENTS.md"), MAIN_AGENTS).unwrap();
    git(&origin, &["add", "-A"]);
    git(&origin, &["commit", "-m", "instructions"]);
    declare_repos(root, &[("app", origin.as_path(), "main")]);
    run_ok(root, &["sync"]);
    run_ok(root, &["feature", "create", "checkout"]);
}

/// A detached `checkout` session under `provider`; its view dir.
fn start_session(root: &Utf8Path, provider: &str) -> Utf8PathBuf {
    let session = run_ok(
        root,
        &[
            "session",
            "start",
            "checkout",
            "--detached",
            "--provider",
            provider,
        ],
    );
    Utf8PathBuf::from(session["view_dir"].as_str().unwrap())
}

/// `checkout`'s worktree of `app`, which exists once `app` is promoted.
fn feature_worktree(root: &Utf8Path) -> Utf8PathBuf {
    root.join(".ivar/repos/app/checkout")
}

/// `ivar guard --provider <provider> <extra…>` fed `payload`, from the hall root.
fn guard(root: &Utf8Path, provider: &str, extra: &[&str], payload: &Value) -> std::process::Output {
    isolated_ivar(&home(root))
        .current_dir(root)
        .args(["guard", "--provider", provider])
        .args(extra)
        .write_stdin(payload.to_string())
        .output()
        .unwrap()
}

/// A Claude `PreToolUse` payload.
fn claude_payload(view: &Utf8Path, session: &str, call: &str, tool: &str, input: &Value) -> Value {
    json!({
        "session_id": session,
        "tool_use_id": call,
        "hook_event_name": "PreToolUse",
        "tool_name": tool,
        "tool_input": input,
        "cwd": view,
    })
}

/// The Claude hook body; always exit 0.
fn claude_body(output: &std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn context(body: &Value) -> Option<&str> {
    body["hookSpecificOutput"]["additionalContext"].as_str()
}

fn label(view: &Utf8Path, file: &str, dir: &str) -> String {
    format!("Repository instructions from {view}/{file} (they apply to work under {view}/{dir}):\n")
}

#[test]
fn claude_sessions_receive_main_then_feature_instructions_once_per_agent() {
    let (_guard, root) = hall_root();
    hall_with_instructed_repo(&root);
    let view = start_session(&root, "claude-code");
    let lib = view.join("app/src/lib.rs");

    // The root file points at the repo's file through the view symlink.
    let root_file = std::fs::read_to_string(view.join("CLAUDE.md")).unwrap();
    assert!(
        root_file.contains(&format!("- `app`: {view}/app/CLAUDE.md\n")),
        "{root_file}"
    );

    // First touch: main's CLAUDE.md, whole, with an allow.
    let read = claude_payload(
        &view,
        "agent-main",
        "toolu_01",
        "Read",
        &json!({ "file_path": lib }),
    );
    let body = claude_body(&guard(&root, "claude-code", &[], &read));
    assert_eq!(body["hookSpecificOutput"]["permissionDecision"], "allow");
    assert_eq!(
        context(&body),
        Some(format!("{}{MAIN_CLAUDE}", label(&view, "app/CLAUDE.md", "app")).as_str()),
        "provider-native file only, whole, labelled"
    );
    assert!(
        view.join(".claude/ivar/instructions/agent-main.json")
            .is_file(),
        "delivery state lives under the ivar-owned config dir"
    );

    // Same agent again: nothing new.
    let again = claude_payload(
        &view,
        "agent-main",
        "toolu_02",
        "Read",
        &json!({ "file_path": lib }),
    );
    let body = claude_body(&guard(&root, "claude-code", &[], &again));
    assert_eq!(body["hookSpecificOutput"]["permissionDecision"], "allow");
    assert_eq!(
        context(&body),
        None,
        "already delivered to this agent: {body}"
    );

    // A subagent of the same session gets its own copy.
    let mut sub = claude_payload(
        &view,
        "agent-main",
        "toolu_03",
        "Read",
        &json!({ "file_path": lib }),
    );
    sub["agent_id"] = json!("sub-1");
    let body = claude_body(&guard(&root, "claude-code", &[], &sub));
    assert!(context(&body).unwrap().contains(MAIN_CLAUDE), "{body}");

    // Promote mid-session and change the feature's file.
    run_ok(&root, &["feature", "promote", "checkout", "app"]);
    let target = std::fs::read_link(view.join("app")).unwrap();
    assert!(
        target.ends_with("app/checkout"),
        "view not repointed: {target:?}"
    );
    std::fs::write(feature_worktree(&root).join("CLAUDE.md"), FEATURE_CLAUDE).unwrap();

    let after = claude_payload(
        &view,
        "agent-main",
        "toolu_04",
        "Read",
        &json!({ "file_path": lib }),
    );
    let body = claude_body(&guard(&root, "claude-code", &[], &after));
    assert_eq!(
        context(&body),
        Some(
            format!(
                "UPDATED {}{FEATURE_CLAUDE}",
                label(&view, "app/CLAUDE.md", "app")
            )
            .as_str()
        ),
        "the feature worktree's file, marked UPDATED, with no restart"
    );

    // A shell command naming the repo counts as a touch (fresh agent).
    let bash = claude_payload(
        &view,
        "agent-bash",
        "toolu_05",
        "Bash",
        &json!({ "command": "cat app/src/lib.rs | head -n 1" }),
    );
    let body = claude_body(&guard(&root, "claude-code", &[], &bash));
    assert!(context(&body).unwrap().contains(FEATURE_CLAUDE), "{body}");
}

#[test]
fn a_denied_write_carries_no_instructions_and_records_none() {
    let (_guard, root) = hall_root();
    hall_with_instructed_repo(&root);
    let view = start_session(&root, "claude-code");

    // `app` is not promoted: its view link is the read-only main worktree.
    let write = claude_payload(
        &view,
        "fresh-agent",
        "toolu_10",
        "Write",
        &json!({ "file_path": view.join("app/src/new.rs"), "content": "x" }),
    );
    let body = claude_body(&guard(&root, "claude-code", &[], &write));
    assert_eq!(body["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(
        body["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .contains("is outside the writable set"),
        "{body}"
    );
    assert_eq!(
        context(&body),
        None,
        "a deny never carries instructions: {body}"
    );

    // The deny recorded nothing: the same agent's first allowed touch still gets them.
    let read = claude_payload(
        &view,
        "fresh-agent",
        "toolu_11",
        "Read",
        &json!({ "file_path": view.join("app/src/lib.rs") }),
    );
    let body = claude_body(&guard(&root, "claude-code", &[], &read));
    assert!(context(&body).unwrap().contains(MAIN_CLAUDE), "{body}");
}

#[test]
fn claude_slice_entries_carry_the_rest_of_a_large_context() {
    let (_guard, root) = hall_root();
    hall_with_instructed_repo(&root);
    let view = start_session(&root, "claude-code");
    run_ok(&root, &["feature", "promote", "checkout", "app"]);
    let big = format!(
        "# src rules\n{}SLICE-END-MARKER\n",
        "- keep every module in src small and documented.\n".repeat(250)
    );
    assert!(big.chars().count() > 9_500 + 2_000);
    std::fs::write(feature_worktree(&root).join("src/CLAUDE.md"), &big).unwrap();

    let read = claude_payload(
        &view,
        "slicer",
        "toolu_slice",
        "Read",
        &json!({ "file_path": view.join("app/src/lib.rs") }),
    );
    let main_entry = claude_body(&guard(&root, "claude-code", &[], &read));
    assert_eq!(
        main_entry["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );
    let slice0 = context(&main_entry).unwrap().to_owned();
    assert_eq!(
        slice0.chars().count(),
        9_500,
        "the guard entry carries a full first slice"
    );
    assert!(!slice0.contains("SLICE-END-MARKER"));

    let slice_entry = claude_body(&guard(&root, "claude-code", &["--slice", "1"], &read));
    assert!(
        slice_entry["hookSpecificOutput"]
            .get("permissionDecision")
            .is_none(),
        "a slice entry never decides: {slice_entry}"
    );
    let slice1 = context(&slice_entry).unwrap();
    let whole = format!("{slice0}{slice1}");
    assert!(
        whole.contains(&format!(
            "{}{MAIN_CLAUDE}",
            label(&view, "app/CLAUDE.md", "app")
        )),
        "{whole}"
    );
    assert!(
        whole.contains(&format!(
            "{}{big}",
            label(&view, "app/src/CLAUDE.md", "app/src")
        )),
        "the 12.5k-char file arrives whole across two slices"
    );

    let past_the_end = claude_body(&guard(&root, "claude-code", &["--slice", "2"], &read));
    assert_eq!(past_the_end, json!({}));
}

#[test]
fn omp_guard_prints_the_instructions_and_allows() {
    let (_guard, root) = hall_root();
    hall_with_instructed_repo(&root);
    let view = start_session(&root, "omp");
    let root_file = std::fs::read_to_string(view.join("AGENTS.md")).unwrap();
    assert!(
        root_file.contains(&format!("- `app`: {view}/app/AGENTS.md\n")),
        "{root_file}"
    );

    let read = json!({
        "tool": "read",
        "args": { "path": view.join("app/src/lib.rs") },
        "cwd": view,
        "agent": "main",
    });
    let output = guard(&root, "omp", &[], &read);
    assert!(output.status.success(), "a read is allowed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout.trim_end(),
        format!("{}{MAIN_AGENTS}", label(&view, "app/AGENTS.md", "app")).trim_end(),
        "omp gets its native AGENTS.md on stdout"
    );
    assert!(view.join(".omp/ivar/instructions/main.json").is_file());

    let again = guard(&root, "omp", &[], &read);
    assert!(again.status.success());
    assert_eq!(
        String::from_utf8_lossy(&again.stdout).trim(),
        "",
        "delivered once"
    );

    let denied = guard(
        &root,
        "omp",
        &[],
        &json!({
            "tool": "write",
            "args": { "path": view.join("app/src/new.rs"), "content": "x" },
            "cwd": view,
            "agent": "other",
        }),
    );
    assert!(!denied.status.success(), "unpromoted repo is read-only");
    assert!(
        !String::from_utf8_lossy(&denied.stdout).contains("Repository instructions"),
        "a deny carries no instructions"
    );
}
