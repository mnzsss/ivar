//! `ivar automation decide | renew | decisions | undo` end to end: the human and
//! the `--json` surface read the same outcome, and the answers change what
//! `scan` reports.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8Path;
use ivar::domain::session::rfc3339_from_epoch_secs;
use ivar::store::memory::db::MemoryDb;
use serde_json::{Value, json};

use crate::common::{hall_root, ivar};

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn day(days_from_now: i64) -> String {
    let secs = now().checked_add_signed(days_from_now * 86_400).unwrap();
    rfc3339_from_epoch_secs(secs as f64)[..10].to_owned()
}

fn hall() -> (tempfile::TempDir, camino::Utf8PathBuf) {
    let (guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    (guard, root)
}

fn plant(root: &Utf8Path, rows: &[(&str, i64, &str)]) {
    let db = MemoryDb::open(root.join(".ivar/memory.db").as_std_path()).unwrap();
    for (session, days, text) in rows {
        let when = format!("{}T10:00:00Z", day(*days));
        db.conn()
            .execute(
                "INSERT INTO request_fingerprints
                   (observation_id, session_id, feature, day, normalized, created_at)
                 VALUES (0, ?1, '', ?2, ?3, ?4)",
                rusqlite::params![session, day(*days), text, when],
            )
            .unwrap();
    }
}

/// Three groups of three sessions on two days each.
fn recurring(root: &Utf8Path) {
    plant(
        root,
        &[
            ("s1", -6, "update changelog entry"),
            ("s2", -6, "update changelog entry"),
            ("s3", -2, "update changelog entry"),
            ("s1", -6, "rotate signing keys"),
            ("s2", -6, "rotate signing keys"),
            ("s3", -2, "rotate signing keys"),
            ("s1", -6, "refresh test fixtures"),
            ("s2", -6, "refresh test fixtures"),
            ("s3", -2, "refresh test fixtures"),
        ],
    );
}

fn json_of(root: &Utf8Path, args: &[&str]) -> Value {
    json_with_code(root, args, 0)
}

/// A run that succeeded with a warning exits 1 and still prints its outcome.
fn json_with_code(root: &Utf8Path, args: &[&str], code: i32) -> Value {
    let out = ivar()
        .current_dir(root)
        .env_remove("IVAR_SESSION_ID")
        .arg("--json")
        .arg("automation")
        .args(args)
        .assert()
        .code(code)
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

fn human(root: &Utf8Path, args: &[&str]) -> String {
    let out = ivar()
        .current_dir(root)
        .env_remove("IVAR_SESSION_ID")
        .arg("automation")
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).unwrap()
}

fn failing(root: &Utf8Path, args: &[&str]) -> assert_cmd::assert::Assert {
    ivar()
        .current_dir(root)
        .env_remove("IVAR_SESSION_ID")
        .arg("automation")
        .args(args)
        .assert()
}

fn scan(root: &Utf8Path) -> Value {
    json_of(root, &["scan", "--no-drain"])
}

/// The `group_ref` of the candidate whose first sample is `text`.
fn group_ref(root: &Utf8Path, text: &str) -> String {
    scan(root)["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["samples"][0]["text"] == text)
        .unwrap_or_else(|| panic!("no candidate `{text}`"))["group_ref"]
        .to_string()
}

fn texts(candidates: &Value) -> Vec<String> {
    let mut found: Vec<String> = candidates
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["samples"][0]["text"].as_str().unwrap().to_owned())
        .collect();
    found.sort();
    found
}

#[test]
fn decide_declined_suppresses_the_group_in_the_next_scan() {
    let (_guard, root) = hall();
    recurring(&root);
    let reference = group_ref(&root, "update changelog entry");

    let out = json_of(
        &root,
        &["decide", &reference, "--declined", "--intent", "changelog"],
    );

    assert_eq!(out["ok"], true);
    assert_eq!(out["decision"]["id"], 1);
    assert_eq!(out["decision"]["kind"], "declined");
    assert_eq!(out["decision"]["intent"], "changelog");
    assert_eq!(out["decision"]["text_count"], 1);
    assert_eq!(out["texts_added"], 1);
    assert!(out["decision"].get("form").is_none());
    assert!(out["decision"].get("until").is_none());
    let after = scan(&root);
    assert_eq!(
        texts(&after["candidates"]),
        ["refresh test fixtures", "rotate signing keys"]
    );
    assert_eq!(
        after["suppressed"],
        json!([{"decision_id": 1, "kind": "declined", "intent": "changelog", "groups": 1}])
    );
}

#[test]
fn the_human_output_of_decide_and_scan_names_the_decision() {
    let (_guard, root) = hall();
    recurring(&root);
    let reference = group_ref(&root, "update changelog entry");

    let text = human(
        &root,
        &["decide", &reference, "--declined", "--intent", "changelog"],
    );
    assert!(
        text.contains("recorded decision 1: declined changelog"),
        "{text}"
    );
    assert!(text.contains("added 1 text(s)"));

    let scanned = human(&root, &["scan", "--no-drain"]);
    assert!(
        scanned.contains("1 group(s) suppressed by a decision"),
        "{scanned}"
    );
    assert!(scanned.contains("decision 1 (declined): changelog - 1 group(s)"));
    assert!(
        !scanned.contains("update changelog entry"),
        "a suppressed group is only counted"
    );
}

#[test]
fn a_decline_survives_the_group_gaining_new_texts() {
    let (_guard, root) = hall();
    recurring(&root);
    let reference = group_ref(&root, "update changelog entry");
    json_of(
        &root,
        &["decide", &reference, "--declined", "--intent", "changelog"],
    );

    plant(&root, &[("s4", -1, "update changelog entry again")]);

    let after = scan(&root);
    assert_eq!(after["suppressed"][0]["groups"], 1);
    assert!(!texts(&after["candidates"]).contains(&"update changelog entry".to_owned()));
}

#[test]
fn decide_deferred_defaults_to_thirty_days_and_takes_until() {
    let (_guard, root) = hall();
    recurring(&root);
    let changelog = group_ref(&root, "update changelog entry");
    let keys = group_ref(&root, "rotate signing keys");
    let until = day(60);

    let default = json_of(
        &root,
        &["decide", &changelog, "--deferred", "--intent", "changelog"],
    );
    let given = json_of(
        &root,
        &[
            "decide",
            &keys,
            "--deferred",
            "--intent",
            "keys",
            "--until",
            &until,
        ],
    );

    assert_eq!(default["decision"]["kind"], "deferred");
    assert_eq!(default["decision"]["until"], day(30));
    assert_eq!(given["decision"]["until"], until);
    assert_eq!(scan(&root)["suppressed"].as_array().unwrap().len(), 2);
    let text = human(&root, &["decisions"]);
    assert!(text.contains(&format!("until {}", day(30))), "{text}");
}

#[test]
fn decide_accepted_records_the_artifact_and_removes_the_candidate() {
    let (_guard, root) = hall();
    recurring(&root);
    std::fs::create_dir_all(root.join(".ivar/scripts")).unwrap();
    std::fs::write(root.join(".ivar/scripts/changelog.sh"), "").unwrap();
    let reference = group_ref(&root, "update changelog entry");

    let out = json_of(
        &root,
        &[
            "decide",
            &reference,
            "--accepted",
            "--intent",
            "changelog",
            "--form",
            "script",
            "--artifact",
            ".ivar/scripts/changelog.sh",
        ],
    );

    assert_eq!(out["decision"]["kind"], "accepted");
    assert_eq!(out["decision"]["form"], "script");
    assert_eq!(out["decision"]["artifact"], ".ivar/scripts/changelog.sh");
    assert!(out.get("warnings").is_none());
    let after = scan(&root);
    assert_eq!(after["candidates"].as_array().unwrap().len(), 2);
    assert_eq!(after["suppressed"], json!([]));
    let text = human(&root, &["decisions"]);
    assert!(
        text.contains("accepted changelog as script `.ivar/scripts/changelog.sh`"),
        "{text}"
    );
}

#[test]
fn accepting_with_a_missing_artifact_warns_and_leaves_the_group_a_candidate() {
    let (_guard, root) = hall();
    recurring(&root);
    let reference = group_ref(&root, "update changelog entry");

    let out = json_with_code(
        &root,
        &[
            "decide",
            &reference,
            "--accepted",
            "--intent",
            "changelog",
            "--form",
            "skill",
            "--artifact",
            ".ivar/skills/changelog/SKILL.md",
        ],
        1,
    );

    assert_eq!(out["decision"]["kind"], "accepted");
    assert_eq!(out["warnings"][0]["code"], "automation.artifact_missing");
    assert_eq!(scan(&root)["candidates"].as_array().unwrap().len(), 3);
}

#[test]
fn same_as_extends_a_decision_and_keeps_its_kind_and_until() {
    let (_guard, root) = hall();
    recurring(&root);
    let changelog = group_ref(&root, "update changelog entry");
    let keys = group_ref(&root, "rotate signing keys");
    let until = day(45);
    json_of(
        &root,
        &[
            "decide",
            &changelog,
            "--deferred",
            "--intent",
            "chores",
            "--until",
            &until,
        ],
    );

    let out = json_of(&root, &["decide", &keys, "--same-as", "1"]);

    assert_eq!(out["decision"]["id"], 1);
    assert_eq!(out["decision"]["kind"], "deferred");
    assert_eq!(out["decision"]["until"], until);
    assert_eq!(out["decision"]["intent"], "chores");
    assert_eq!(out["decision"]["text_count"], 2);
    assert_eq!(out["texts_added"], 1);
    let after = scan(&root);
    assert_eq!(texts(&after["candidates"]), ["refresh test fixtures"]);
    assert_eq!(after["suppressed"][0]["groups"], 2);
    let text = human(
        &root,
        &[
            "decide",
            &group_ref(&root, "refresh test fixtures"),
            "--same-as",
            "1",
        ],
    );
    assert!(
        text.contains("recorded decision 1: deferred chores"),
        "{text}"
    );
}

#[test]
fn undo_makes_the_group_a_candidate_again() {
    let (_guard, root) = hall();
    recurring(&root);
    let reference = group_ref(&root, "update changelog entry");
    json_of(
        &root,
        &["decide", &reference, "--declined", "--intent", "changelog"],
    );
    assert_eq!(scan(&root)["candidates"].as_array().unwrap().len(), 2);

    let out = json_of(&root, &["undo", "1"]);

    assert_eq!(out["removed"]["id"], 1);
    assert_eq!(out["removed"]["intent"], "changelog");
    assert_eq!(scan(&root)["candidates"].as_array().unwrap().len(), 3);
    assert_eq!(json_of(&root, &["decisions"])["decisions"], json!([]));
    failing(&root, &["undo", "1"])
        .code(2)
        .stderr(predicates::str::contains("no automation decision 1"));
}

#[test]
fn undo_in_human_output_names_what_was_removed() {
    let (_guard, root) = hall();
    recurring(&root);
    let reference = group_ref(&root, "update changelog entry");
    json_of(
        &root,
        &["decide", &reference, "--declined", "--intent", "changelog"],
    );

    let text = human(&root, &["undo", "1"]);

    assert!(
        text.contains("removed decision 1: declined changelog"),
        "{text}"
    );
}

#[test]
fn renew_rewrites_updated_at_of_an_accepted_decision() {
    let (_guard, root) = hall();
    recurring(&root);
    std::fs::write(root.join("changelog.md"), "").unwrap();
    let reference = group_ref(&root, "update changelog entry");
    json_of(
        &root,
        &[
            "decide",
            &reference,
            "--accepted",
            "--intent",
            "changelog",
            "--form",
            "procedure",
            "--artifact",
            "changelog.md",
        ],
    );
    // Age the decision so a rewrite is visible.
    rusqlite::Connection::open(root.join(".ivar/memory.db"))
        .unwrap()
        .execute(
            "UPDATE automation_decisions SET updated_at = '2020-01-01T00:00:00.000000000Z'",
            [],
        )
        .unwrap();

    let out = json_of(&root, &["renew", "1"]);

    assert_eq!(out["decision"]["id"], 1);
    assert_ne!(
        out["decision"]["updated_at"],
        "2020-01-01T00:00:00.000000000Z"
    );
    assert_eq!(
        out["decision"]["created_at"].as_str().unwrap().len(),
        out["decision"]["updated_at"].as_str().unwrap().len()
    );
    let listed = json_of(&root, &["decisions"]);
    assert_eq!(
        listed["decisions"][0]["updated_at"],
        out["decision"]["updated_at"]
    );
    let text = human(&root, &["renew", "1"]);
    assert!(
        text.contains("renewed decision 1: accepted changelog"),
        "{text}"
    );
}

#[test]
fn renew_refuses_a_decision_that_is_not_accepted_or_does_not_exist() {
    let (_guard, root) = hall();
    recurring(&root);
    let reference = group_ref(&root, "update changelog entry");
    json_of(
        &root,
        &["decide", &reference, "--declined", "--intent", "changelog"],
    );

    failing(&root, &["renew", "1"])
        .code(2)
        .stderr(predicates::str::contains(
            "only an accepted decision is renewed",
        ));
    failing(&root, &["renew", "9"])
        .code(2)
        .stderr(predicates::str::contains("no automation decision 9"));
}

#[test]
fn decisions_lists_every_kind_in_json_and_human_output() {
    let (_guard, root) = hall();
    recurring(&root);
    std::fs::write(root.join("changelog.md"), "").unwrap();
    let changelog = group_ref(&root, "update changelog entry");
    let keys = group_ref(&root, "rotate signing keys");
    let fixtures = group_ref(&root, "refresh test fixtures");
    json_of(
        &root,
        &["decide", &changelog, "--declined", "--intent", "changelog"],
    );
    json_of(&root, &["decide", &keys, "--deferred", "--intent", "keys"]);
    json_of(
        &root,
        &[
            "decide",
            &fixtures,
            "--accepted",
            "--intent",
            "fixtures",
            "--form",
            "agent",
            "--artifact",
            "changelog.md",
        ],
    );

    let out = json_of(&root, &["decisions"]);

    assert_eq!(out["ok"], true);
    let listed = out["decisions"].as_array().unwrap();
    let kinds: Vec<_> = listed.iter().map(|d| d["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["declined", "deferred", "accepted"]);
    assert_eq!(listed[2]["form"], "agent");
    assert_eq!(listed[1]["until"], day(30));
    let text = human(&root, &["decisions"]);
    assert!(text.contains("3 automation decision(s)"), "{text}");
    assert!(text.contains("decision 1: declined changelog"));
    assert!(text.contains("decision 2: deferred keys until"));
    assert!(text.contains("decision 3: accepted fixtures as agent `changelog.md`"));
}

#[test]
fn decisions_in_an_empty_hall_is_an_empty_list_and_creates_no_database() {
    let (_guard, root) = hall();

    let out = json_of(&root, &["decisions"]);

    assert_eq!(out["decisions"], json!([]));
    assert!(human(&root, &["decisions"]).contains("0 automation decision(s)"));
    assert!(!root.join(".ivar/memory.db").exists());
}

#[test]
fn a_ref_that_no_longer_exists_fails_with_a_clear_message() {
    let (_guard, root) = hall();
    recurring(&root);

    failing(&root, &["decide", "9999", "--declined", "--intent", "x"])
        .code(2)
        .stderr(predicates::str::contains("no request fingerprint 9999"));
    assert_eq!(json_of(&root, &["decisions"])["decisions"], json!([]));
}

#[test]
fn contradictory_or_incomplete_arguments_are_usage_errors() {
    let (_guard, root) = hall();
    recurring(&root);
    let reference = group_ref(&root, "update changelog entry");
    let cases: &[&[&str]] = &[
        &["decide", &reference],
        &["decide", &reference, "--declined"],
        &[
            "decide",
            &reference,
            "--declined",
            "--deferred",
            "--intent",
            "x",
        ],
        &["decide", &reference, "--accepted", "--intent", "x"],
        &[
            "decide",
            &reference,
            "--accepted",
            "--intent",
            "x",
            "--form",
            "tool",
            "--artifact",
            "a",
        ],
        &[
            "decide",
            &reference,
            "--declined",
            "--intent",
            "x",
            "--until",
            "2099-01-01",
        ],
        &[
            "decide",
            &reference,
            "--declined",
            "--intent",
            "x",
            "--form",
            "script",
        ],
        &["decide", &reference, "--same-as", "1", "--intent", "x"],
        &[
            "decide",
            &reference,
            "--same-as",
            "1",
            "--declined",
            "--intent",
            "x",
        ],
    ];
    for args in cases {
        failing(&root, args).code(2);
    }
    assert_eq!(json_of(&root, &["decisions"])["decisions"], json!([]));
}

#[test]
fn an_invalid_date_or_artifact_exits_2_and_writes_nothing() {
    let (_guard, root) = hall();
    recurring(&root);
    let reference = group_ref(&root, "update changelog entry");

    failing(
        &root,
        &[
            "decide",
            &reference,
            "--deferred",
            "--intent",
            "x",
            "--until",
            "next week",
        ],
    )
    .code(2)
    .stderr(predicates::str::contains("YYYY-MM-DD"));
    failing(
        &root,
        &[
            "decide",
            &reference,
            "--deferred",
            "--intent",
            "x",
            "--until",
            &day(-1),
        ],
    )
    .code(2)
    .stderr(predicates::str::contains("already past"));
    failing(
        &root,
        &[
            "decide",
            &reference,
            "--accepted",
            "--intent",
            "x",
            "--form",
            "script",
            "--artifact",
            "../outside.sh",
        ],
    )
    .code(2)
    .stderr(predicates::str::contains("must not contain `..`"));
    assert_eq!(json_of(&root, &["decisions"])["decisions"], json!([]));
}

#[test]
fn doctor_warns_about_an_accepted_decision_whose_artifact_is_missing() {
    let (_guard, root) = hall();
    recurring(&root);
    let reference = group_ref(&root, "update changelog entry");
    json_with_code(
        &root,
        &[
            "decide",
            &reference,
            "--accepted",
            "--intent",
            "changelog",
            "--form",
            "script",
            "--artifact",
            ".ivar/scripts/changelog.sh",
        ],
        1,
    );

    let out = ivar()
        .current_dir(&root)
        .env_remove("IVAR_SESSION_ID")
        .args(["--json", "doctor"])
        .assert()
        .get_output()
        .stdout
        .clone();
    let report: Value = serde_json::from_slice(&out).unwrap();

    let finding = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["code"] == "automation.artifact_missing")
        .unwrap_or_else(|| panic!("{report}"));
    assert!(
        finding["what"]
            .as_str()
            .unwrap()
            .contains(".ivar/scripts/changelog.sh")
    );
    assert!(
        finding["fix"]
            .as_str()
            .unwrap()
            .contains("ivar automation undo 1")
    );
}

#[test]
fn memory_reindex_leaves_the_decisions_alone() {
    let (_guard, root) = hall();
    recurring(&root);
    let reference = group_ref(&root, "update changelog entry");
    json_of(
        &root,
        &["decide", &reference, "--declined", "--intent", "changelog"],
    );

    ivar()
        .current_dir(&root)
        .env_remove("IVAR_SESSION_ID")
        .args(["memory", "reindex"])
        .assert()
        .success();

    assert_eq!(
        json_of(&root, &["decisions"])["decisions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(scan(&root)["suppressed"].as_array().unwrap().len(), 1);
}
