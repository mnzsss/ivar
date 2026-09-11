use tempfile::tempdir;
use camino::Utf8PathBuf;
use crate::action::Ctx;
use crate::action::graph::clean_cmd;
use crate::action::graph::input::CleanInput;
use crate::action::graph::stats_cmd;
use crate::store::graph::db::GraphDb;

#[test]
fn test_clean_feature_and_stats_reporting() {
    let hall_dir = tempdir().unwrap();
    std::fs::write(hall_dir.path().join("ivar.json"), "{}").unwrap();
    let root_path = Utf8PathBuf::from_path_buf(hall_dir.path().to_path_buf()).unwrap();
    let db_path = hall_dir.path().join(".ivar/memory.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let db = GraphDb::open(&db_path).unwrap();
    db.ensure_views_base_mode().unwrap();

    // 1. Seed base repo and two feature layers
    db.insert_repo("core", "/base/core", "main", Some("c1")).unwrap();
    let l1 = db.ensure_layer_record("feat-1", "core", "/feat1/core", "c1").unwrap();
    let r1 = format!("core/{l1}");
    db.insert_repo(&r1, "/feat1/core", "feat-1", Some("c1")).unwrap();
    db.upsert_file(&r1, "src/a.rs", "h1", 10, 10).unwrap();

    let l2 = db.ensure_layer_record("feat-2", "core", "/feat2/core", "c1").unwrap();
    let r2 = format!("core/{l2}");
    db.insert_repo(&r2, "/feat2/core", "feat-2", Some("c1")).unwrap();
    db.upsert_file(&r2, "src/b.rs", "h2", 10, 10).unwrap();
    db.upsert_file(&r2, "src/c.rs", "h3", 10, 10).unwrap();

    let ctx = Ctx::new(root_path);

    // 2. Test stats reporting includes layers
    let stats_report = stats_cmd(&ctx).expect("stats report");
    let stats = stats_report.value.0;
    assert_eq!(stats.layers.len(), 2);
    let feat1_layer = stats.layers.iter().find(|l| l.feature == "feat-1").expect("feat-1 in stats");
    assert_eq!(feat1_layer.repo, "core");
    assert_eq!(feat1_layer.file_count, 1);
    assert_eq!(feat1_layer.base_commit, "c1");

    let feat2_layer = stats.layers.iter().find(|l| l.feature == "feat-2").expect("feat-2 in stats");
    assert_eq!(feat2_layer.file_count, 2);

    // 3. Clean only feat-1 via clean_cmd
    let clean_report = clean_cmd(&ctx, CleanInput {
        repo: None,
        feature: Some("feat-1".to_owned()),
        all: false,
    }).expect("clean feat-1");

    assert_eq!(clean_report.value.feature.as_deref(), Some("feat-1"));

    // 4. Verify stats after clean
    let stats_after = stats_cmd(&ctx).expect("stats report after clean").value.0;
    assert_eq!(stats_after.layers.len(), 1);
    assert_eq!(stats_after.layers[0].feature, "feat-2");
}
