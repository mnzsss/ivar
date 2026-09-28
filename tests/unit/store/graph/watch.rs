#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate::store::graph::db::GraphDb;

#[test]
fn a_scope_is_settled_only_after_its_catchup_and_every_observed_burst_are_indexed() {
    let db = GraphDb::open_in_memory().unwrap();
    db.watch_register("base:api").unwrap();
    assert!(!db.watch_settled(&["base:api"]).unwrap(), "a registered scope needs catch-up");

    db.watch_finish("base:api", 0, true).unwrap();
    assert!(db.watch_settled(&["base:api"]).unwrap());

    let seq = db.watch_bump_observed("base:api").unwrap();
    assert_eq!(seq, 1);
    assert!(!db.watch_settled(&["base:api"]).unwrap(), "an observed burst is pending");

    db.watch_finish("base:api", seq, false).unwrap();
    assert!(db.watch_settled(&["base:api"]).unwrap());
}

#[test]
fn a_scope_the_leader_never_registered_is_never_settled() {
    let db = GraphDb::open_in_memory().unwrap();
    assert!(!db.watch_settled(&["layer:feat:api"]).unwrap());
}

#[test]
fn a_failure_unsettles_the_scope_and_records_the_reason() {
    let db = GraphDb::open_in_memory().unwrap();
    db.watch_register("base:api").unwrap();
    db.watch_finish("base:api", 0, true).unwrap();
    db.watch_fail("base:api", "index_repo: disk full").unwrap();
    assert!(!db.watch_settled(&["base:api"]).unwrap());
    let rows = db.watch_scopes().unwrap();
    assert_eq!(rows[0].error.as_deref(), Some("index_repo: disk full"));
    assert!(rows[0].needs_catchup);
}

#[test]
fn an_older_sequence_never_moves_indexed_backwards() {
    let db = GraphDb::open_in_memory().unwrap();
    db.watch_register("base:api").unwrap();
    db.watch_bump_observed("base:api").unwrap();
    db.watch_bump_observed("base:api").unwrap();
    db.watch_finish("base:api", 2, true).unwrap();
    db.watch_finish("base:api", 1, false).unwrap();
    assert!(db.watch_settled(&["base:api"]).unwrap());
    assert_eq!(db.watch_scopes().unwrap()[0].indexed, 2);
}

#[test]
fn takeover_marks_every_scope_for_catchup() {
    let db = GraphDb::open_in_memory().unwrap();
    for scope in ["base:api", "layer:feat:api"] {
        db.watch_register(scope).unwrap();
        db.watch_finish(scope, 0, true).unwrap();
    }
    db.watch_mark_all_catchup().unwrap();
    assert!(!db.watch_settled(&["base:api"]).unwrap());
    assert!(!db.watch_settled(&["layer:feat:api"]).unwrap());
}
