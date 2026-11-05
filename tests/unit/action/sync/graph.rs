use super::*;

// -- the codebase graph ----------------------------------------------------

#[test]
fn sync_leaves_a_hall_without_a_graph_database_without_one() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert!(!root.join(".ivar/graph.db").as_std_path().exists());
}

#[test]
fn sync_reindexes_base_repos_when_the_hall_already_has_a_graph() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();
    let db_path = root.join(".ivar/graph.db");
    drop(crate::store::graph::db::GraphDb::open(db_path.as_std_path()).unwrap());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean(), "{:?}", report.warnings);
    let db = crate::store::graph::db::GraphDb::open(db_path.as_std_path()).unwrap();
    assert!(db.get_repo_last_commit("api").unwrap().is_some());
}

#[test]
fn a_graph_that_cannot_be_opened_warns_without_failing_sync() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();
    std::fs::write(
        root.join(".ivar/graph.db"),
        b"not a sqlite database at all, just junk bytes",
    )
    .unwrap();

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.code == "sync.graph_index_failed"),
        "{:?}",
        report.warnings
    );
}

#[test]
fn sync_indexes_pages_without_a_warning() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let page = root.join("docs/memory/gotcha/a.md");
    std::fs::create_dir_all(page.parent().unwrap()).unwrap();
    std::fs::write(&page, "# A\nsyncword\n").unwrap();

    let report = sync(&Ctx::new(root.clone()), &SyncInput::default()).unwrap();

    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let db = crate::store::memory::db::MemoryDb::open(
        crate::store::layout::Layout::at(root)
            .memory_db()
            .as_std_path(),
    )
    .unwrap();
    let rows: i64 = db
        .conn()
        .query_row("SELECT count(*) FROM pages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);
}
