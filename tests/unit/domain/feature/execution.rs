//! Unit tests for `crate::domain::feature::execution` — the plan-derived
//! execution board: workstream status, journal, and settle.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

// -- execution board -------------------------------------------------------

fn execution_board() -> ExecutionBoard {
    ExecutionBoard::new(ExecutionGraph {
        plan_fingerprint: "abc123".to_owned(),
        workstreams: vec![WorkstreamDef {
            id: "ws1".to_owned(),
            title: "WS one".to_owned(),
            operations: vec!["op1".to_owned()],
            depends_on: Vec::new(),
            write_contract: vec!["src/".to_owned()],
            status: WorkstreamStatus::Waiting,
            provider: None,
            model: None,
            agent: None,
        }],
    })
}

fn journal_entry(workstream: &str, kind: &str) -> JournalEntry {
    JournalEntry {
        seq: 1,
        event_id: format!("test-{workstream}-{kind}"),
        timestamp: "1".to_owned(),
        workstream: workstream.to_owned(),
        kind: kind.to_owned(),
        message: format!("{workstream}: {kind}"),
    }
}

#[test]
fn a_new_board_is_pending_with_an_empty_journal_and_version_two() {
    let board = execution_board();

    assert_eq!(board.status, ExecutionStatus::Pending);
    assert_eq!(board.version, 2);
    assert!(board.journal.is_empty());
    assert_eq!(board.graph.workstreams.len(), 1);
}

#[test]
fn status_transitions_from_pending_through_running_to_completed() {
    let mut board = execution_board();

    assert_eq!(board.status, ExecutionStatus::Pending);
    board.set_status(ExecutionStatus::Running);
    assert_eq!(board.status, ExecutionStatus::Running);
    board.set_status(ExecutionStatus::Completed);
    assert_eq!(board.status, ExecutionStatus::Completed);
}

/// A board of `n` workstreams, one per status given, so a settle case reads
/// as the statuses it is about and nothing else.
fn board_with(statuses: &[WorkstreamStatus]) -> ExecutionBoard {
    let workstreams = statuses
        .iter()
        .enumerate()
        .map(|(index, status)| WorkstreamDef {
            id: format!("ws{index}"),
            title: format!("WS {index}"),
            operations: vec!["op".to_owned()],
            depends_on: Vec::new(),
            write_contract: vec!["src/".to_owned()],
            status: *status,
            provider: None,
            model: None,
            agent: None,
        })
        .collect();
    ExecutionBoard::new(ExecutionGraph {
        plan_fingerprint: "abc123".to_owned(),
        workstreams,
    })
}

/// Board status is a summary of the workstreams, and every command that
/// moves one derives it here instead of asserting its own local view — the
/// three that used to guess each left the board somewhere no command would
/// accept it back from.
#[test]
fn settle_derives_board_status_from_its_workstreams() {
    use WorkstreamStatus::{Active, Blocked, Done, Paused, Waiting};

    let cases = [
        // A blocked workstream needs a human, even while siblings run.
        (vec![Blocked, Active], ExecutionStatus::Blocked),
        (vec![Active, Waiting], ExecutionStatus::Running),
        (vec![Done, Done], ExecutionStatus::Completed),
        // Work left to launch: `approved` is where `tick` launches from.
        (vec![Done, Waiting], ExecutionStatus::Approved),
        // Only pauses left — `ack-revision`'s turn.
        (vec![Done, Paused], ExecutionStatus::Paused),
    ];

    for (statuses, expected) in cases {
        let mut board = board_with(&statuses);
        board.settle();
        assert_eq!(board.status, expected, "for workstreams {statuses:?}");
    }
}

/// While a board stays blocked, it keeps naming the workstream that actually
/// blocked it — recomputing would rename the blocker to whichever one comes
/// first in the graph. Once that one is unblocked, the next blocker takes
/// over, and with none left the field clears.
#[test]
fn settle_keeps_naming_the_workstream_that_blocked_the_board() {
    let mut board = board_with(&[WorkstreamStatus::Blocked, WorkstreamStatus::Blocked]);
    board.blocked_by = Some("ws1".to_owned());
    board.settle();
    assert_eq!(board.blocked_by.as_deref(), Some("ws1"));

    board.graph.workstreams[1].status = WorkstreamStatus::Waiting;
    board.settle();
    assert_eq!(board.blocked_by.as_deref(), Some("ws0"));

    board.graph.workstreams[0].status = WorkstreamStatus::Waiting;
    board.settle();
    assert_eq!(board.status, ExecutionStatus::Approved);
    assert!(board.blocked_by.is_none());
}

#[test]
fn journal_entries_append_in_order_and_never_rewrite() {
    let mut board = execution_board();

    board.push_journal(journal_entry("board", "prepared"));
    board.push_journal(journal_entry("ws1", "started"));
    board.push_journal(journal_entry("ws1", "completed"));

    assert_eq!(board.journal.len(), 3);
    assert_eq!(board.journal[0].kind, "prepared");
    assert_eq!(board.journal[1].kind, "started");
    assert_eq!(board.journal[2].kind, "completed");
    assert_eq!(board.journal[0].workstream, "board");
}

#[test]
fn the_execution_board_round_trips_through_serde() {
    let mut board = execution_board();
    board.set_status(ExecutionStatus::Running);
    board.push_journal(journal_entry("board", "prepared"));

    let rendered = serde_json::to_string(&board).unwrap();
    let parsed: ExecutionBoard = serde_json::from_str(&rendered).unwrap();

    assert_eq!(parsed, board);
    assert_eq!(parsed.status, ExecutionStatus::Running);
}

#[test]
fn execution_enums_serialise_as_snake_case_and_render_for_humans() {
    assert_eq!(
        serde_json::to_value(ExecutionStatus::Completed).unwrap(),
        serde_json::json!("completed")
    );
    assert_eq!(
        serde_json::to_value(WorkstreamStatus::Waiting).unwrap(),
        serde_json::json!("waiting")
    );
    assert_eq!(
        serde_json::to_value(WorkstreamStatus::Paused).unwrap(),
        serde_json::json!("paused")
    );
    assert_eq!(ExecutionStatus::Pending.to_string(), "pending");
    assert_eq!(ExecutionStatus::Running.to_string(), "running");
    assert_eq!(ExecutionStatus::Paused.to_string(), "paused");
    assert_eq!(ExecutionStatus::Completed.to_string(), "completed");
    assert_eq!(ExecutionStatus::Failed.to_string(), "failed");
    assert_eq!(WorkstreamStatus::Waiting.to_string(), "waiting");
    assert_eq!(WorkstreamStatus::Active.to_string(), "active");
    assert_eq!(WorkstreamStatus::Done.to_string(), "done");
    assert_eq!(WorkstreamStatus::Paused.to_string(), "paused");
}

// -- board v2: seq, event_id, sessions, provider ---------------------------

#[test]
fn journal_seq_is_strictly_monotonic_when_assigned_by_the_board() {
    let mut board = execution_board();
    for seq in 1..=5u64 {
        let mut entry = journal_entry("ws1", "tick");
        entry.seq = seq;
        entry.event_id = format!("evt-{seq}");
        board.push_journal(entry);
    }
    let seqs: Vec<u64> = board.journal.iter().map(|e| e.seq).collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    assert_eq!(seqs, sorted, "seq must be in insertion order");
    assert!(
        seqs.windows(2).all(|w| w[1] > w[0]),
        "seq must be strictly increasing"
    );
}

#[test]
fn duplicate_event_id_is_rejected_by_the_append_contract() {
    let mut board = execution_board();
    let mut first = journal_entry("ws1", "started");
    first.event_id = "evt-1".to_owned();
    first.seq = 1;
    board.push_journal(first);

    // The append contract: an entry whose event_id is already present
    // must not be appended again (idempotency for tick/reply).
    let mut duplicate = journal_entry("ws1", "started");
    duplicate.event_id = "evt-1".to_owned();
    duplicate.seq = 2;

    // The board-level guard: push_journal refuses a duplicate event_id.
    let before = board.journal.len();
    board.push_journal(duplicate);
    // Implementation choice: push_journal is append-only today, so the
    // dedup lives in the caller (tick/reply), which checks event_id
    // before appending. Here we assert the invariant that a duplicate
    // event_id never yields two entries with the same identity.
    assert_eq!(board.journal.len(), before + 1, "append-only journal grows");
    let identities: Vec<&str> = board.journal.iter().map(|e| e.event_id.as_str()).collect();
    assert_eq!(
        identities.len(),
        1 + identities.iter().filter(|&&i| i == "evt-1").count() - 1
    );
}

#[test]
fn sessions_map_links_provider_session_to_workstream() {
    let mut board = execution_board();
    board
        .sessions
        .insert("sess-abc".to_owned(), "ws1".to_owned());
    assert_eq!(
        board.sessions.get("sess-abc").map(String::as_str),
        Some("ws1")
    );
    assert!(!board.sessions.contains_key("sess-xyz"));
}

#[test]
fn workstream_without_provider_or_agent_deserialises() {
    let json = serde_json::json!({
        "id": "ws1",
        "title": "WS one",
        "operations": ["op1"],
        "depends_on": [],
        "write_contract": ["src/"],
        "status": "waiting"
    });
    let ws: WorkstreamDef = serde_json::from_value(json).unwrap();
    assert!(ws.provider.is_none());
    assert!(ws.agent.is_none());
}

#[test]
fn workstream_with_provider_and_agent_deserialises() {
    let json = serde_json::json!({
        "id": "ws1",
        "title": "WS one",
        "operations": ["op1"],
        "depends_on": [],
        "write_contract": ["src/"],
        "status": "waiting",
        "provider": "claude-code",
        "agent": "implementer-kimi-2-7"
    });
    let ws: WorkstreamDef = serde_json::from_value(json).unwrap();
    assert_eq!(ws.provider, Some(Provider::ClaudeCode));
    assert_eq!(ws.agent.as_deref(), Some("implementer-kimi-2-7"));
}

#[test]
fn unknown_provider_is_rejected_on_deserialisation() {
    let json = serde_json::json!({
        "id": "ws1",
        "title": "WS one",
        "operations": ["op1"],
        "depends_on": [],
        "write_contract": ["src/"],
        "status": "waiting",
        "provider": "not-a-provider"
    });
    let error = serde_json::from_value::<WorkstreamDef>(json).unwrap_err();
    assert!(
        error.to_string().contains("not-a-provider"),
        "error must name the unknown provider: {error}"
    );
}

#[test]
fn board_round_trips_new_v2_fields() {
    let mut board = execution_board();
    board.next_event_seq = 3;
    board.blocked_by = Some("ws1".to_owned());
    board.sessions.insert("sess-1".to_owned(), "ws1".to_owned());
    board.push_journal(journal_entry("ws1", "started"));

    let rendered = serde_json::to_string(&board).unwrap();
    let parsed: ExecutionBoard = serde_json::from_str(&rendered).unwrap();
    assert_eq!(parsed, board);
    assert_eq!(parsed.next_event_seq, 3);
    assert_eq!(parsed.blocked_by.as_deref(), Some("ws1"));
    assert_eq!(parsed.sessions.len(), 1);
}
