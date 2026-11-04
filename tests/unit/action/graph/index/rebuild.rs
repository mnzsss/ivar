use super::*;

#[test]
fn test_gitignore_and_ignored_directories() {
    let temp = tempdir().expect("tempdir");
    let repo_path = temp.path();
    let git_repo = git2::Repository::init(repo_path).expect("git init");

    // Write .gitignore
    fs::write(
        repo_path.join(".gitignore"),
        "secret.rs\nignored_folder/\n*.gen.ts\n",
    )
    .expect("write .gitignore");

    // Valid tracked file
    fs::write(repo_path.join("tracked.rs"), "pub fn tracked_fn() {}\n").expect("write tracked.rs");

    // Gitignored files
    fs::write(repo_path.join("secret.rs"), "pub fn secret_fn() {}\n").expect("write secret.rs");

    fs::create_dir(repo_path.join("ignored_folder")).expect("create ignored_folder");
    fs::write(
        repo_path.join("ignored_folder").join("file.rs"),
        "pub fn hidden_fn() {}\n",
    )
    .expect("write ignored_folder/file.rs");

    fs::write(
        repo_path.join("auto.gen.ts"),
        "export function genFn() {}\n",
    )
    .expect("write auto.gen.ts");

    // Unignored build/package directory (e.g. node_modules, dist) even if not in .gitignore
    fs::create_dir(repo_path.join("node_modules")).expect("create node_modules");
    fs::write(
        repo_path.join("node_modules").join("dep.js"),
        "function depFn() {}\n",
    )
    .expect("write node_modules/dep.js");

    fs::create_dir(repo_path.join("dist")).expect("create dist");
    fs::write(
        repo_path.join("dist").join("bundle.js"),
        "function bundleFn() {}\n",
    )
    .expect("write dist/bundle.js");

    create_git_commit(&git_repo, "Initial commit").expect("commit");

    let db = GraphDb::open_in_memory().expect("open db");

    let outcome = index_repo(&db, "test-repo", repo_path, false, &Silent).expect("index");
    // .gitignore and tracked.rs are indexed
    assert_eq!(outcome.files_indexed, 2);
    assert_eq!(outcome.files_deleted, 0);

    let stats = db.stats().expect("stats");
    assert_eq!(stats.file_count, 2);

    let found_tracked = db.search_symbols_fts("tracked_fn", 10).expect("search");
    assert_eq!(found_tracked.len(), 1);

    let found_secret = db.search_symbols_fts("secret_fn", 10).expect("search");
    assert_eq!(found_secret.len(), 0);

    let found_hidden = db.search_symbols_fts("hidden_fn", 10).expect("search");
    assert_eq!(found_hidden.len(), 0);

    let found_dep = db.search_symbols_fts("depFn", 10).expect("search");
    assert_eq!(found_dep.len(), 0);
}

#[test]
fn test_index_lock_contention_and_release() {
    let temp = tempdir().expect("tempdir");
    let lock_path = temp.path().join("memory.lock");

    let guard1 = fs::File::create(&lock_path).expect("create lock1");
    guard1.lock().expect("lock1 acquire");

    let guard2 = fs::File::create(&lock_path).expect("create lock2");
    let second_try = guard2.try_lock();
    assert!(
        second_try.is_err(),
        "second lock acquisition must fail while guard1 is held"
    );

    guard1.unlock().expect("unlock guard1");
    drop(guard1);

    let second_acquire = guard2.try_lock();
    assert!(
        second_acquire.is_ok(),
        "second lock acquisition must succeed after guard1 is dropped"
    );
}

#[test]
fn test_force_full_index_rebuilds_unchanged_repo() {
    let temp = tempdir().expect("tempdir");
    let repo_path = temp.path();
    let git_repo = git2::Repository::init(repo_path).expect("git init");

    fs::write(repo_path.join("file1.rs"), "pub fn original_symbol() {}\n").expect("write file1.rs");
    fs::write(repo_path.join("file2.rs"), "pub fn second_symbol() {}\n").expect("write file2.rs");

    create_git_commit(&git_repo, "Initial commit").expect("commit");

    let db = GraphDb::open_in_memory().expect("open db");

    // 1. Initial index (full)
    let outcome1 = index_repo(&db, "test-repo", repo_path, false, &Silent).expect("first index");
    assert_eq!(outcome1.files_indexed, 2);
    assert!(!outcome1.skipped_up_to_date);

    let stats1 = db.stats().expect("stats1");
    assert_eq!(stats1.file_count, 2);
    assert_eq!(stats1.symbol_count, 2);

    // 2. Second index without force_full -> skipped_up_to_date
    let outcome2 = index_repo(&db, "test-repo", repo_path, false, &Silent).expect("second index");
    assert!(outcome2.skipped_up_to_date);
    assert_eq!(outcome2.files_indexed, 0);

    // 3. Mutate DB state directly (delete records) without touching Git or working directory
    db.delete_repo_files("test-repo")
        .expect("delete repo files");
    let stats_cleared = db.stats().expect("stats cleared");
    assert_eq!(stats_cleared.file_count, 0);
    assert_eq!(stats_cleared.symbol_count, 0);

    // Running normal incremental index would still think HEAD is unchanged and skip
    let outcome_skipped =
        index_repo(&db, "test-repo", repo_path, false, &Silent).expect("skip index");
    assert!(outcome_skipped.skipped_up_to_date);
    assert_eq!(outcome_skipped.files_indexed, 0);

    // 4. Force full index with unchanged HEAD -> must genuinely reparse and rebuild files & symbols
    let outcome_forced =
        index_repo(&db, "test-repo", repo_path, true, &Silent).expect("force full index");
    assert!(!outcome_forced.skipped_up_to_date);
    assert_eq!(outcome_forced.files_indexed, 2);
    assert_eq!(outcome_forced.files_deleted, 0);
    assert_eq!(outcome_forced.symbols_indexed, 2);

    let stats_restored = db.stats().expect("stats restored");
    assert_eq!(stats_restored.file_count, 2);
    assert_eq!(stats_restored.symbol_count, 2);

    let sym1 = db
        .search_symbols_fts("original_symbol", 10)
        .expect("search sym1");
    assert_eq!(sym1.len(), 1);
    let sym2 = db
        .search_symbols_fts("second_symbol", 10)
        .expect("search sym2");
    assert_eq!(sym2.len(), 1);
}

#[test]
fn test_index_repo_progress_reporting_and_clearing() {
    let temp = tempdir().expect("tempdir");
    let repo_path = temp.path();
    let git_repo = git2::Repository::init(repo_path).expect("git init");

    let file1 = repo_path.join("a.rs");
    let file2 = repo_path.join("b.ts");
    fs::write(&file1, "pub fn a() {}\n").expect("write a.rs");
    fs::write(&file2, "export function b() {}\n").expect("write b.ts");

    create_git_commit(&git_repo, "Initial commit").expect("commit");

    let db = GraphDb::open_in_memory().expect("open db");
    let recording = Recording::default();

    // 1. Initial index reports progress for all files and clears when done
    let outcome = index_repo(&db, "test-repo", repo_path, false, &recording).expect("index");
    assert_eq!(outcome.files_indexed, 2);
    assert_eq!(recording.clears(), 1);
    let steps = recording.steps();
    assert_eq!(steps.len(), 2);
    assert!(
        steps
            .iter()
            .any(|s| s.contains("[1/2] test-repo:") || s.contains("[2/2] test-repo:"))
    );
    assert!(steps.iter().any(|s| s.contains("a.rs")));
    assert!(steps.iter().any(|s| s.contains("b.ts")));

    // 2. Up-to-date no-op does not emit any progress steps
    let recording_noop = Recording::default();
    let outcome_noop =
        index_repo(&db, "test-repo", repo_path, false, &recording_noop).expect("noop index");
    assert!(outcome_noop.skipped_up_to_date);
    assert_eq!(recording_noop.steps().len(), 0);
    assert_eq!(recording_noop.clears(), 0);
}

#[test]
fn test_index_repo_progress_clears_when_a_file_fails() {
    let temp = tempdir().expect("tempdir");
    let repo_path = temp.path();
    let git_repo = git2::Repository::init(repo_path).expect("git init");

    let file = repo_path.join("valid.rs");
    fs::write(&file, "pub fn valid() {}\n").expect("write valid.rs");
    create_git_commit(&git_repo, "Initial commit").expect("commit");

    let db = GraphDb::open_in_memory().expect("open db");

    // Make the file unreadable to trigger an Io error during extraction
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&file).expect("metadata").permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&file, perms).expect("set perms");

        let recording = Recording::default();
        let result = index_repo(&db, "test-repo", repo_path, true, &recording);
        assert_eq!(result.expect("index").files_failed.len(), 1);
        assert_eq!(recording.clears(), 1);
        assert_eq!(recording.steps().len(), 1);

        // Restore permissions for cleanup
        let mut perms = fs::metadata(&file).expect("metadata").permissions();
        perms.set_mode(0o644);
        let _ = fs::set_permissions(&file, perms);
    }
}

#[test]
fn test_force_full_index_file_failure_preserves_existing_db_records() {
    let temp = tempdir().expect("tempdir");
    let repo_path = temp.path();
    let git_repo = git2::Repository::init(repo_path).expect("git init");

    let file = repo_path.join("valid.rs");
    fs::write(&file, "pub fn valid() {}\n").expect("write valid.rs");
    create_git_commit(&git_repo, "Initial commit").expect("commit");

    let db = GraphDb::open_in_memory().expect("open db");

    // 1. Initial index succeeds
    let outcome1 = index_repo(&db, "test-repo", repo_path, false, &Silent).expect("initial index");
    assert_eq!(outcome1.files_indexed, 1);
    let stats1 = db.stats().expect("stats1");
    assert_eq!(stats1.file_count, 1);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&file).expect("metadata").permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&file, perms).expect("set perms");

        // 2. Force full index fails due to unreadable file
        let result = index_repo(&db, "test-repo", repo_path, true, &Silent);
        assert_eq!(result.expect("index").files_failed.len(), 1);

        // Restore permissions
        let mut perms = fs::metadata(&file).expect("metadata").permissions();
        perms.set_mode(0o644);
        let _ = fs::set_permissions(&file, perms);

        // 3. Existing database records must be preserved rather than wiped
        let stats_after_failure = db.stats().expect("stats after failure");
        assert_eq!(stats_after_failure.file_count, 1);
    }
}

#[test]
fn an_unreadable_file_is_reported_while_the_rest_of_the_repo_indexes() {
    let temp = tempdir().expect("tempdir");
    let repo_path = temp.path();
    let git_repo = git2::Repository::init(repo_path).expect("init");
    let good = repo_path.join("good.rs");
    let bad = repo_path.join("bad.rs");
    let other = repo_path.join("other.ts");
    fs::write(&good, "pub fn good() {}\n").expect("write good.rs");
    fs::write(&bad, "pub fn bad() {}\n").expect("write bad.rs");
    fs::write(&other, "export function other() {}\n").expect("write other.ts");
    let head = create_git_commit(&git_repo, "Initial commit").expect("commit");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&bad).expect("metadata").permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&bad, perms).expect("set perms");
    }

    let db = GraphDb::open_in_memory().expect("open db");

    let outcome = index_repo(&db, "mixed", repo_path, false, &Silent).expect("index");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&bad).expect("metadata").permissions();
        perms.set_mode(0o644);
        let _ = fs::set_permissions(&bad, perms);
    }

    let [failure] = outcome.files_failed.as_slice() else {
        panic!("expected one failed file, got {:?}", outcome.files_failed);
    };
    assert_eq!(outcome.files_indexed, 2);
    assert_eq!(failure.path, "bad.rs");
    assert!(!failure.reason.is_empty());
    assert_eq!(
        db.get_repo_last_commit("mixed").expect("last commit"),
        Some(head.to_string())
    );
}

#[test]
fn parallel_extraction_indexes_every_file_once() {
    let temp = tempdir().expect("tempdir");
    let repo_path = temp.path();
    let git_repo = git2::Repository::init(repo_path).expect("git init");
    for n in 0..64 {
        fs::write(
            repo_path.join(format!("mod_{n}.rs")),
            format!(
                "pub fn function_{n}() {{}}
"
            ),
        )
        .expect("write file");
    }
    create_git_commit(&git_repo, "init 64 files").expect("commit");
    let db = GraphDb::open_in_memory().expect("open db");
    let recording = Recording::default();

    let outcome = index_repo(&db, "wide", repo_path, false, &recording).expect("index");

    assert_eq!(outcome.files_indexed, 64);
    assert!(outcome.files_failed.is_empty());
    assert_eq!(db.stats().expect("stats").file_count, 64);
    let steps = recording.steps();
    assert_eq!(steps.len(), 64);
    assert!(steps.iter().any(|step| step.starts_with("[64/64] wide:")));
    assert_eq!(recording.clears(), 1);
}

#[test]
fn duration_ms_never_wraps_negative_when_read_back_as_i64() {
    let elapsed = std::time::Duration::from_millis(1_500);
    let duration_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    assert!(
        i64::try_from(duration_ms).is_ok(),
        "duration_ms must fit a signed 64-bit count for any JSON number encoder"
    );
}

#[test]
fn full_index_stores_text_without_symbols_and_skips_binary() {
    let temp = tempdir().unwrap();
    let repo_path = temp.path();
    let repo = git2::Repository::init(repo_path).unwrap();
    fs::create_dir_all(repo_path.join(".github/workflows")).unwrap();
    fs::write(
        repo_path.join(".github/workflows/ci.yml"),
        "name: release\njobs:\n  test: {}\n",
    )
    .unwrap();
    fs::write(repo_path.join("Dockerfile"), "FROM scratch\n").unwrap();
    fs::write(repo_path.join("asset.bin"), b"text\0binary").unwrap();
    create_git_commit(&repo, "initial commit").unwrap();

    let db = GraphDb::open_in_memory().unwrap();
    let outcome = index_repo(&db, "app", repo_path, true, &Silent).unwrap();

    assert_eq!(
        db.search_file_content("release", Some("app"), 10)
            .unwrap()
            .first()
            .map(|f| f.path.as_str()),
        Some(".github/workflows/ci.yml")
    );
    assert_eq!(
        db.search_file_content("scratch", Some("app"), 10)
            .unwrap()
            .first()
            .map(|f| f.path.as_str()),
        Some("Dockerfile")
    );
    assert!(db.get_file("app", "asset.bin").unwrap().is_none());
    assert_eq!(
        outcome.files_failed.len(),
        0,
        "expected binary exclusion is not a failure"
    );
}
