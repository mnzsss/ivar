use camino::Utf8PathBuf;

use crate::action::Ctx;
use crate::action::confirm::fixed;
use crate::action::memory::clean::clean;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;
use crate::test_support::seeded_hall;

fn hall_with_both_databases() -> (tempfile::TempDir, Utf8PathBuf, Layout) {
    let (guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    drop(GraphDb::open(layout.graph_db().as_std_path()).unwrap());
    drop(MemoryDb::open(layout.memory_db().as_std_path()).unwrap());
    (guard, root, layout)
}

#[test]
fn clean_without_a_terminal_deletes_nothing() {
    let (_guard, root, layout) = hall_with_both_databases();

    let failure = clean(&Ctx::new(root)).unwrap_err();

    assert_eq!(failure.code, "memory.clean_not_confirmed");
    assert!(layout.memory_db().as_std_path().exists());
}

#[test]
fn clean_answered_no_deletes_nothing() {
    let (_guard, root, layout) = hall_with_both_databases();

    let failure = clean(&Ctx::new(root).with_confirm(fixed(false))).unwrap_err();

    assert_eq!(failure.code, "memory.clean_not_confirmed");
    assert!(layout.memory_db().as_std_path().exists());
}

#[test]
fn clean_confirmed_removes_the_database_and_its_sidecars_but_never_the_graph() {
    let (_guard, root, layout) = hall_with_both_databases();
    let wal = Utf8PathBuf::from(format!("{}-wal", layout.memory_db()));
    std::fs::write(wal.as_std_path(), b"").unwrap();

    let report = clean(&Ctx::new(root).with_confirm(fixed(true))).unwrap();

    assert!(!layout.memory_db().as_std_path().exists());
    assert!(!wal.as_std_path().exists());
    assert!(report.value.removed.contains(&layout.memory_db()));
    assert!(report.value.removed.contains(&wal));
    assert!(layout.graph_db().as_std_path().exists());
}

#[test]
fn clean_on_a_hall_without_a_memory_database_creates_and_deletes_nothing() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());

    let report = clean(&Ctx::new(root).with_confirm(fixed(true))).unwrap();

    assert!(
        report.value.removed.is_empty(),
        "{:?}",
        report.value.removed
    );
    assert!(!layout.memory_db().as_std_path().exists());
}

#[test]
fn clean_on_a_pre_split_hall_moves_the_graph_and_deletes_nothing() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    drop(GraphDb::open(layout.memory_db().as_std_path()).unwrap());

    let report = clean(&Ctx::new(root).with_confirm(fixed(true))).unwrap();

    assert!(
        report.value.removed.is_empty(),
        "{:?}",
        report.value.removed
    );
    assert!(!layout.memory_db().as_std_path().exists());
    assert!(GraphDb::open_read_only(layout.graph_db().as_std_path()).is_ok());
}
