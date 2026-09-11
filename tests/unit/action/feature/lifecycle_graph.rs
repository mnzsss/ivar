use tempfile::tempdir;
use crate::store::graph::db::GraphDb;

#[test]
fn test_rename_and_drop_feature_layers_in_graph_db() {
    let hall_dir = tempdir().unwrap();
    let db_path = hall_dir.path().join(".ivar/memory.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let db = GraphDb::open(&db_path).unwrap();
    db.ensure_views_base_mode().unwrap();

    // 1. Seed base repo and feature layers for feat-alpha
    db.insert_repo("core", "/base/core", "main", Some("c1")).unwrap();
    let l1 = db.ensure_layer_record("feat-alpha", "core", "/feat-a/core", "c1").unwrap();
    let layer_repo = format!("core/{l1}");
    db.insert_repo(&layer_repo, "/feat-a/core", "feat-alpha", Some("c1")).unwrap();
    assert!(db.get_layer_record("feat-alpha", "core").unwrap().is_some());

    // 2. Test renaming feature layers
    let renamed = db.rename_feature_layers("feat-alpha", "feat-beta").unwrap();
    assert_eq!(renamed, 1);
    assert!(db.get_layer_record("feat-alpha", "core").unwrap().is_none());
    let rec = db.get_layer_record("feat-beta", "core").unwrap().expect("renamed record exists");
    assert_eq!(rec.feature, "feat-beta");

    // 3. Test dropping feature layers
    let dropped = db.drop_feature_layers("feat-beta").unwrap();
    assert_eq!(dropped, 1);
    assert!(db.get_layer_record("feat-beta", "core").unwrap().is_none());
    // Verify pseudo-repo was deleted
    let repo_exists: bool = db.conn().query_row(
        "SELECT COUNT(*) > 0 FROM repos WHERE id = ?1",
        [&layer_repo],
        |r| r.get(0),
    ).unwrap();
    assert!(!repo_exists);

    // 4. Test lazy GC of stale layers
    let l2 = db.ensure_layer_record("feat-stale", "core", "/stale/core", "c1").unwrap();
    db.insert_repo(&format!("core/{l2}"), "/stale/core", "feat-stale", Some("c1")).unwrap();
    let l3 = db.ensure_layer_record("feat-active", "core", "/active/core", "c1").unwrap();
    db.insert_repo(&format!("core/{l3}"), "/active/core", "feat-active", Some("c1")).unwrap();

    let gc_count = db.gc_stale_layers(&["feat-active"]).unwrap();
    assert_eq!(gc_count, 1);
    assert!(db.get_layer_record("feat-stale", "core").unwrap().is_none());
    assert!(db.get_layer_record("feat-active", "core").unwrap().is_some());
}
