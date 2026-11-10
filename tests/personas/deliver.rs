//! Deliver refuses exactly what its preview lists, and says what `--land` pushes.

use camino::Utf8Path;
use predicates::prelude::*;

use super::support::{commit_slice, home, isolated_ivar, one_repo_hall, run_ok};
use crate::common::hall_root;

fn feature_with_a_slice(root: &Utf8Path) {
    one_repo_hall(root);
    run_ok(root, &["feature", "create", "ship"]);
    run_ok(root, &["feature", "promote", "ship", "app"]);
    commit_slice(root, "app", "ship", "slice.md");
}

fn approve_plan(root: &Utf8Path) {
    run_ok(root, &["plan", "create", "ship"]);
    for gate in ["requirements", "analysis", "plan"] {
        run_ok(root, &["plan", "approve", "ship", gate]);
    }
    crate::common::record_memory_review(root, "ship");
}

#[test]
fn deliver_refuses_exactly_what_its_preview_lists_as_blocked() {
    let (_guard, root) = hall_root();
    feature_with_a_slice(&root);
    approve_plan(&root);
    std::fs::write(root.join(".ivar/repos/app/ship/wip.md"), "wip\n").unwrap();

    let preview = run_ok(&root, &["feature", "deliver", "ship", "--preview"]);

    let blockers = preview["blockers"].as_array().expect("blockers listed");
    assert!(
        blockers
            .iter()
            .any(|b| b.as_str().unwrap().contains("uncommitted")),
        "the dirty worktree is a blocker: {blockers:?}"
    );
    assert!(
        preview["preview"]["repos"][0]["pending"][0]
            .as_str()
            .unwrap()
            .contains("not pushed"),
        "unpushed work is what deliver delivers, not a blocker: {preview}"
    );
    let fingerprint = preview["preview"]["fingerprint"].as_str().unwrap();
    isolated_ivar(&home(&root))
        .current_dir(&root)
        .args(["feature", "deliver", "ship", "--fingerprint", fingerprint])
        .assert()
        .failure()
        .stderr(predicate::str::contains("uncommitted"));
    let origin = root.parent().unwrap().join("origins/app");
    let branches = std::process::Command::new("git")
        .args(["branch", "--list", "ship"])
        .current_dir(&origin)
        .output()
        .unwrap();
    assert!(
        branches.stdout.is_empty(),
        "a refused delivery pushed nothing"
    );
}

#[test]
fn deliver_lists_a_missing_plan_and_says_to_create_one() {
    let (_guard, root) = hall_root();
    feature_with_a_slice(&root);

    let preview = run_ok(&root, &["feature", "deliver", "ship", "--preview"]);
    let blockers = preview["blockers"].as_array().expect("blockers listed");
    assert!(
        blockers
            .iter()
            .any(|b| b.as_str().unwrap().contains("plan")),
        "the unapproved plan gate is listed: {blockers:?}"
    );

    let fingerprint = preview["preview"]["fingerprint"].as_str().unwrap();
    isolated_ivar(&home(&root))
        .current_dir(&root)
        .args(["feature", "deliver", "ship", "--fingerprint", fingerprint])
        .assert()
        .failure()
        .stderr(predicate::str::contains("ivar plan create ship"));
}

#[test]
fn deliver_land_says_it_pushes_the_default_branch() {
    let (_guard, root) = hall_root();
    feature_with_a_slice(&root);

    isolated_ivar(&home(&root))
        .current_dir(&root)
        .args(["feature", "deliver", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("then push each default"));
    isolated_ivar(&home(&root))
        .current_dir(&root)
        .args(["feature", "deliver", "ship", "--land", "--preview"])
        .assert()
        .success()
        .stdout(predicate::str::contains("then push main to"));
}

/// Runs `calls` (name, arguments) through `ivar graph mcp` started in `cwd`
/// with no session in its environment, as a T3 thread or OMP does when its
/// server started before the session existed. Returns each answer's text.
fn mcp_unbound(cwd: &Utf8Path, calls: &[(&str, serde_json::Value)]) -> Vec<String> {
    let stdin: String = calls
        .iter()
        .enumerate()
        .map(|(i, (name, arguments))| {
            format!(
                "{}\n",
                serde_json::json!({"jsonrpc": "2.0", "id": i + 1, "method": "tools/call",
                    "params": {"name": name, "arguments": arguments}})
            )
        })
        .collect();
    let output = isolated_ivar(&home(cwd))
        .current_dir(cwd)
        .args(["graph", "mcp"])
        .write_stdin(stdin)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let responses: Vec<serde_json::Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (1..=calls.len())
        .map(|id| {
            let response = responses.iter().find(|r| r["id"] == id).unwrap();
            let text = response["result"]["content"][0]["text"].as_str().unwrap();
            assert!(response["result"]["isError"] != true, "{response}");
            text.to_owned()
        })
        .collect()
}

#[test]
fn a_converted_discovery_delivers_after_a_review_confirmed_with_an_explicit_feature() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    let session = run_ok(&root, &["session", "start", "--detached"]);
    let session_id = session["session_id"].as_str().unwrap();
    isolated_ivar(&home(&root))
        .current_dir(session["view_dir"].as_str().unwrap())
        .env("IVAR_SESSION_ID", session_id)
        .args(["discovery", "create", "ship"])
        .assert()
        .success();
    run_ok(&root, &["session", "convert", session_id]);
    run_ok(&root, &["feature", "promote", "ship", "app"]);
    commit_slice(&root, "app", "ship", "slice.md");
    run_ok(&root, &["plan", "create", "ship"]);
    for gate in ["requirements", "analysis", "plan"] {
        run_ok(&root, &["plan", "approve", "ship", gate]);
    }

    let preview = run_ok(&root, &["feature", "deliver", "ship", "--preview"]);
    assert!(
        preview["blockers"].to_string().contains("memory review"),
        "an unreviewed feature is blocked: {preview}"
    );

    let answers = mcp_unbound(
        &root,
        &[
            (
                "memory_query",
                serde_json::json!({"query": "locks", "feature": "ship"}),
            ),
            (
                "memory_write_page",
                serde_json::json!({"kind": "decision", "title": "Locks", "body": "Why.\n", "feature": "ship"}),
            ),
            ("memory_review", serde_json::json!({"feature": "ship"})),
            (
                "memory_review",
                serde_json::json!({"feature": "ship", "confirm": true}),
            ),
        ],
    );
    let report: serde_json::Value = serde_json::from_str(&answers[2]).unwrap();
    assert_eq!(
        report["written"][0]["path"],
        "docs/memory/decision/locks.md"
    );
    assert!(
        !report["gaps"]
            .to_string()
            .contains("analysis_without_query"),
        "{report}"
    );
    let confirmed: serde_json::Value = serde_json::from_str(&answers[3]).unwrap();
    assert_eq!(confirmed["last"]["source_event"], "memory.reviewed");

    let preview = run_ok(&root, &["feature", "deliver", "ship", "--preview"]);
    let fingerprint = preview["preview"]["fingerprint"].as_str().unwrap();
    isolated_ivar(&home(&root))
        .current_dir(&root)
        .args(["feature", "deliver", "ship", "--fingerprint", fingerprint])
        .assert()
        .success();
}
