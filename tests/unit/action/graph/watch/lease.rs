#![allow(clippy::unwrap_used)]

use camino::Utf8PathBuf;
use tempfile::tempdir;

use crate::action::graph::watch::lease::{Lease, leader_pid};
use crate::store::layout::Layout;

fn layout() -> (tempfile::TempDir, Layout) {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    std::fs::create_dir_all(root.join(".ivar")).unwrap();
    (tmp, Layout::at(root))
}

#[test]
fn only_one_holder_at_a_time_and_release_on_drop() {
    let (_tmp, layout) = layout();
    assert_eq!(leader_pid(&layout), None, "no lease file yet");

    let first = Lease::try_acquire(&layout).unwrap().expect("free lease");
    assert!(Lease::try_acquire(&layout).unwrap().is_none(), "held lease");
    assert_eq!(leader_pid(&layout), Some(std::process::id()));

    drop(first);
    assert_eq!(leader_pid(&layout), None, "dropping releases the flock");
    assert!(Lease::try_acquire(&layout).unwrap().is_some());
}

#[test]
fn probing_for_the_leader_never_takes_the_lease() {
    let (_tmp, layout) = layout();
    let _held = Lease::try_acquire(&layout).unwrap().unwrap();
    for _ in 0..3 {
        assert_eq!(leader_pid(&layout), Some(std::process::id()));
    }
}
