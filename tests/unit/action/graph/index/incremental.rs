use super::*;

#[test]
fn a_file_with_unchanged_size_and_modification_time_is_not_read_again() {
    let temp = tempdir().expect("tempdir");
    let repo_path = temp.path();
    let git_repo = git2::Repository::init(repo_path).expect("git init");
    // The rewrite below changes ctime, which git's stat check also compares
    // (at second granularity), so a run that crosses a second boundary would
    // see the file as modified. Trust only size and mtime, the premise here.
    git_repo
        .config()
        .expect("repo config")
        .set_bool("core.trustctime", false)
        .expect("disable core.trustctime");
    let file = repo_path.join("lib.rs");
    fs::write(
        &file,
        "pub fn alpha() {}
",
    )
    .expect("write lib.rs");
    set_mtime(&file, long_ago());
    create_git_commit(&git_repo, "alpha").expect("commit");
    let db = GraphDb::open_in_memory().expect("open db");
    let first = index_repo(&db, "stat-repo", repo_path, false, &Silent).expect("first index");
    assert_eq!(first.files_indexed, 1);

    fs::write(
        &file,
        "pub fn gamma() {}
",
    )
    .expect("rewrite with the same size");
    set_mtime(&file, long_ago());
    let second = index_repo(&db, "stat-repo", repo_path, false, &Silent).expect("second index");

    assert_eq!(second.files_indexed, 0);
}

#[test]
fn a_file_modified_within_the_second_it_was_indexed_is_hashed_again() {
    let temp = tempdir().expect("tempdir");
    let repo_path = temp.path();
    let git_repo = git2::Repository::init(repo_path).expect("git init");
    let file = repo_path.join("lib.rs");
    let racy_mtime = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
    fs::write(
        &file,
        "pub fn alpha() {}
",
    )
    .expect("write lib.rs");
    set_mtime(&file, racy_mtime);
    create_git_commit(&git_repo, "alpha").expect("commit");
    let db = GraphDb::open_in_memory().expect("open db");
    index_repo(&db, "racy-repo", repo_path, false, &Silent).expect("first index");

    fs::write(
        &file,
        "pub fn gamma() {}
",
    )
    .expect("rewrite with the same size");
    set_mtime(&file, racy_mtime);
    let second = index_repo(&db, "racy-repo", repo_path, true, &Silent).expect("second index");

    assert_eq!(second.files_indexed, 1);
}

#[test]
fn a_committed_change_with_preserved_size_and_mtime_is_extracted_again() {
    let temp = tempdir().expect("tempdir");
    let repo_path = temp.path();
    let git_repo = git2::Repository::init(repo_path).expect("git init");
    let file = repo_path.join("lib.rs");
    fs::write(&file, "pub fn alpha() {}\n").expect("write lib.rs");
    set_mtime(&file, long_ago());
    create_git_commit(&git_repo, "alpha").expect("commit alpha");
    let db = GraphDb::open_in_memory().expect("open db");
    index_repo(&db, "diff-repo", repo_path, false, &Silent).expect("first index");

    fs::write(&file, "pub fn gamma() {}\n").expect("rewrite with the same size");
    set_mtime(&file, long_ago());
    create_git_commit(&git_repo, "gamma").expect("commit gamma");
    let second = index_repo(&db, "diff-repo", repo_path, false, &Silent).expect("second index");

    assert_eq!(second.files_indexed, 1);
}

#[test]
fn test_full_index_and_incremental_flow() {
    let temp = tempdir().expect("tempdir");
    let repo_path = temp.path();

    // Initialize real git repo
    let git_repo = git2::Repository::init(repo_path).expect("git init");

    // Write initial files
    let file1 = repo_path.join("main.rs");
    fs::write(
        &file1,
        r#"
        fn helper() -> i32 {
            42
        }

        fn main() {
            let x = helper();
        }
        "#,
    )
    .expect("write main.rs");

    let file2 = repo_path.join("utils.ts");
    fs::write(
        &file2,
        r#"
        export function formatName(name: string): string {
            return name.trim();
        }
        "#,
    )
    .expect("write utils.ts");

    create_git_commit(&git_repo, "Initial commit").expect("commit");

    let db = GraphDb::open_in_memory().expect("open db");

    // Test 1: Full index of initial commit
    let outcome1 = index_repo(&db, "test-repo", repo_path, false, &Silent).expect("first index");
    assert_eq!(outcome1.repo, "test-repo");
    assert_eq!(outcome1.files_indexed, 2);
    assert_eq!(outcome1.files_deleted, 0);
    assert!(outcome1.symbols_indexed >= 3);
    assert!(!outcome1.skipped_up_to_date);

    // Verify DB contains symbols
    let stats = db.stats().expect("stats");
    assert_eq!(stats.file_count, 2);
    assert!(stats.symbol_count >= 3);

    // Verify last indexed commit is set
    let last_commit = db.get_repo_last_commit("test-repo").expect("get commit");
    assert!(last_commit.is_some());

    // Test 2: Second run with no changes -> skipped_up_to_date = true
    let outcome2 = index_repo(&db, "test-repo", repo_path, false, &Silent).expect("second index");
    assert!(outcome2.skipped_up_to_date);
    assert_eq!(outcome2.files_indexed, 0);
    assert_eq!(outcome2.files_deleted, 0);

    // Test 3: Modify 1 file, commit, and index -> only 1 file indexed, old symbols updated
    fs::write(
        &file1,
        r#"
        fn new_helper() -> i32 {
            99
        }

        fn main() {
            let y = new_helper();
        }
        "#,
    )
    .expect("write modified main.rs");

    create_git_commit(&git_repo, "Update main.rs").expect("second commit");

    let outcome3 =
        index_repo(&db, "test-repo", repo_path, false, &Silent).expect("incremental index");
    assert!(!outcome3.skipped_up_to_date);
    assert_eq!(outcome3.files_indexed, 1);
    assert_eq!(outcome3.files_deleted, 0);

    // Verify symbols were updated (helper gone, new_helper present)
    let fts = db.search_symbols_fts("helper", 10).expect("search");
    let helper_syms: Vec<_> = fts.iter().filter(|s| s.name == "helper").collect();
    assert_eq!(helper_syms.len(), 0);

    let new_helper_syms: Vec<_> = fts.iter().filter(|s| s.name == "new_helper").collect();
    assert_eq!(new_helper_syms.len(), 1);

    // Test 4: Delete 1 file, commit, and index -> file and symbols deleted from DB
    fs::remove_file(&file2).expect("delete utils.ts");
    create_git_commit(&git_repo, "Delete utils.ts").expect("delete commit");

    let outcome4 = index_repo(&db, "test-repo", repo_path, false, &Silent).expect("delete index");
    assert!(!outcome4.skipped_up_to_date);
    assert_eq!(outcome4.files_indexed, 0);
    assert_eq!(outcome4.files_deleted, 1);

    let stats_after = db.stats().expect("stats");
    assert_eq!(stats_after.file_count, 1);

    let fts_utils = db.search_symbols_fts("formatName", 10).expect("search");
    assert_eq!(fts_utils.len(), 0);
}

#[test]
fn test_index_repo_dirty_worktree_incremental_flow() {
    let temp = tempdir().expect("tempdir");
    let repo_path = temp.path();
    let git_repo = git2::Repository::init(repo_path).expect("git init");

    let file1 = repo_path.join("main.rs");
    let file2 = repo_path.join("extra.rs");
    fs::write(
        &file1,
        r#"
        pub fn initial_func() {}
        "#,
    )
    .expect("write main.rs");
    fs::write(
        &file2,
        r#"
        pub fn extra_func() {}
        "#,
    )
    .expect("write extra.rs");

    create_git_commit(&git_repo, "Initial commit").expect("commit");

    let db = GraphDb::open_in_memory().expect("open db");

    let outcome1 = index_repo(&db, "test-repo", repo_path, false, &Silent).expect("initial index");
    assert_eq!(outcome1.files_indexed, 2);
    assert!(!outcome1.skipped_up_to_date);
    assert!(
        db.get_file("test-repo", "main.rs")
            .expect("get main.rs")
            .is_some()
    );
    assert!(
        db.get_file("test-repo", "extra.rs")
            .expect("get extra.rs")
            .is_some()
    );

    let outcome2 = index_repo(&db, "test-repo", repo_path, false, &Silent).expect("clean index");
    assert!(outcome2.skipped_up_to_date);
    assert_eq!(outcome2.files_indexed, 0);

    // 1. Uncommitted modification to tracked file
    fs::write(
        &file1,
        r#"
        pub fn initial_func() {}
        pub fn dirty_added_func() {}
        "#,
    )
    .expect("modify main.rs");

    let outcome3 =
        index_repo(&db, "test-repo", repo_path, false, &Silent).expect("dirty edit index");
    assert!(
        !outcome3.skipped_up_to_date,
        "dirty worktree must not be skipped as up to date"
    );
    assert_eq!(outcome3.files_indexed, 1);

    let found = db
        .search_symbols_fts("dirty_added_func", 10)
        .expect("search");
    assert_eq!(found.len(), 1);
    assert_eq!(
        found.first().map(|s| s.name.as_str()),
        Some("dirty_added_func")
    );

    // 2. Uncommitted addition of untracked Rust file
    let file3 = repo_path.join("untracked.rs");
    fs::write(
        &file3,
        r#"
        pub fn untracked_func() {}
        "#,
    )
    .expect("write untracked.rs");

    let outcome4 =
        index_repo(&db, "test-repo", repo_path, false, &Silent).expect("dirty untracked add index");
    assert!(
        !outcome4.skipped_up_to_date,
        "dirty worktree with untracked file must not be skipped"
    );
    assert_eq!(outcome4.files_indexed, 1);
    assert!(
        db.get_file("test-repo", "untracked.rs")
            .expect("get untracked.rs")
            .is_some()
    );

    let found_untracked = db.search_symbols_fts("untracked_func", 10).expect("search");
    assert_eq!(found_untracked.len(), 1);
    assert_eq!(
        found_untracked.first().map(|s| s.name.as_str()),
        Some("untracked_func")
    );

    // 3. Uncommitted deletion of tracked file
    fs::remove_file(&file2).expect("remove extra.rs");

    let outcome5 =
        index_repo(&db, "test-repo", repo_path, false, &Silent).expect("dirty delete index");
    assert!(
        !outcome5.skipped_up_to_date,
        "dirty worktree with deleted file must not be skipped"
    );
    assert_eq!(outcome5.files_deleted, 1);
    assert!(
        db.get_file("test-repo", "extra.rs")
            .expect("get extra.rs")
            .is_none()
    );

    let found_deleted = db.search_symbols_fts("extra_func", 10).expect("search");
    assert_eq!(found_deleted.len(), 0);
}
