#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::common::{hall_root, ivar};

#[test]
fn begin_list_and_cancel_round_trip_on_the_terminal() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    let begun = ivar()
        .current_dir(&root)
        .env_remove("IVAR_SESSION_ID")
        .args([
            "memory",
            "handoff",
            "begin",
            "--summary",
            "pick up the docs",
            "--json",
        ])
        .assert()
        .success();
    let opened: serde_json::Value = serde_json::from_slice(&begun.get_output().stdout).unwrap();
    let id = opened["handoff_id"].as_str().unwrap().to_owned();

    let listed = ivar()
        .current_dir(&root)
        .args(["memory", "handoff", "list", "--json"])
        .assert()
        .success();
    let rows: serde_json::Value = serde_json::from_slice(&listed.get_output().stdout).unwrap();
    assert_eq!(rows["handoffs"][0]["summary"], "pick up the docs");

    ivar()
        .current_dir(&root)
        .args(["memory", "handoff", "cancel", &id])
        .assert()
        .success();
    let after = ivar()
        .current_dir(&root)
        .args(["memory", "handoff", "list", "--json"])
        .assert()
        .success();
    let rows: serde_json::Value = serde_json::from_slice(&after.get_output().stdout).unwrap();
    assert_eq!(rows["handoffs"], serde_json::json!([]));
}
