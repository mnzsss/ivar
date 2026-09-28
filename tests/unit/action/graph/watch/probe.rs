#![allow(clippy::unwrap_used)]

use camino::Utf8PathBuf;

use crate::action::graph::watch::lease::Lease;
use crate::action::graph::watch::{LeaderState, Watch};
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;

fn no_targets() -> crate::action::graph::watch::worker::Discover {
    Box::new(|_, _| Vec::new())
}

#[test]
fn the_first_prober_leads_and_marks_every_known_scope_for_catchup() {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    std::fs::create_dir_all(root.join(".ivar")).unwrap();
    let layout = Layout::at(root);
    let db = GraphDb::open(layout.ivar_dir().join("memory.db").as_std_path()).unwrap();
    db.watch_register("base:api").unwrap();
    db.watch_finish("base:api", 0, true).unwrap();

    let mut watch = Watch::with_discover(layout.clone(), no_targets);
    assert_eq!(watch.probe(&db), LeaderState::Us);
    assert!(
        !db.watch_settled(&["base:api"]).unwrap(),
        "a stale scope from a dead leader is distrusted"
    );
    assert_eq!(
        watch.probe(&db),
        LeaderState::Us,
        "still leader, no re-election"
    );
}

#[test]
fn a_prober_follows_while_another_holder_keeps_the_lease_and_takes_over_after() {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    std::fs::create_dir_all(root.join(".ivar")).unwrap();
    let layout = Layout::at(root);
    let db = GraphDb::open(layout.ivar_dir().join("memory.db").as_std_path()).unwrap();

    let other = Lease::try_acquire(&layout).unwrap().unwrap();
    let mut watch = Watch::with_discover(layout.clone(), no_targets);
    assert_eq!(watch.probe(&db), LeaderState::Other);
    drop(other);
    assert_eq!(watch.probe(&db), LeaderState::Us);
}
