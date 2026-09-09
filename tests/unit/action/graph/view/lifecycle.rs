#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::str_to_string
)]

use tempfile::tempdir;

use crate::action::feature::workspace::OpenAttempt;
use crate::action::graph::input::GraphViewInput;
use crate::action::graph::outcome::GraphViewOutcome;
use crate::action::graph::view::lifecycle::prepare_view_session;
use crate::action::graph::view::types::ViewSeed;
use crate::error::WriteHuman;
use crate::store::graph::db::GraphDb;
#[test]
fn test_prepare_view_session_binds_and_creates_outcome() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("memory.db");
    let db = GraphDb::open(&db_path).unwrap();

    let input = GraphViewInput {
        seed: ViewSeed::Symbol("domain_symbol".into()),
        depth: 2,
        limit: 400,
        no_open: true,
    };

    let session = prepare_view_session(db, input).expect("prepares view session");
    assert!(session.url().starts_with("http://127.0.0.1:"));
    assert_eq!(
        session.outcome.seed,
        ViewSeed::Symbol("domain_symbol".into())
    );
    assert!(matches!(session.outcome.open, OpenAttempt::NotRequested));
}

#[test]
fn test_view_outcome_human_and_compact_rendering() {
    let outcome = GraphViewOutcome {
        url: "http://127.0.0.1:54321".into(),
        seed: ViewSeed::Default,
        open: OpenAttempt::NotRequested,
    };

    let mut buf = Vec::new();
    outcome.write_human(&mut buf).unwrap();
    let human = String::from_utf8(buf).unwrap();
    assert!(human.contains("Serving graph viewer at http://127.0.0.1:54321"));

    let compact = crate::action::graph::compact::ToCompact::to_compact(&outcome);
    assert!(compact.contains("http://127.0.0.1:54321"));
}
