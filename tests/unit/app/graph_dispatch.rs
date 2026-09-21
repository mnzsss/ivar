#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::cli::graph::GraphExploreArgs;
use crate::domain::name::SessionId;
use crate::domain::provider::Provider;
use crate::domain::session::SessionState;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;

#[test]
fn a_cli_graph_call_records_its_session_and_query() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root);
    let db_path = layout.ivar_dir().join("memory.db");
    GraphDb::open(db_path.as_std_path()).unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-0000000008bb").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    SessionState::new(Provider::ClaudeCode, "2026-08-29T00:00:00Z")
        .write(&view_dir)
        .unwrap();

    dispatch_graph(
        GraphCommand::Explore(GraphExploreArgs {
            query: "enforceSession".to_owned(),
            repo: None,
        }),
        &Ctx::new(view_dir),
        true,
        false,
        &mut Vec::new(),
        &mut Vec::new(),
    );

    let db = GraphDb::open(db_path.as_std_path()).unwrap();
    let (_ts, query) = db
        .last_graph_call(session_id.as_str())
        .unwrap()
        .expect("the CLI call is keyed by the session");
    assert_eq!(query.as_deref(), Some("enforceSession"));
}
