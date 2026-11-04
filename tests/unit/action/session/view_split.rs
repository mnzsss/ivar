#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::materialise;
use crate::domain::provider::Provider;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::store::memory::split::is_legacy_graph_db;
use crate::test_support::seeded_hall;

fn legacy(layout: &Layout) -> GraphDb {
    let db = GraphDb::open(layout.memory_db().as_std_path()).unwrap();
    db.insert_repo("api", "/x/api", "main", None).unwrap();
    db
}

#[test]
fn opening_a_session_view_moves_a_legacy_graph_database() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    drop(legacy(&layout));
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let view_dir = layout.ivar_dir().join("sessions/view-split");

    let report = materialise(&layout, &manifest, None, Provider::ClaudeCode, &view_dir).unwrap();

    assert!(
        report
            .warnings
            .iter()
            .all(|w| w.code != "memory.split_deferred")
    );
    let graph = GraphDb::open_read_only(layout.graph_db().as_std_path()).unwrap();
    assert!(graph.get_repo("api").unwrap().is_some());
    assert!(!is_legacy_graph_db(layout.memory_db().as_std_path()).unwrap());
}

#[test]
fn a_busy_legacy_database_only_warns_and_the_session_still_opens() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let _held = legacy(&layout);
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let view_dir = layout.ivar_dir().join("sessions/view-split");

    let report = materialise(&layout, &manifest, None, Provider::ClaudeCode, &view_dir).unwrap();

    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.code == "memory.split_deferred")
    );
    assert!(view_dir.as_std_path().is_dir());
    assert!(!layout.graph_db().as_std_path().exists());
}
