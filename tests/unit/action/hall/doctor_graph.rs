use super::*;

#[test]
fn doctor_finds_nothing_in_a_graph_indexed_at_head() {
    let (_guard, root) = hall_with_indexed_graph();

    let report = doctor(&Ctx::new(root)).unwrap();

    assert!(
        report.value.findings.is_empty(),
        "{:?}",
        report.value.findings
    );
}

#[test]
fn doctor_names_a_repo_whose_graph_lags_its_worktree_head() {
    let (_guard, root) = hall_with_indexed_graph();
    let db =
        crate::store::graph::db::GraphDb::open(root.join(".ivar/memory.db").as_std_path()).unwrap();
    db.update_repo_commit("api", "0000000000000000000000000000000000000000")
        .unwrap();
    drop(db);

    let report = doctor(&Ctx::new(root)).unwrap();

    let finding = finding(&report.value, "graph.repo_stale");
    assert!(finding.what.contains("api"), "was: {}", finding.what);
    assert!(finding.fix.contains("ivar graph index"));
}

#[test]
fn doctor_names_hall_directories_the_graph_does_not_index() {
    let (_guard, root) = hall_with_indexed_graph();
    std::fs::create_dir_all(root.join("brand")).unwrap();
    std::fs::write(root.join("brand/GUIDELINE.md"), "# guideline\n").unwrap();
    std::fs::create_dir_all(root.join("docs")).unwrap();

    let report = doctor(&Ctx::new(root)).unwrap();

    let finding = finding(&report.value, "graph.hall_paths_unset");
    assert!(finding.what.contains("`brand`"), "was: {}", finding.what);
    assert!(
        !finding.what.contains("`docs`"),
        "an empty directory is not named: {}",
        finding.what
    );
    assert!(finding.fix.contains("ivar graph index --path brand"));
}

#[test]
fn doctor_is_quiet_about_hall_directories_once_graph_paths_lists_one() {
    let (_guard, root) = hall_with_indexed_graph();
    std::fs::create_dir_all(root.join("brand")).unwrap();
    std::fs::write(root.join("brand/GUIDELINE.md"), "# guideline\n").unwrap();
    let layout = Layout::at(root.clone());
    let manifest = crate::store::manifest::Manifest::read(&layout)
        .unwrap()
        .unwrap()
        .with_graph_path_added("brand")
        .unwrap();
    crate::store::manifest::Manifest::write(&layout, &manifest).unwrap();

    let report = doctor(&Ctx::new(root)).unwrap();

    assert!(
        report
            .value
            .findings
            .iter()
            .all(|f| f.code != "graph.hall_paths_unset"),
        "{:?}",
        report.value.findings
    );
}

#[test]
fn doctor_names_a_graph_schema_other_than_the_expected_one() {
    let (_guard, root) = hall_with_indexed_graph();
    rusqlite::Connection::open(root.join(".ivar/memory.db").as_std_path())
        .unwrap()
        .execute_batch("PRAGMA user_version = 4;")
        .unwrap();

    let report = doctor(&Ctx::new(root)).unwrap();

    let finding = finding(&report.value, "graph.schema_mismatch");
    assert!(finding.what.contains('4'), "was: {}", finding.what);
}

#[test]
fn doctor_names_a_scope_the_watcher_could_not_reindex() {
    let (_guard, root) = hall_with_indexed_graph();
    let db =
        crate::store::graph::db::GraphDb::open(root.join(".ivar/memory.db").as_std_path()).unwrap();
    db.watch_register("base:api").unwrap();
    db.watch_fail("base:api", "index_repo: permission denied")
        .unwrap();
    drop(db);

    let report = doctor(&Ctx::new(root)).unwrap();

    let failed = finding(&report.value, "graph.watch_scope_failed");
    assert!(
        failed.what.contains("base:api") && failed.what.contains("permission denied"),
        "was: {}",
        failed.what
    );
}

#[test]
fn doctor_names_an_indexed_repo_the_manifest_no_longer_declares() {
    let (_guard, root) = hall_with_indexed_graph();
    let db =
        crate::store::graph::db::GraphDb::open(root.join(".ivar/memory.db").as_std_path()).unwrap();
    db.insert_repo("orca", "/x/orca", "main", None).unwrap();
    let file_id = db.upsert_file("orca", "a.rs", "h", 1, 1).unwrap();
    db.insert_symbols(&[crate::domain::graph::Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "orca".into(),
        name: "main".into(),
        kind: crate::domain::graph::SymbolKind::Fn,
        scope: None,
        signature: None,
        docstring: None,
        span: crate::domain::graph::Span::new(1, 1, 2, 1),
        is_exported: true,
        complexity: None,
    }])
    .unwrap();
    drop(db);

    let report = doctor(&Ctx::new(root)).unwrap();

    let f = finding(&report.value, "graph.repo_not_in_manifest");
    assert!(
        f.what.contains("orca") && f.what.contains("1 files") && f.what.contains("1 symbols"),
        "was: {}",
        f.what
    );
    assert!(
        f.fix.contains("ivar graph clean --repo orca"),
        "was: {}",
        f.fix
    );
}

#[test]
fn doctor_says_counts_are_unavailable_when_an_orphan_repo_cannot_be_counted() {
    let (_guard, root) = hall_with_indexed_graph();
    let db =
        crate::store::graph::db::GraphDb::open(root.join(".ivar/memory.db").as_std_path()).unwrap();
    db.insert_repo("orca", "/x/orca", "main", None).unwrap();
    db.conn().execute_batch("DROP TABLE edges;").unwrap();
    drop(db);

    let report = doctor(&Ctx::new(root)).unwrap();

    let f = finding(&report.value, "graph.repo_not_in_manifest");
    assert!(
        f.what.contains("orca") && f.what.contains("counts unavailable"),
        "was: {}",
        f.what
    );
    assert!(!f.what.contains("0 files"), "was: {}", f.what);
}

#[test]
fn doctor_names_a_layer_whose_feature_is_gone_and_its_scope_once() {
    let (_guard, root) = hall_with_indexed_graph();
    let db =
        crate::store::graph::db::GraphDb::open(root.join(".ivar/memory.db").as_std_path()).unwrap();
    let id = db
        .ensure_layer_record("ghost", "api", "/g/api", "c1")
        .unwrap();
    db.insert_repo(&format!("api/{id}"), "/g/api", "ghost", None)
        .unwrap();
    db.watch_register("layer:ghost:api").unwrap();
    db.watch_fail("layer:ghost:api", "worktree missing")
        .unwrap();
    drop(db);

    let report = doctor(&Ctx::new(root)).unwrap();

    let layer = finding(&report.value, "graph.layer_orphaned");
    assert!(
        layer.what.contains("ghost") && layer.fix.contains("ivar graph index"),
        "{layer:?}"
    );
    let scope = finding(&report.value, "graph.watch_scope_orphaned");
    assert!(scope.what.contains("layer:ghost:api"), "{scope:?}");
    assert!(
        !report
            .value
            .findings
            .iter()
            .any(|f| f.code == "graph.watch_scope_failed"),
        "an orphan scope is not also reported as failed: {:?}",
        report.value.findings
    );
}

#[test]
fn doctor_keeps_quiet_about_a_layer_scope_of_a_live_feature_without_a_session() {
    let (_guard, root) = hall_with_indexed_graph();
    let layout = Layout::at(root.clone());
    let mut feature = crate::domain::feature::Feature::new(
        FeatureName::new("quiet").unwrap(),
        crate::domain::name::BranchName::new("quiet").unwrap(),
    );
    feature.promote(crate::domain::name::RepoName::new("api").unwrap());
    feature.write(&layout).unwrap();
    let db =
        crate::store::graph::db::GraphDb::open(root.join(".ivar/memory.db").as_std_path()).unwrap();
    db.watch_register("layer:quiet:api").unwrap();
    db.watch_finish("layer:quiet:api", 0, true).unwrap();
    drop(db);

    let report = doctor(&Ctx::new(root)).unwrap();

    assert!(
        !report
            .value
            .findings
            .iter()
            .any(|f| f.code.starts_with("graph.")),
        "{:?}",
        report.value.findings
    );
}

#[test]
fn doctor_tells_a_failed_live_scope_that_the_watcher_retries() {
    let (_guard, root) = hall_with_indexed_graph();
    let db =
        crate::store::graph::db::GraphDb::open(root.join(".ivar/memory.db").as_std_path()).unwrap();
    db.watch_register("base:api").unwrap();
    db.watch_fail("base:api", "disk full").unwrap();
    drop(db);

    let report = doctor(&Ctx::new(root)).unwrap();

    let f = finding(&report.value, "graph.watch_scope_failed");
    assert!(f.fix.contains("retries"), "was: {}", f.fix);
}

#[test]
fn doctor_names_unfinished_catchup_only_when_no_leader_is_alive() {
    let (_guard, root) = hall_with_indexed_graph();
    let db =
        crate::store::graph::db::GraphDb::open(root.join(".ivar/memory.db").as_std_path()).unwrap();
    db.watch_register("base:api").unwrap(); // needs_catchup = 1
    drop(db);

    let report = doctor(&Ctx::new(root.clone())).unwrap();
    let pending = finding(&report.value, "graph.watch_catchup_pending");
    assert!(
        pending.fix.contains("ivar graph index"),
        "was: {}",
        pending.fix
    );
    assert_eq!(report.value.graph_watch_leader, None);

    let layout = crate::store::layout::Layout::at(root.clone());
    let _lease = crate::action::graph::watch::lease::Lease::try_acquire(&layout)
        .unwrap()
        .unwrap();
    let report = doctor(&Ctx::new(root)).unwrap();
    assert!(
        report
            .value
            .findings
            .iter()
            .all(|f| f.code != "graph.watch_catchup_pending"),
        "a live leader is catching up"
    );
    assert_eq!(report.value.graph_watch_leader, Some(std::process::id()));
}
