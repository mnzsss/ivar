use crate::action::Ctx;
use crate::action::memory::status::status;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;
use crate::store::memory::schema::{APPLICATION_ID, MEMORY_SCHEMA_VERSION};
use crate::test_support::seeded_hall;

#[test]
fn status_repairs_a_memory_database_an_older_ivar_polluted() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    status(&Ctx::new(root.clone())).unwrap();
    drop(GraphDb::open(layout.memory_db().as_std_path()).unwrap());
    assert!(
        MemoryDb::inspect(layout.memory_db().as_std_path())
            .unwrap()
            .polluted
    );

    status(&Ctx::new(root)).unwrap();

    let inspection = MemoryDb::inspect(layout.memory_db().as_std_path()).unwrap();
    assert!(!inspection.polluted);
    assert_eq!(inspection.schema_version, MEMORY_SCHEMA_VERSION);
}

#[test]
fn status_moves_a_legacy_graph_and_reports_the_new_memory_database() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    drop(GraphDb::open(layout.memory_db().as_std_path()).unwrap());

    let report = status(&Ctx::new(root)).unwrap();

    let value = report.value;
    assert_eq!(value.path, layout.memory_db());
    assert!(value.exists);
    assert_eq!(value.schema_version, Some(MEMORY_SCHEMA_VERSION));
    assert_eq!(value.application_id, Some(APPLICATION_ID));
    assert!(value.size_bytes.unwrap() > 0);
    assert!(layout.graph_db().as_std_path().exists());
}

#[test]
fn status_json_carries_every_field() {
    let (_guard, root) = seeded_hall();

    let report = status(&Ctx::new(root)).unwrap();

    let json = serde_json::to_value(&report.value).unwrap();
    for field in [
        "path",
        "exists",
        "schema_version",
        "application_id",
        "size_bytes",
    ] {
        assert!(json.get(field).is_some(), "{field} missing from {json}");
    }
}

#[test]
fn status_waits_for_the_memory_lock_before_repairing() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    drop(GraphDb::open(layout.graph_db().as_std_path()).unwrap());
    status(&Ctx::new(root.clone())).unwrap();
    drop(GraphDb::open(layout.memory_db().as_std_path()).unwrap());
    let held = crate::infra::fs::lock_exclusive(&layout.memory_lock()).unwrap();
    let worker = std::thread::spawn(move || status(&Ctx::new(root)).is_ok());

    std::thread::sleep(std::time::Duration::from_secs(2));
    assert!(!worker.is_finished(), "status must wait for memory.lock");
    assert!(
        MemoryDb::inspect(layout.memory_db().as_std_path())
            .unwrap()
            .polluted,
        "status must not repair memory.db while memory.lock is held"
    );
    drop(held);

    assert!(worker.join().unwrap());
    assert!(
        !MemoryDb::inspect(layout.memory_db().as_std_path())
            .unwrap()
            .polluted
    );
}
