#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::execute::prepare::{self as prepare_action, PrepareInput as PrepareActionInput};
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::domain::feature::ExecutionBoard;
use crate::error::Status;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::test_support::hall_root;

const GRAPH_JSON: &str = r#"{
    "workstreams": [
        {
            "id": "ws-src",
            "title": "Source files",
            "operations": ["write-code"],
            "depends_on": [],
            "write_contract": ["src/"],
            "provider": "claude-code"
        },
        {
            "id": "ws-docs",
            "title": "Docs",
            "operations": ["write-docs"],
            "depends_on": [],
            "write_contract": ["docs/"],
            "provider": "claude-code"
        }
    ]
}"#;

/// A plan that backs `GRAPH_JSON`. `prepare` refuses a graph whose
/// operations the plan does not document, so the scaffolded plan
/// `plan create` writes is not enough to seed a board with.
const PLAN_TEXT: &str = r#"# Plan

## Operations

### ws-src
- write-code
write_contract:
- src/

### ws-docs
- write-docs
write_contract:
- docs/

## Operation details

**write-code** — Implement write-code.

**write-docs** — Implement write-docs.
"#;

/// A hall with a prepared board, sessions injected, and status set to Blocked.
fn seeded_blocked_board() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());

    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();

    feature_create::create(
        &ctx,
        FeatureCreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    plan_create::create(
        &ctx,
        PlanCreateInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    fs::write_text(&root.join("plans/checkout/plan.md"), PLAN_TEXT).unwrap();

    let graph = root.join("graph.json");
    fs::write_text(&graph, GRAPH_JSON).unwrap();
    prepare_action::prepare(
        &ctx,
        PrepareActionInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
            session: None,
        },
    )
    .unwrap();

    // Inject sessions and set board to Blocked.
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
    board
        .sessions
        .insert("sess-src".to_owned(), "ws-src".to_owned());
    board
        .sessions
        .insert("sess-docs".to_owned(), "ws-docs".to_owned());
    board.set_status(ExecutionStatus::Blocked);
    board.blocked_by = Some("ws-src".to_owned());
    board
        .graph
        .workstreams
        .iter_mut()
        .find(|ws| ws.id == "ws-src")
        .unwrap()
        .status = WorkstreamStatus::Blocked;
    board.write(&layout, &feature).unwrap();

    (guard, root)
}

#[test]
fn reply_is_blocked_when_board_is_not_blocked() {
    // A board with sessions but status other than `Blocked` — the reply
    // must be refused naming the status, not the session.
    let (_guard, root) = seeded_blocked_board();
    let ctx = Ctx::new(root.clone());

    // Move the board off Blocked so only the status gate can fire.
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
    board.set_status(ExecutionStatus::Running);
    board.write(&layout, &feature).unwrap();

    let failure = reply(
        &ctx,
        ReplyInput {
            feature: Some("checkout".to_owned()),
            session: Some("sess-src".to_owned()),
            message: "fix it".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "execute.reply.not_blocked");
}

#[test]
fn reply_records_journal_entry_and_clears_blocked_by() {
    let (_guard, root) = seeded_blocked_board();
    let ctx = Ctx::new(root.clone());

    let outcome = reply(
        &ctx,
        ReplyInput {
            feature: Some("checkout".to_owned()),
            session: Some("sess-src".to_owned()),
            message: "I fixed the issue".to_owned(),
        },
    )
    .unwrap();

    assert!(outcome.is_clean());
    assert_eq!(outcome.value.workstream, "ws-src");

    // Verify persistence.
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();

    // The board is tickable again.
    assert_eq!(board.status, ExecutionStatus::Approved);
    // blocked_by is cleared.
    assert!(board.blocked_by.is_none());

    // A new journal entry was appended.
    let last_entry = board.journal.last().unwrap();
    assert_eq!(last_entry.kind, "human.replied");
    assert_eq!(last_entry.workstream, "ws-src");
    assert_eq!(last_entry.message, "I fixed the issue");
    assert_eq!(
        last_entry.event_id,
        build_event_id("ws-src", "I fixed the issue")
    );
    assert_eq!(last_entry.seq, outcome.value.entry_seq);
}

#[test]
fn reply_lands_the_message_in_the_workstream_inbox() {
    let (_guard, root) = seeded_blocked_board();
    let ctx = Ctx::new(root.clone());

    reply(
        &ctx,
        ReplyInput {
            feature: Some("checkout".to_owned()),
            session: Some("sess-src".to_owned()),
            message: "inbox test line".to_owned(),
        },
    )
    .unwrap();

    // Read the inbox file.
    let layout = Layout::at(root.clone());
    let inbox_path = layout.execution_inbox(&FeatureName::new("checkout").unwrap(), "ws-src");
    let content = fs::read_text(&inbox_path).unwrap().unwrap();
    let lines: Vec<&str> = content.trim_end().split('\n').collect();
    assert_eq!(lines.len(), 1);

    // Parse the JSONL line.
    let entry: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(entry["kind"], "inbox");
    assert_eq!(entry["message"], "inbox test line");
}

/// A reply has to hand the workstream back to something that can act on it.
/// It used to leave it `Active` on a `Running` board — a state no command
/// accepts: `tick` demands `approved` and only launches `Waiting`, `approve`
/// demands `awaiting_approval`, `reply` demands `blocked`. The question got
/// answered and the workstream was stranded in the same breath.
#[test]
fn reply_returns_the_workstream_to_waiting_for_the_next_tick() {
    let (_guard, root) = seeded_blocked_board();
    let ctx = Ctx::new(root.clone());

    reply(
        &ctx,
        ReplyInput {
            feature: Some("checkout".to_owned()),
            session: Some("sess-src".to_owned()),
            message: "unblocking".to_owned(),
        },
    )
    .unwrap();

    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();

    let ws = board
        .graph
        .workstreams
        .iter()
        .find(|ws| ws.id == "ws-src")
        .unwrap();
    assert_eq!(ws.status, WorkstreamStatus::Waiting);

    // The other workstream should be unchanged.
    let docs = board
        .graph
        .workstreams
        .iter()
        .find(|ws| ws.id == "ws-docs")
        .unwrap();
    assert_eq!(docs.status, WorkstreamStatus::Waiting);
}

#[test]
fn replying_twice_with_same_content_does_not_duplicate() {
    let (_guard, root) = seeded_blocked_board();
    let ctx = Ctx::new(root.clone());

    let first = reply(
        &ctx,
        ReplyInput {
            feature: Some("checkout".to_owned()),
            session: Some("sess-src".to_owned()),
            message: "same fix again".to_owned(),
        },
    )
    .unwrap();

    let second = reply(
        &ctx,
        ReplyInput {
            feature: Some("checkout".to_owned()),
            session: Some("sess-src".to_owned()),
            message: "same fix again".to_owned(),
        },
    )
    .unwrap();

    // Both return the same seq.
    assert_eq!(first.value.entry_seq, second.value.entry_seq);

    // Only one journal entry with this event_id exists.
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();

    let count = board
        .journal
        .iter()
        .filter(|e| e.event_id == build_event_id("ws-src", "same fix again"))
        .count();
    assert_eq!(
        count, 1,
        "duplicate event_id should not create a second entry"
    );

    // Inbox has exactly one line.
    let inbox_path = layout.execution_inbox(&feature, "ws-src");
    let content = fs::read_text(&inbox_path).unwrap().unwrap();
    let lines: Vec<&str> = content.trim_end().split('\n').collect();
    assert_eq!(lines.len(), 1);
}

#[test]
fn unknown_session_returns_blocked() {
    let (_guard, root) = seeded_blocked_board();
    let ctx = Ctx::new(root.clone());

    let failure = reply(
        &ctx,
        ReplyInput {
            feature: Some("checkout".to_owned()),
            session: Some("sess-ghost".to_owned()),
            message: "nothing".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "execute.reply.unknown_session");
}

#[test]
fn missing_feature_argument_returns_blocked() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());

    let failure = reply(
        &ctx,
        ReplyInput {
            feature: None,
            session: Some("sess-src".to_owned()),
            message: "nothing".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "execute.reply.missing_feature");
}

#[test]
fn missing_session_argument_returns_blocked() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());

    let failure = reply(
        &ctx,
        ReplyInput {
            feature: Some("checkout".to_owned()),
            session: None,
            message: "nothing".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "execute.reply.missing_session");
}

/// One reply does not unblock a board two workstreams are blocking. The
/// board stays `Blocked`, now naming the one that is still waiting on a
/// human.
#[test]
fn a_board_with_another_blocked_workstream_stays_blocked() {
    let (_guard, root) = seeded_blocked_board();
    let ctx = Ctx::new(root.clone());

    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    {
        let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
        board
            .graph
            .workstreams
            .iter_mut()
            .find(|ws| ws.id == "ws-docs")
            .unwrap()
            .status = WorkstreamStatus::Blocked;
        board.write(&layout, &feature).unwrap();
    }

    reply(
        &ctx,
        ReplyInput {
            feature: Some("checkout".to_owned()),
            session: Some("sess-src".to_owned()),
            message: "src answered".to_owned(),
        },
    )
    .unwrap();

    let board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
    assert_eq!(board.status, ExecutionStatus::Blocked);
    assert_eq!(
        board.blocked_by.as_deref(),
        Some("ws-docs"),
        "the board must name the workstream still waiting on a human"
    );
}
