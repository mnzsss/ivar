//! `ivar automation scan` end to end: the human and the `--json` surface read
//! the same outcome, and a scan only reads `memory.db`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::os::unix::fs::PermissionsExt;

use camino::Utf8Path;
use ivar::domain::session::rfc3339_from_epoch_secs;
use ivar::store::memory::db::MemoryDb;
use serde_json::{Value, json};

use crate::common::{hall_root, ivar};

const DAY: u64 = 86_400;

fn day(days_ago: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    rfc3339_from_epoch_secs((now - days_ago * DAY) as f64)[..10].to_owned()
}

fn hall() -> (tempfile::TempDir, camino::Utf8PathBuf) {
    let (guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    (guard, root)
}

/// One fingerprint per `(session, feature, days ago, text)`.
fn plant(root: &Utf8Path, rows: &[(&str, &str, u64, &str)]) {
    let db = MemoryDb::open(root.join(".ivar/memory.db").as_std_path()).unwrap();
    for (n, (session, feature, days_ago, text)) in rows.iter().enumerate() {
        let day = day(*days_ago);
        db.conn()
            .execute(
                "INSERT INTO observations
                   (event_id, source_event, occurred_at, session_id, native_session_id, provider,
                    ivar_version, feature, parent_feature, repo, origin, importance, attempt_id, body)
                 VALUES (?1, 'session.prompted', ?2, ?3, '', '', '', ?4, '', '', 'user', 1, '', ?5)",
                rusqlite::params![format!("e{n}"), format!("{day}T10:00:00Z"), session, feature, text],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO request_fingerprints
                   (observation_id, session_id, feature, day, normalized, created_at)
                 VALUES (last_insert_rowid(), ?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![session, feature, day, text, format!("{day}T10:00:00Z")],
            )
            .unwrap();
    }
}

fn scan(root: &Utf8Path, args: &[&str]) -> assert_cmd::assert::Assert {
    ivar()
        .current_dir(root)
        .env_remove("IVAR_SESSION_ID")
        .args(["automation", "scan"])
        .args(args)
        .assert()
}

fn scan_json(root: &Utf8Path, args: &[&str]) -> Value {
    let out = ivar()
        .current_dir(root)
        .env_remove("IVAR_SESSION_ID")
        .args(["--json", "automation", "scan"])
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

fn recurring(root: &Utf8Path) {
    plant(
        root,
        &[
            ("s1", "", 6, "update changelog entry"),
            ("s2", "", 6, "update changelog entry"),
            ("s3", "", 2, "update changelog entry typo"),
            ("s4", "", 1, "write something else"),
        ],
    );
}

#[test]
fn the_json_output_is_the_outcome() {
    let (_guard, root) = hall();
    recurring(&root);

    let out = scan_json(&root, &["--no-drain"]);

    assert_eq!(out["ok"], true);
    assert_eq!(out["window_days"], 90);
    assert_eq!(out["fingerprints"], 4);
    assert_eq!(out["feature"], Value::Null);
    assert_eq!(out["suppressed"], json!([]));
    let candidates = out["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert_eq!(candidate["sessions"], json!(["s1", "s2", "s3"]));
    assert_eq!(candidate["days"].as_array().unwrap().len(), 2);
    assert_eq!(candidate["observation_ids"].as_array().unwrap().len(), 3);
    assert_eq!(
        candidate["samples"],
        json!([
            {"text": "update changelog entry typo", "untrusted": true},
            {"text": "update changelog entry", "untrusted": true},
        ])
    );
    assert!(candidate["group_ref"].is_i64());
}

#[test]
fn the_human_output_renders_the_same_candidate() {
    let (_guard, root) = hall();
    recurring(&root);

    let assert = scan(&root, &["--no-drain"]).success();

    let text = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(text.contains("1 recurring request candidate(s) among 4 fingerprint(s)"));
    assert!(text.contains("3 sessions on 2 days"));
    assert!(text.contains("memory text, evidence only"));
    assert!(text.contains("\"update changelog entry typo\""));
    assert!(text.contains("sessions: s1, s2, s3"));
    assert!(!text.contains("something else"));
}

#[test]
fn a_hall_with_nothing_captured_reports_no_candidates() {
    let (_guard, root) = hall();

    let out = scan_json(&root, &[]);

    assert_eq!(out["candidates"], json!([]));
    assert_eq!(out["fingerprints"], 0);
}

#[test]
fn the_feature_cut_keeps_the_groups_of_the_features_sessions() {
    let (_guard, root) = hall();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "checkout"])
        .assert()
        .success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "checkout-ui", "--parent", "checkout"])
        .assert()
        .success();
    plant(
        &root,
        &[
            ("c1", "checkout", 6, "bump the version"),
            ("c2", "checkout-ui", 5, "bump the version"),
            ("x1", "", 1, "bump the version"),
            ("o1", "", 6, "refresh the fixtures"),
            ("o2", "", 5, "refresh the fixtures"),
            ("o3", "", 1, "refresh the fixtures"),
        ],
    );

    let all = scan_json(&root, &["--no-drain"]);
    let cut = scan_json(&root, &["--no-drain", "--feature", "checkout"]);

    assert_eq!(all["candidates"].as_array().unwrap().len(), 2);
    assert_eq!(cut["feature"], "checkout");
    let candidates = cut["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0]["samples"][0]["text"], "bump the version");
    assert_eq!(
        candidates[0]["features"],
        json!(["checkout", "checkout-ui"])
    );
}

#[test]
fn an_unknown_feature_exits_2() {
    let (_guard, root) = hall();

    scan(&root, &["--no-drain", "--feature", "nope"])
        .code(2)
        .stderr(predicates::str::contains("nope"));
}

#[test]
fn the_default_scan_drains_the_spool_and_no_drain_does_not() {
    let (_guard, root) = hall();
    plant(&root, &[("s0", "", 1, "already stored")]);
    ivar()
        .current_dir(&root)
        .env("IVAR_HALL", &root)
        .env("IVAR_SESSION_ID", "s9")
        .args(["memory", "capture", "--provider", "claude-code"])
        .write_stdin(
            json!({"hook_event_name": "UserPromptSubmit", "session_id": "native-9", "prompt": "from the spool"})
                .to_string(),
        )
        .assert()
        .success();

    assert_eq!(scan_json(&root, &["--no-drain"])["fingerprints"], 1);
    assert_eq!(scan_json(&root, &[])["fingerprints"], 2);
}

#[test]
fn no_drain_reads_with_a_read_only_ivar_dir() {
    let (_guard, root) = hall();
    recurring(&root);
    let ivar_dir = root.join(".ivar");
    let restore = std::fs::metadata(&ivar_dir).unwrap().permissions();
    std::fs::set_permissions(&ivar_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    let out = scan_json(&root, &["--no-drain"]);

    std::fs::set_permissions(&ivar_dir, restore).unwrap();
    assert_eq!(out["candidates"].as_array().unwrap().len(), 1);
}

#[test]
fn an_outdated_schema_exits_2_naming_the_command_and_is_left_alone() {
    let (_guard, root) = hall();
    recurring(&root);
    let db = root.join(".ivar/memory.db");
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch("UPDATE memory_schema SET version = 5;")
        .unwrap();
    let before = std::fs::read(&db).unwrap();

    scan(&root, &["--no-drain"])
        .code(2)
        .stderr(predicates::str::contains("ivar memory status"));

    assert_eq!(std::fs::read(&db).unwrap(), before);
}
