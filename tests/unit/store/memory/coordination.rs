#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::*;
use crate::domain::coordination::{
    Address, HandoffState, MessageState, NewHandoff, NewMessage, Owner, Queue, Viewer,
};
use crate::domain::name::FeatureName;
use crate::store::memory::db::MemoryDb;

fn open() -> (tempfile::TempDir, std::path::PathBuf, MemoryDb) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memory.db");
    let db = MemoryDb::open(&path).unwrap();
    (dir, path, db)
}

fn owner(key: &str) -> Owner {
    Owner {
        key: key.to_owned(),
        provider: "claude-code".to_owned(),
    }
}

fn omp(session: &str) -> Viewer {
    Viewer {
        session_key: Some(session.to_owned()),
        provider: "omp".to_owned(),
        feature: None,
    }
}

fn handoff(id: &str, created_at: i64, expires_at: i64) -> NewHandoff {
    NewHandoff {
        id: id.to_owned(),
        queue: Queue::Feature(FeatureName::new("login").unwrap()),
        address: Some(Address::Provider("omp".to_owned())),
        owner: owner("s1"),
        summary: "half done".to_owned(),
        pending_tasks: vec!["wire the hook".to_owned()],
        open_decisions: vec![],
        modified_files: vec!["src/a.rs".to_owned()],
        created_at,
        expires_at,
    }
}

fn message(id: &str, created_at: i64) -> NewMessage {
    NewMessage {
        id: id.to_owned(),
        queue: Queue::Hall,
        address: None,
        sender: owner("s1"),
        subject: Some("api".to_owned()),
        body: "the route moved".to_owned(),
        created_at,
        expires_at: created_at + 100,
    }
}

#[test]
fn open_handoffs_round_trip_newest_first_and_skip_the_expired() {
    let (_dir, _path, db) = open();
    insert_handoff(db.conn(), &handoff("h1", 10, 1_000)).unwrap();
    insert_handoff(db.conn(), &handoff("h2", 20, 1_000)).unwrap();
    insert_handoff(db.conn(), &handoff("h3", 30, 40)).unwrap();

    let open = open_handoffs(db.conn(), 50).unwrap();

    let ids: Vec<&str> = open.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(ids, ["h2", "h1"]);
    assert_eq!(open[0].pending_tasks, ["wire the hook"]);
    assert_eq!(open[0].address, Some(Address::Provider("omp".to_owned())));
    assert_eq!(
        open[0].queue,
        Queue::Feature(FeatureName::new("login").unwrap())
    );
    assert_eq!(open[0].state, HandoffState::Open);
}

#[test]
fn a_handoff_is_claimed_once_even_from_two_connections() {
    let (_dir, path, db) = open();
    insert_handoff(db.conn(), &handoff("h1", 10, 1_000)).unwrap();
    let other = rusqlite::Connection::open(&path).unwrap();

    assert!(claim_handoff(db.conn(), "h1", "s2", &omp("s2"), 50).unwrap());
    assert!(!claim_handoff(&other, "h1", "s3", &omp("s3"), 51).unwrap());

    let claimed = get_handoff(db.conn(), "h1").unwrap().unwrap();
    assert_eq!(claimed.state, HandoffState::Accepted);
    assert_eq!(claimed.accepted_by.as_deref(), Some("s2"));
    assert_eq!(claimed.accepted_at, Some(50));
    assert!(!claim_handoff(db.conn(), "h1", "s2", &omp("s2"), 52).unwrap());
}

#[test]
fn an_expired_handoff_cannot_be_claimed() {
    let (_dir, _path, db) = open();
    insert_handoff(db.conn(), &handoff("h1", 10, 40)).unwrap();

    assert!(!claim_handoff(db.conn(), "h1", "s2", &omp("s2"), 50).unwrap());
}

#[test]
fn only_the_owner_cancels_unless_no_owner_is_given() {
    let (_dir, _path, db) = open();
    insert_handoff(db.conn(), &handoff("h1", 10, 1_000)).unwrap();
    insert_handoff(db.conn(), &handoff("h2", 10, 1_000)).unwrap();

    assert!(!cancel_handoff(db.conn(), "h1", Some("intruder")).unwrap());
    assert!(cancel_handoff(db.conn(), "h1", Some("s1")).unwrap());
    assert!(cancel_handoff(db.conn(), "h2", None).unwrap());
    assert_eq!(
        get_handoff(db.conn(), "h1").unwrap().unwrap().state,
        HandoffState::Cancelled
    );
    assert!(owner_has_handoff(db.conn(), "s1").unwrap());
    assert!(!owner_has_handoff(db.conn(), "s9").unwrap());
}

#[test]
fn a_message_pops_once_and_the_sender_cancels_the_rest() {
    let (_dir, _path, db) = open();
    insert_message(db.conn(), &message("m1", 10)).unwrap();
    insert_message(db.conn(), &message("m2", 20)).unwrap();
    insert_message(db.conn(), &message("m3", 30)).unwrap();

    let pending: Vec<String> = pending_messages(db.conn(), 50)
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(pending, ["m1", "m2", "m3"]);
    assert!(pop_message(db.conn(), "m1", "s2", &omp("s2"), 50).unwrap());
    assert!(!pop_message(db.conn(), "m1", "s3", &omp("s3"), 51).unwrap());
    assert_eq!(
        get_message(db.conn(), "m1").unwrap().unwrap().state,
        MessageState::Popped
    );
    assert_eq!(cancel_messages(db.conn(), "intruder", None).unwrap(), 0);
    assert_eq!(cancel_messages(db.conn(), "s1", Some("m2")).unwrap(), 1);
    assert_eq!(cancel_messages(db.conn(), "s1", None).unwrap(), 1);
    assert!(pending_messages(db.conn(), 53).unwrap().is_empty());
}

#[test]
fn overdue_rows_expire_and_terminal_rows_are_pruned_by_their_age() {
    let (_dir, _path, db) = open();
    insert_handoff(db.conn(), &handoff("h1", 10, 40)).unwrap();
    insert_handoff(db.conn(), &handoff("h2", 10, 1_000)).unwrap();
    insert_message(db.conn(), &message("m1", 10)).unwrap();

    let expired = expire_overdue(db.conn(), 200).unwrap();

    assert_eq!(expired.handoffs, ["h1"]);
    assert_eq!(expired.messages, 1);
    assert_eq!(
        get_handoff(db.conn(), "h1").unwrap().unwrap().state,
        HandoffState::Expired
    );
    assert_eq!(
        get_handoff(db.conn(), "h2").unwrap().unwrap().state,
        HandoffState::Open
    );
    assert_eq!(prune_terminal(db.conn(), 40).unwrap(), 0);
    assert_eq!(prune_terminal(db.conn(), 200).unwrap(), 2);
    assert!(get_handoff(db.conn(), "h1").unwrap().is_none());
    assert!(get_handoff(db.conn(), "h2").unwrap().is_some());
}

#[test]
fn a_row_addressed_to_a_blank_value_is_not_read_back() {
    let (_dir, _path, db) = open();
    insert_handoff(db.conn(), &handoff("h1", 10, 1_000)).unwrap();
    db.conn()
        .execute("UPDATE handoffs SET to_value = ' ' WHERE id = 'h1'", [])
        .unwrap();

    assert!(get_handoff(db.conn(), "h1").unwrap().is_none());
    assert!(open_handoffs(db.conn(), 50).unwrap().is_empty());
}

#[test]
fn a_claim_by_a_viewer_the_row_is_not_addressed_to_changes_nothing() {
    let (_dir, _path, db) = open();
    insert_handoff(db.conn(), &handoff("h1", 10, 1_000)).unwrap();
    let mut addressed = message("m1", 10);
    addressed.address = Some(Address::Session("s2".to_owned()));
    insert_message(db.conn(), &addressed).unwrap();
    let claude = Viewer {
        provider: "claude-code".to_owned(),
        ..omp("s9")
    };

    assert!(!claim_handoff(db.conn(), "h1", "s9", &claude, 50).unwrap());
    assert!(!pop_message(db.conn(), "m1", "s3", &omp("s3"), 50).unwrap());
    assert_eq!(
        get_handoff(db.conn(), "h1").unwrap().unwrap().state,
        HandoffState::Open
    );
    assert!(claim_handoff(db.conn(), "h1", "s9", &omp("s9"), 50).unwrap());
    assert!(pop_message(db.conn(), "m1", "s2", &omp("s2"), 50).unwrap());
}
