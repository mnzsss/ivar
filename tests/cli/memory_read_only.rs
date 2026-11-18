//! `ivar memory status --read-only` and `ivar memory review --read-only` end
//! to end: they read a hall whose `.ivar/` cannot be written and leave every
//! byte of it as it was.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;

use camino::Utf8Path;
use ivar::store::graph::db::GraphDb;
use serde_json::{Value, json};

use crate::common::{hall_root, ivar};

fn run(root: &Utf8Path, args: &[&str]) {
    ivar().current_dir(root).args(args).assert().success();
}

/// A hall with feature `checkout`, a memory database, and one hook event still
/// waiting in the capture spool.
fn hall_with_a_pending_spool() -> (tempfile::TempDir, camino::Utf8PathBuf) {
    let (guard, root) = hall_root();
    run(&root, &["init"]);
    run(&root, &["feature", "create", "checkout"]);
    run(&root, &["memory", "status"]);
    ivar()
        .current_dir(&root)
        .env("IVAR_HALL", &root)
        .env("IVAR_SESSION_ID", "s1")
        .env("IVAR_FEATURE", "checkout")
        .args(["memory", "capture", "--provider", "claude-code"])
        .write_stdin(
            json!({"hook_event_name": "UserPromptSubmit", "session_id": "native-1", "prompt": "still in the spool"})
                .to_string(),
        )
        .assert()
        .success();
    (guard, root)
}

/// Every file under `.ivar/` with its bytes.
fn snapshot(root: &Utf8Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(base).unwrap().display().to_string();
            if path.is_dir() {
                out.insert(format!("{rel}/"), Vec::new());
                walk(&path, base, out);
            } else {
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let base = root.join(".ivar");
    let mut out = BTreeMap::new();
    walk(base.as_std_path(), base.as_std_path(), &mut out);
    out
}

/// Fails naming each entry that was added, removed or changed.
fn assert_unchanged(root: &Utf8Path, before: &BTreeMap<String, Vec<u8>>) {
    let after = snapshot(root);
    let mut changes = Vec::new();
    for (path, bytes) in &after {
        match before.get(path) {
            None => changes.push(format!("added {path}")),
            Some(old) if old != bytes => changes.push(format!("changed {path}")),
            Some(_) => {}
        }
    }
    changes.extend(
        before
            .keys()
            .filter(|path| !after.contains_key(*path))
            .map(|path| format!("removed {path}")),
    );
    assert!(changes.is_empty(), ".ivar/ was written: {changes:?}");
}

fn read_only_json(root: &Utf8Path, args: &[&str]) -> Value {
    let ivar_dir = root.join(".ivar");
    let restore = std::fs::metadata(&ivar_dir).unwrap().permissions();
    std::fs::set_permissions(&ivar_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let out = ivar()
        .current_dir(root)
        .env_remove("IVAR_SESSION_ID")
        .args(["--json", "memory"])
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    std::fs::set_permissions(&ivar_dir, restore).unwrap();
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn status_read_only_reports_the_pending_spool_and_changes_nothing() {
    let (_guard, root) = hall_with_a_pending_spool();
    let before = snapshot(&root);

    let status = read_only_json(&root, &["status", "--read-only"]);

    assert_unchanged(&root, &before);
    assert_eq!(status["read_only"], true, "{status}");
    assert_eq!(status["exists"], true, "{status}");
    assert_eq!(status["spool"]["drained"], false, "{status}");
    assert_eq!(status["spool"]["files"], 1, "{status}");
    assert!(status["spool"]["bytes"].as_u64().unwrap() > 0, "{status}");
    assert!(status["pages"]["invalid"].is_null(), "{status}");
    assert!(status["schema_version"].is_i64(), "{status}");
}

#[test]
fn status_without_the_flag_drains_and_says_so() {
    let (_guard, root) = hall_with_a_pending_spool();

    let out = ivar()
        .current_dir(&root)
        .env_remove("IVAR_SESSION_ID")
        .args(["--json", "memory", "status"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let status: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(status["read_only"], false, "{status}");
    assert_eq!(status["spool"]["drained"], true, "{status}");
    assert_eq!(status["spool"]["files"], 0, "{status}");
}

#[test]
fn status_read_only_without_a_database_creates_none() {
    let (_guard, root) = hall_root();
    run(&root, &["init"]);
    let before = snapshot(&root);

    let status = read_only_json(&root, &["status", "--read-only"]);

    assert_unchanged(&root, &before);
    assert_eq!(status["exists"], false, "{status}");
    assert!(!root.join(".ivar/memory.db").exists());
}

#[test]
fn review_read_only_leaves_the_spool_intact_and_warns() {
    let (_guard, root) = hall_with_a_pending_spool();
    drop(GraphDb::open(root.join(".ivar/graph.db").as_std_path()).unwrap());
    let before = snapshot(&root);

    let review = read_only_json(&root, &["review", "checkout", "--read-only"]);

    assert_unchanged(&root, &before);
    assert_eq!(review["feature"], "checkout", "{review}");
    let warnings = review["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("not drained")),
        "{review}"
    );
    assert!(review["written"].as_array().unwrap().is_empty(), "{review}");
}

#[test]
fn review_read_only_refuses_to_record() {
    let (_guard, root) = hall_with_a_pending_spool();

    for flag in [
        &["--confirm"][..],
        &["--stale", "p.md=updated"],
        &["--not-required", "x"],
    ] {
        ivar()
            .current_dir(&root)
            .args(["memory", "review", "checkout", "--read-only"])
            .args(flag)
            .assert()
            .failure();
    }
}

#[test]
fn an_outdated_schema_exits_2_naming_the_command_and_is_left_alone() {
    let (_guard, root) = hall_with_a_pending_spool();
    let db = root.join(".ivar/memory.db");
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch("UPDATE memory_schema SET version = 5;")
        .unwrap();
    let before = snapshot(&root);

    for args in [
        &["memory", "status", "--read-only"][..],
        &["memory", "review", "checkout", "--read-only"],
    ] {
        ivar()
            .current_dir(&root)
            .args(args)
            .assert()
            .code(2)
            .stderr(predicates::str::contains("ivar memory status"));
    }

    assert_unchanged(&root, &before);
}

#[test]
fn a_database_of_another_program_is_not_read_as_memory() {
    let (_guard, root) = hall_with_a_pending_spool();
    let db = root.join(".ivar/memory.db");
    std::fs::remove_file(&db).unwrap();
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch("CREATE TABLE other (x); PRAGMA application_id = 7;")
        .unwrap();
    let before = snapshot(&root);

    ivar()
        .current_dir(&root)
        .args(["memory", "status", "--read-only"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("ivar memory status"));

    assert_unchanged(&root, &before);
}
