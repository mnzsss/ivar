//! The one door to the graph database path: it moves a legacy
//! `memory.db`, refuses while another connection holds it, and the
//! best-effort recorders never go through it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::action::Ctx;
use crate::action::graph::{CleanInput, clean_cmd, graph_db_path, record_usage};
use crate::domain::graph::{UsageEvent, UsageSource};
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;
use crate::store::memory::schema::APPLICATION_ID;
use crate::store::memory::split::is_legacy_graph_db;
use crate::test_support::seeded_hall;

fn legacy_graph_db(layout: &Layout) -> GraphDb {
    let db = GraphDb::open(layout.memory_db().as_std_path()).unwrap();
    db.insert_repo("api", "/x/api", "main", None).unwrap();
    db
}

#[test]
fn the_port_moves_a_legacy_database_and_returns_graph_db() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    drop(legacy_graph_db(&layout));

    let path = graph_db_path(&layout).unwrap();

    assert_eq!(path, layout.graph_db());
    let graph = GraphDb::open_read_only(path.as_std_path()).unwrap();
    assert!(graph.get_repo("api").unwrap().is_some());
    let memory = MemoryDb::inspect(layout.memory_db().as_std_path()).unwrap();
    assert_eq!(memory.application_id, APPLICATION_ID);
}

#[test]
fn a_legacy_database_held_open_elsewhere_is_refused_as_split_busy() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let _held = legacy_graph_db(&layout);

    let failure = graph_db_path(&layout).unwrap_err();

    assert_eq!(failure.code, "memory.split_busy");
    assert!(!layout.graph_db().as_std_path().exists());
    assert!(is_legacy_graph_db(layout.memory_db().as_std_path()).unwrap());
}

#[test]
fn graph_clean_all_on_a_legacy_hall_moves_it_without_waiting_on_its_own_index_lock() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    drop(legacy_graph_db(&layout));

    let report = clean_cmd(
        &Ctx::new(root),
        CleanInput {
            repo: None,
            feature: None,
            all: true,
            path: None,
        },
    )
    .unwrap();

    assert_eq!(report.value.repos_removed, 1);
    assert!(layout.graph_db().as_std_path().exists());
}

#[test]
fn recording_usage_never_moves_a_legacy_database() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    drop(legacy_graph_db(&layout));

    record_usage(
        &Ctx::new(root),
        &UsageEvent {
            command: "find".to_owned(),
            source: UsageSource::Cli,
            duration_ms: 1,
            result_count: Some(0),
            error: false,
            session: None,
            query: None,
        },
    );

    assert!(!layout.graph_db().as_std_path().exists());
    assert!(is_legacy_graph_db(layout.memory_db().as_std_path()).unwrap());
}
