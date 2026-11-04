use super::*;

#[test]
fn test_open_in_memory_and_stats() {
    let db = GraphDb::open_in_memory().expect("failed to open in memory db");
    let stats = db.stats().expect("failed to get stats");
    assert_eq!(stats.repo_count, 0);
    assert_eq!(stats.file_count, 0);
    assert_eq!(stats.symbol_count, 0);
    assert_eq!(stats.edge_count, 0);
}

#[test]
fn test_open_on_disk() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("sub").join("graph.db");
    let db = GraphDb::open(&db_path).expect("failed to open on-disk db");
    let stats = db.stats().expect("stats");
    assert_eq!(stats.repo_count, 0);

    // Verify foreign_keys enabled
    let fk: i64 = db
        .conn()
        .query_row("PRAGMA foreign_keys;", [], |r| r.get(0))
        .expect("pragma fk");
    assert_eq!(fk, 1);
}

#[test]
fn test_open_read_only_does_not_create_file_or_run_migrations() {
    let temp = tempdir().expect("tempdir");
    let non_existent = temp.path().join("does_not_exist.db");

    // Opening non-existent database in read-only mode fails and does NOT create parent dir or file
    let res = GraphDb::open_read_only(&non_existent);
    assert!(
        res.is_err(),
        "open_read_only on non-existent file must fail"
    );
    assert!(
        !non_existent.exists(),
        "open_read_only must not create file"
    );

    // Opening existing database read-only succeeds and permits queries
    let valid_db_path = temp.path().join("valid.db");
    {
        let db = GraphDb::open(&valid_db_path).expect("open to create");
        let stats = db.stats().expect("stats");
        assert_eq!(stats.symbol_count, 0);
    }

    let ro_db = GraphDb::open_read_only(&valid_db_path).expect("open read only");
    let ro_stats = ro_db.stats().expect("stats from ro db");
    assert_eq!(ro_stats.symbol_count, 0);
}

#[test]
fn test_repo_and_file_crud() {
    let db = GraphDb::open_in_memory().expect("open");
    db.insert_repo("ivar", "/path/to/ivar", "main", Some("abcdef012345"))
        .expect("insert repo");

    let repo = db.get_repo("ivar").expect("get repo").expect("exists");
    assert_eq!(repo.id, "ivar");
    assert_eq!(repo.root_path, "/path/to/ivar");
    assert_eq!(repo.default_branch, "main");
    assert_eq!(repo.last_indexed_commit.as_deref(), Some("abcdef012345"));

    db.update_repo_commit("ivar", "112233445566")
        .expect("update repo commit");
    let repo_updated = db.get_repo("ivar").expect("get repo").expect("exists");
    assert_eq!(
        repo_updated.last_indexed_commit.as_deref(),
        Some("112233445566")
    );

    let file_id = db
        .upsert_file("ivar", "src/main.rs", "hash_abc", 1000, 2048)
        .expect("upsert file");
    assert!(file_id > 0);

    let file = db
        .get_file("ivar", "src/main.rs")
        .expect("get file")
        .expect("exists");
    assert_eq!(file.id, file_id);
    assert_eq!(file.repo, "ivar");
    assert_eq!(file.path, "src/main.rs");
    assert_eq!(file.content_hash, "hash_abc");

    // Re-upsert file updates existing row
    let file_id_2 = db
        .upsert_file("ivar", "src/main.rs", "hash_def", 2000, 4096)
        .expect("upsert updated file");
    assert_eq!(file_id, file_id_2);

    let file2 = db
        .get_file("ivar", "src/main.rs")
        .expect("get file")
        .expect("exists");
    assert_eq!(file2.content_hash, "hash_def");
    assert_eq!(file2.size_bytes, 4096);
}

#[test]
fn test_foreign_key_cascades() {
    let db = GraphDb::open_in_memory().expect("open");
    db.insert_repo("ivar", "/path", "main", None).unwrap();
    let file_id = db
        .upsert_file("ivar", "src/lib.rs", "hash1", 1, 100)
        .unwrap();

    let sym = Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "ivar".to_owned(),
        name: "init".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("pub fn init()".to_owned()),
        docstring: Some("Initializes the system.".to_owned()),
        span: Span::new(1, 1, 10, 1),
        is_exported: true,
        complexity: Some(1),
    };
    let sym_ids = db.insert_symbols(&[sym]).unwrap();
    let sym_id = sym_ids[0];

    let edge = Edge {
        id: None,
        repo: "ivar".to_owned(),
        file_id: Some(file_id),
        from_symbol_id: Some(sym_id),
        to_symbol_id: None,
        to_name: Some("sub_init".to_owned()),
        kind: EdgeKind::Calls,
        provenance: Provenance::Extracted,
        line: 5,
        col: 9,
        confidence: 1.0,
    };
    db.insert_edges(&[edge]).unwrap();

    let stats = db.stats().unwrap();
    assert_eq!(stats.file_count, 1);
    assert_eq!(stats.symbol_count, 1);
    assert_eq!(stats.edge_count, 1);

    // Deleting file should cascade delete symbol and edge
    db.delete_file("ivar", "src/lib.rs").unwrap();

    let stats_after = db.stats().unwrap();
    assert_eq!(stats_after.file_count, 0);
    assert_eq!(stats_after.symbol_count, 0);
    assert_eq!(stats_after.edge_count, 0);
}

#[test]
fn test_delete_repo_and_clean_all() {
    let db = GraphDb::open_in_memory().expect("open");
    db.insert_repo("repo1", "/path1", "main", None).unwrap();
    db.insert_repo("repo2", "/path2", "main", None).unwrap();

    let f1 = db.upsert_file("repo1", "src/a.rs", "h1", 1, 10).unwrap();
    let f2 = db.upsert_file("repo2", "src/b.rs", "h2", 2, 20).unwrap();

    let s1 = db
        .insert_symbols(&[Symbol {
            id: None,
            file_id: Some(f1),
            repo: "repo1".into(),
            name: "sym1".into(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 2, 1),
            is_exported: true,
            complexity: None,
        }])
        .unwrap()[0];

    let s2 = db
        .insert_symbols(&[Symbol {
            id: None,
            file_id: Some(f2),
            repo: "repo2".into(),
            name: "sym2".into(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 2, 1),
            is_exported: true,
            complexity: None,
        }])
        .unwrap()[0];

    db.insert_edges(&[
        Edge {
            id: None,
            repo: "repo1".into(),
            file_id: Some(f1),
            from_symbol_id: Some(s1),
            to_symbol_id: None,
            to_name: Some("sym2".into()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 1,
            col: 1,
            confidence: 1.0,
        },
        Edge {
            id: None,
            repo: "repo2".into(),
            file_id: Some(f2),
            from_symbol_id: Some(s2),
            to_symbol_id: None,
            to_name: Some("external".into()),
            kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 1,
            col: 1,
            confidence: 1.0,
        },
    ])
    .unwrap();

    let stats = db.stats().unwrap();
    assert_eq!(stats.repo_count, 2);
    assert_eq!(stats.file_count, 2);
    assert_eq!(stats.symbol_count, 2);
    assert_eq!(stats.edge_count, 2);

    // Delete repo1
    let clean_res = db.delete_repo("repo1").unwrap();
    assert_eq!(
        clean_res,
        Some(RepoCleanStats {
            repo: "repo1".into(),
            files_removed: 1,
            symbols_removed: 1,
            edges_removed: 1,
        })
    );

    // Verify repo1 is gone, repo2 remains
    assert!(db.get_repo("repo1").unwrap().is_none());
    assert!(db.get_repo("repo2").unwrap().is_some());
    let stats_after = db.stats().unwrap();
    assert_eq!(stats_after.repo_count, 1);
    assert_eq!(stats_after.file_count, 1);
    assert_eq!(stats_after.symbol_count, 1);
    assert_eq!(stats_after.edge_count, 1);

    // Deleting repo1 again returns None
    assert_eq!(db.delete_repo("repo1").unwrap(), None);

    // Clean all
    let all_res = db.clean_all().unwrap();
    assert_eq!(
        all_res,
        CleanAllStats {
            repos_removed: 1,
            files_removed: 1,
            symbols_removed: 1,
            edges_removed: 1,
            usage_removed: 0,
        }
    );

    let final_stats = db.stats().unwrap();
    assert_eq!(final_stats.repo_count, 0);
    assert_eq!(final_stats.file_count, 0);
    assert_eq!(final_stats.symbol_count, 0);
    assert_eq!(final_stats.edge_count, 0);
}

#[test]
fn test_delete_symbols_for_file() {
    let db = GraphDb::open_in_memory().expect("open");
    db.insert_repo("ivar", "/path", "main", None).unwrap();
    let file_id = db
        .upsert_file("ivar", "src/lib.rs", "hash1", 1, 100)
        .unwrap();

    let sym = Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "ivar".to_owned(),
        name: "test_fn".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: None,
        docstring: None,
        span: Span::new(1, 1, 5, 1),
        is_exported: false,
        complexity: None,
    };
    let sym_ids = db.insert_symbols(&[sym]).unwrap();
    let sym_id = sym_ids[0];

    let edge = Edge {
        id: None,
        repo: "ivar".to_owned(),
        file_id: Some(file_id),
        from_symbol_id: Some(sym_id),
        to_symbol_id: None,
        to_name: Some("callee".to_owned()),
        kind: EdgeKind::Calls,
        provenance: Provenance::Extracted,
        line: 2,
        col: 5,
        confidence: 1.0,
    };
    db.insert_edges(&[edge]).unwrap();

    db.delete_symbols_for_file(file_id).unwrap();

    let stats = db.stats().unwrap();
    assert_eq!(stats.file_count, 1);
    assert_eq!(stats.symbol_count, 0);
    assert_eq!(stats.edge_count, 0);
}
