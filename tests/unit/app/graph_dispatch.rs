#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::cli::graph::GraphExploreArgs;
use crate::domain::name::SessionId;
use crate::test_support::{graph_session_view, last_graph_query, seeded_hall};

#[test]
fn a_cli_graph_call_records_its_session_and_query() {
    let (_guard, root) = seeded_hall();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-0000000008bb").unwrap();
    let view_dir = graph_session_view(&root, &session_id);

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

    let query = last_graph_query(&root, session_id.as_str())
        .expect("the CLI call is keyed by the session");
    assert_eq!(query.as_deref(), Some("enforceSession"));
}
