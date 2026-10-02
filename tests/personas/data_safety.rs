//! Prune, delete and session stop never destroy work silently.

use camino::Utf8Path;

use super::support::{commit_slice, home, isolated_ivar, one_repo_hall, run_ok};
use crate::common::{git, hall_root};

fn ivar_in(root: &Utf8Path, cwd: &Utf8Path) -> assert_cmd::Command {
    let mut cmd = isolated_ivar(&home(root));
    cmd.current_dir(cwd);
    cmd
}

fn discovery_session_ids(root: &Utf8Path) -> Vec<String> {
    std::fs::read_dir(root.join(".ivar/sessions"))
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn data_safety_prune_keeps_a_fresh_feature_with_uncommitted_work() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["feature", "create", "fresh"]);
    run_ok(&root, &["feature", "promote", "fresh", "app"]);
    let draft = root.join(".ivar/repos/app/fresh/draft.md");
    std::fs::write(&draft, "unsaved\n").unwrap();

    let _ = ivar_in(&root, &root).args(["feature", "prune"]).output();

    assert!(draft.exists(), "prune destroyed uncommitted work");
}

#[test]
fn data_safety_prune_keeps_a_clean_feature_with_no_commits_even_after_its_base_moves() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["feature", "create", "parent"]);
    run_ok(&root, &["feature", "promote", "parent", "app"]);
    run_ok(&root, &["feature", "create", "idle", "--parent", "parent"]);
    run_ok(&root, &["feature", "promote", "idle", "app"]);
    commit_slice(&root, "app", "parent", "moved.txt");

    let pruned = run_ok(&root, &["feature", "prune"]);

    assert_eq!(pruned["pruned"], serde_json::json!([]), "{pruned}");
    assert!(root.join(".ivar/features/idle/feature.json").is_file());
}

#[test]
fn data_safety_prune_removes_a_feature_whose_commits_landed() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["feature", "create", "landed"]);
    run_ok(&root, &["feature", "promote", "landed", "app"]);
    commit_slice(&root, "app", "landed", "landed.txt");
    git(
        &root.join(".ivar/repos/app/main"),
        &["merge", "--no-ff", "-m", "land", "landed"],
    );

    let pruned = run_ok(&root, &["feature", "prune"]);

    assert_eq!(pruned["pruned"], serde_json::json!(["landed"]), "{pruned}");
}

#[test]
fn data_safety_delete_refuses_a_dirty_worktree_until_forced() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["feature", "create", "wip"]);
    run_ok(&root, &["feature", "promote", "wip", "app"]);
    let draft = root.join(".ivar/repos/app/wip/draft.md");
    std::fs::write(&draft, "unsaved\n").unwrap();

    ivar_in(&root, &root)
        .args(["feature", "delete", "wip"])
        .assert()
        .failure();
    assert!(draft.exists(), "a refused delete removed the draft");

    ivar_in(&root, &root)
        .args(["feature", "delete", "wip", "--force"])
        .assert()
        .success();
    assert!(!root.join(".ivar/features/wip").exists());
}

#[test]
fn data_safety_delete_refuses_a_feature_with_a_live_session_until_forced() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["feature", "create", "busy"]);
    run_ok(&root, &["session", "start", "busy", "--detached"]);

    ivar_in(&root, &root)
        .args(["feature", "delete", "busy"])
        .assert()
        .failure();
    assert!(root.join(".ivar/features/busy/feature.json").is_file());

    ivar_in(&root, &root)
        .args(["feature", "delete", "busy", "--force"])
        .assert()
        .success();
}

#[test]
fn data_safety_stop_without_an_id_outside_a_session_stops_nothing() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["session", "start", "--detached"]);
    run_ok(&root, &["session", "start", "--detached"]);

    ivar_in(&root, &root)
        .args(["session", "stop"])
        .assert()
        .failure();
    ivar_in(&root, &root)
        .args(["session", "stop", "no-such-session"])
        .assert()
        .failure();

    assert_eq!(discovery_session_ids(&root).len(), 2);
}

#[test]
fn data_safety_stop_without_an_id_stops_the_current_session_and_all_stops_every_one() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["session", "start", "--detached"]);
    run_ok(&root, &["session", "start", "--detached"]);
    let current = discovery_session_ids(&root).remove(0);

    ivar_in(&root, &root)
        .env("IVAR_SESSION_ID", &current)
        .args(["session", "stop"])
        .assert()
        .success();
    assert!(!discovery_session_ids(&root).contains(&current));
    assert_eq!(discovery_session_ids(&root).len(), 1);

    let stopped = run_ok(&root, &["session", "stop", "--all"]);
    assert_eq!(stopped["stopped"], 1, "{stopped}");
    assert!(discovery_session_ids(&root).is_empty());
}

#[test]
fn data_safety_stop_by_id_ends_a_feature_session() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["feature", "create", "work"]);
    let session = run_ok(&root, &["session", "start", "work", "--detached"]);
    let id = session["session_id"].as_str().unwrap();
    let view_dir = session["view_dir"].as_str().unwrap();

    let stopped = run_ok(&root, &["session", "stop", id]);

    assert_eq!(stopped["stopped"], 1, "{stopped}");
    assert!(!Utf8Path::new(view_dir).exists(), "view dir survived stop");
}

#[test]
fn data_safety_stopping_a_discovery_keeps_its_doc() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["session", "start", "--detached"]);
    let id = discovery_session_ids(&root).remove(0);
    let view_dir = root.join(".ivar/sessions").join(&id);
    ivar_in(&root, &view_dir)
        .args(["discovery", "create", "notes"])
        .assert()
        .success();
    assert!(view_dir.join("discovery.md").is_file());

    run_ok(&root, &["session", "stop", &id]);

    let list = run_ok(&root, &["discovery", "list"]);
    assert!(
        list.to_string().contains("\"notes\""),
        "discovery doc lost: {list}"
    );
}
