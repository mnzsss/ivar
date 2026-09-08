#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use std::fs;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::tempdir;

use crate::infra::progress::{Progress, Silent};

#[derive(Debug, Default)]
struct Recording {
    steps: Mutex<Vec<String>>,
    clears: AtomicUsize,
}

impl Recording {
    fn steps(&self) -> Vec<String> {
        self.steps.lock().unwrap().clone()
    }

    fn clears(&self) -> usize {
        self.clears.load(Ordering::Relaxed)
    }
}

impl Progress for Recording {
    fn step(&self, message: &str) {
        self.steps.lock().unwrap().push(message.to_owned());
    }

    fn clear(&self) {
        self.clears.fetch_add(1, Ordering::Relaxed);
    }
}
fn create_git_commit(repo: &git2::Repository, message: &str) -> Result<git2::Oid, git2::Error> {
    let mut index = repo.index()?;
    index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
    index.write()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let sig = git2::Signature::now("Ivar Test", "test@ivar.run")?;
    let parent = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .and_then(|oid| repo.find_commit(oid).ok());
    let parents: Vec<&git2::Commit> = parent.as_ref().into_iter().collect();

    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
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

    // Test 2: Second run with no changes -> skipped_up_to_date = true in <15ms
    let outcome2 = index_repo(&db, "test-repo", repo_path, false, &Silent).expect("second index");
    assert!(outcome2.skipped_up_to_date);
    assert_eq!(outcome2.files_indexed, 0);
    assert_eq!(outcome2.files_deleted, 0);
    assert!(outcome2.duration_ms < 50);

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
    assert!(outcome3.duration_ms < 50);

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
    // Only tracked.rs is indexed!
    assert_eq!(outcome.files_indexed, 1);
    assert_eq!(outcome.files_deleted, 0);

    let stats = db.stats().expect("stats");
    assert_eq!(stats.file_count, 1);

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
fn test_index_repo_progress_clears_on_error() {
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
        assert!(result.is_err());
        assert_eq!(recording.clears(), 1);
        assert_eq!(recording.steps().len(), 1);

        // Restore permissions for cleanup
        let mut perms = fs::metadata(&file).expect("metadata").permissions();
        perms.set_mode(0o644);
        let _ = fs::set_permissions(&file, perms);
    }
}
