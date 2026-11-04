use super::*;

#[test]
fn indexed_content_is_searchable_replaced_and_deleted_with_its_file() {
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("app", "/tmp/app", "main", None).unwrap();
    let empty = crate::store::graph::extractor::ExtractedFile::default();
    db.index_extracted_file(
        "app",
        ".github/workflows/ci.yml",
        "h1",
        1,
        50,
        "name: release sentinel-old",
        false,
        &empty,
    )
    .unwrap();
    assert_eq!(
        db.search_file_content("sentinel-old", Some("app"), 10)
            .unwrap()
            .len(),
        1
    );

    db.index_extracted_file(
        "app",
        ".github/workflows/ci.yml",
        "h2",
        2,
        50,
        "name: release sentinel-new",
        true,
        &empty,
    )
    .unwrap();
    assert!(
        db.search_file_content("sentinel-old", Some("app"), 10)
            .unwrap()
            .is_empty()
    );
    let hits = db
        .search_file_content("sentinel-new", Some("app"), 10)
        .unwrap();
    assert_eq!(hits[0].path, ".github/workflows/ci.yml");
    assert!(hits[0].content_truncated);

    db.delete_file_cascade("app", ".github/workflows/ci.yml")
        .unwrap();
    assert!(
        db.search_file_content("sentinel-new", Some("app"), 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn indexed_content_search_handles_punctuation_and_quotes() {
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("app", "/tmp/app", "main", None).unwrap();
    let empty = crate::store::graph::extractor::ExtractedFile::default();
    db.index_extracted_file(
        "app",
        "config/settings.json",
        "h1",
        1,
        100,
        r#"{"key-name": "value:with:colons", "quoted": "hello \"world\""}"#,
        false,
        &empty,
    )
    .unwrap();

    assert_eq!(
        db.search_file_content("key-name", Some("app"), 10)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.search_file_content("value:with:colons", Some("app"), 10)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.search_file_content(r#"hello "world""#, Some("app"), 10)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.search_file_content("   ", Some("app"), 10)
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        db.search_file_content("", Some("app"), 10).unwrap().len(),
        0
    );
}

#[test]
fn list_base_repos_skips_layer_repos_and_orders_by_id() {
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("web", "/web", "main", None).unwrap();
    db.insert_repo("api", "/api", "main", None).unwrap();
    db.insert_repo("api/7", "/f/api", "feat", None).unwrap();

    let ids: Vec<String> = db
        .list_base_repos()
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();

    assert_eq!(ids, vec!["api", "web"]);
}

#[test]
fn list_layer_records_returns_every_layer_in_feature_order() {
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("api", "/api", "main", None).unwrap();
    db.ensure_layer_record("zeta", "api", "/z/api", "c1")
        .unwrap();
    db.ensure_layer_record("alpha", "api", "/a/api", "c1")
        .unwrap();

    let features: Vec<String> = db
        .list_layer_records()
        .unwrap()
        .into_iter()
        .map(|l| l.feature)
        .collect();

    assert_eq!(features, vec!["alpha", "zeta"]);
}

#[test]
fn delete_files_under_removes_only_the_prefix_and_treats_like_wildcards_literally() {
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("hall", "/hall", "", None).unwrap();
    db.upsert_file("hall", "brand/DESIGN.md", "h1", 1, 1)
        .unwrap();
    db.upsert_file("hall", "brand/moodboard/index.html", "h2", 1, 1)
        .unwrap();
    db.upsert_file("hall", "brandy/x.md", "h3", 1, 1).unwrap();
    db.upsert_file("hall", "docs/a.md", "h4", 1, 1).unwrap();
    db.upsert_file("hall", "br_nd/y.md", "h5", 1, 1).unwrap();

    let removed = db.delete_files_under("hall", "brand").unwrap();

    assert_eq!(removed, 2);
    let left: Vec<String> = db
        .get_files_for_repo("hall")
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(left, vec!["br_nd/y.md", "brandy/x.md", "docs/a.md"]);
    assert_eq!(db.delete_files_under("hall", "br_nd/").unwrap(), 1);
}

#[test]
fn drop_feature_layers_propagates_a_failing_drop() {
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("api", "/api", "main", None).unwrap();
    let id = db
        .ensure_layer_record("feat", "api", "/f/api", "c1")
        .unwrap();
    db.insert_repo(&format!("api/{id}"), "/f/api", "feat", None)
        .unwrap();
    db.conn()
        .execute_batch("DROP TABLE layer_tombstones;")
        .unwrap();

    let error = db.drop_feature_layers("feat").unwrap_err();

    assert!(
        error.to_string().contains("layer_tombstones"),
        "was: {error}"
    );
}

#[test]
fn forget_repo_commits_in_batches_so_the_write_lock_is_short() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("big", "/big", "main", None).unwrap();
    for i in 0..(2 * FORGET_BATCH + 1) {
        db.upsert_file("big", &format!("src/f{i}.rs"), "h", 1, 1)
            .unwrap();
    }
    static COMMITS: AtomicUsize = AtomicUsize::new(0);
    COMMITS.store(0, Ordering::SeqCst);
    db.conn().commit_hook(Some(|| {
        COMMITS.fetch_add(1, Ordering::SeqCst);
        false
    }));

    db.forget_repo("big").unwrap();

    db.conn().commit_hook(None::<fn() -> bool>);
    assert!(db.get_repo("big").unwrap().is_none());
    assert!(db.get_files_for_repo("big").unwrap().is_empty());
    assert!(
        COMMITS.load(Ordering::SeqCst) >= 3,
        "commits: {}",
        COMMITS.load(Ordering::SeqCst)
    );
}

#[test]
fn repo_counts_reports_files_symbols_and_edges_per_repo() {
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("ivar", "/ivar", "main", None).unwrap();
    db.insert_repo("other", "/other", "main", None).unwrap();
    let file_id = db.upsert_file("ivar", "src/lib.rs", "h", 1, 1).unwrap();
    db.upsert_file("other", "a.rs", "h", 1, 1).unwrap();
    let sym_id = db
        .insert_symbols(&[Symbol {
            id: None,
            file_id: Some(file_id),
            repo: "ivar".to_owned(),
            name: "init".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 2, 1),
            is_exported: true,
            complexity: None,
        }])
        .unwrap()[0];
    db.insert_edges(&[Edge {
        id: None,
        repo: "ivar".to_owned(),
        file_id: Some(file_id),
        from_symbol_id: Some(sym_id),
        to_symbol_id: None,
        to_name: Some("helper".to_owned()),
        kind: EdgeKind::Calls,
        provenance: Provenance::Extracted,
        line: 1,
        col: 1,
        confidence: 1.0,
    }])
    .unwrap();

    assert_eq!(
        db.repo_counts("ivar").unwrap(),
        RepoCounts {
            files: 1,
            symbols: 1,
            edges: 1
        }
    );
    assert_eq!(
        db.repo_counts("other").unwrap(),
        RepoCounts {
            files: 1,
            symbols: 0,
            edges: 0
        }
    );
    assert_eq!(db.repo_counts("missing").unwrap(), RepoCounts::default());
}

#[test]
fn under_dir_like_escapes_like_wildcards() {
    assert_eq!(under_dir_like("a_b%c"), "a\\_b\\%c/%");
    assert_eq!(under_dir_like("a\\b"), "a\\\\b/%");
}
