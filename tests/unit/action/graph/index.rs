#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use std::fs;
use tempfile::tempdir;

fn create_git_commit(
    repo: &git2::Repository,
    message: &str,
) -> Result<git2::Oid, git2::Error> {
    let mut index = repo.index()?;
    index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
    index.write()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let sig = git2::Signature::now("Ivar Test", "test@ivar.run")?;
    let parent = repo.head().ok().and_then(|h| h.target()).and_then(|oid| repo.find_commit(oid).ok());
    let parents = match &parent {
        Some(p) => vec![p],
        None => vec![],
    };

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
    let outcome1 = index_repo(&db, "test-repo", repo_path).expect("first index");
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
    let outcome2 = index_repo(&db, "test-repo", repo_path).expect("second index");
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

    let outcome3 = index_repo(&db, "test-repo", repo_path).expect("incremental index");
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

    let outcome4 = index_repo(&db, "test-repo", repo_path).expect("delete index");
    assert!(!outcome4.skipped_up_to_date);
    assert_eq!(outcome4.files_indexed, 0);
    assert_eq!(outcome4.files_deleted, 1);

    let stats_after = db.stats().expect("stats");
    assert_eq!(stats_after.file_count, 1);

    let fts_utils = db.search_symbols_fts("formatName", 10).expect("search");
    assert_eq!(fts_utils.len(), 0);
}
