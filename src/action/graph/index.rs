//! Fast incremental Git diff codebase graph indexer.
//!
//! Synchronously detects repository changes using `git2` diff delta against the last indexed
//! commit OID, parses modified/added files via Tree-sitter, deletes obsolete files/symbols/edges,
//! and updates dangling references in SQLite.

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::infra::graph::db::{GraphDb, GraphDbError};
use crate::infra::graph::extractor::{extract_file, ExtractorError};
use crate::infra::graph::parser::SupportedLanguage;
use crate::infra::hash;

/// Error encountered during repository indexing.
#[derive(Debug, Error)]
pub enum IndexError {
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),
    #[error("Database error: {0}")]
    Db(#[from] GraphDbError),
    #[error("AST extractor error in `{path}`: {source}")]
    Extractor {
        path: String,
        source: ExtractorError,
    },
    #[error("IO error in `{path}`: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

/// Statistics and outcome of an indexing run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IndexOutcome {
    pub repo: String,
    pub files_indexed: usize,
    pub files_deleted: usize,
    pub symbols_indexed: usize,
    pub edges_indexed: usize,
    pub duration_ms: u64,
    pub skipped_up_to_date: bool,
}

/// Incrementally indexes a git repository into the GraphDb.
///
/// Fast Path:
/// If the last indexed commit matches HEAD and there are no uncommitted working tree changes,
/// returns immediately with `skipped_up_to_date = true` in <1ms.
///
/// Delta Path:
/// Computes diffs against the last indexed commit (or scans the full tree on first index),
/// processes deleted files, extracts AST symbols/edges for modified & added files, and
/// updates the last indexed commit OID.
pub fn index_repo(
    db: &GraphDb,
    repo_id: &str,
    repo_path: &Path,
) -> Result<IndexOutcome, IndexError> {
    let start_time = Instant::now();

    let git_repo = git2::Repository::open(repo_path)?;

    // Ensure repo record exists in GraphDb
    let default_branch = git_repo
        .head()
        .ok()
        .and_then(|h| h.shorthand().ok().map(String::from))
        .unwrap_or_else(|| "main".to_string());
    db.insert_repo(
        repo_id,
        &repo_path.to_string_lossy(),
        &default_branch,
        None,
    )?;

    // Retrieve HEAD commit OID
    let head_oid = match git_repo.head().and_then(|h| h.target().ok_or_else(|| git2::Error::from_str("HEAD has no target"))) {
        Ok(oid) => Some(oid),
        Err(_) => None,
    };

    let head_oid_str = head_oid.map(|oid| oid.to_string());
    let last_indexed = db.get_repo_last_commit(repo_id)?;

    // Fast check: If HEAD commit is unchanged, check if workdir is clean
    if let (Some(current_head), Some(last_head)) = (&head_oid_str, &last_indexed) {
        if current_head == last_head {
            // Check if there are dirty working tree modifications to supported files
            let mut diff_opts = git2::DiffOptions::new();
            diff_opts.include_untracked(true);
            diff_opts.recurse_untracked_dirs(true);

            let head_commit = git_repo.find_commit(head_oid.unwrap())?;
            let head_tree = head_commit.tree()?;
            let diff = git_repo.diff_tree_to_workdir_with_index(Some(&head_tree), Some(&mut diff_opts))?;

            let mut has_relevant_changes = false;
            diff.foreach(
                &mut |delta, _| {
                    if let Some(path) = delta.new_file().path().or_else(|| delta.old_file().path()) {
                        if is_supported_file(path) {
                            has_relevant_changes = true;
                            return false; // stop iteration
                        }
                    }
                    true
                },
                None,
                None,
                None,
            )?;

            if !has_relevant_changes {
                let duration_ms = start_time.elapsed().as_millis() as u64;
                return Ok(IndexOutcome {
                    repo: repo_id.to_string(),
                    files_indexed: 0,
                    files_deleted: 0,
                    symbols_indexed: 0,
                    edges_indexed: 0,
                    duration_ms,
                    skipped_up_to_date: true,
                });
            }
        }
    }

    // Determine changed/deleted files
    let mut files_to_index = Vec::new();
    let mut files_to_delete = Vec::new();

    if let Some(last_head_str) = &last_indexed {
        // Delta Path: Diff from last indexed commit tree to workdir
        let last_oid = git2::Oid::from_str(last_head_str).ok();
        let last_tree = if let Some(oid) = last_oid {
            git_repo.find_commit(oid).ok().and_then(|c| c.tree().ok())
        } else {
            None
        };

        let mut diff_opts = git2::DiffOptions::new();
        diff_opts.include_untracked(true);
        diff_opts.recurse_untracked_dirs(true);

        let diff = git_repo.diff_tree_to_workdir_with_index(last_tree.as_ref(), Some(&mut diff_opts))?;

        let mut seen_indexed = HashSet::new();
        let mut seen_deleted = HashSet::new();

        diff.foreach(
            &mut |delta, _| {
                match delta.status() {
                    git2::Delta::Deleted => {
                        if let Some(path) = delta.old_file().path() {
                            if is_supported_file(path) {
                                let path_str = path.to_string_lossy().to_string();
                                if seen_deleted.insert(path_str.clone()) {
                                    files_to_delete.push(path_str);
                                }
                            }
                        }
                    }
                    git2::Delta::Added
                    | git2::Delta::Modified
                    | git2::Delta::Untracked
                    | git2::Delta::Typechange
                    | git2::Delta::Renamed
                    | git2::Delta::Copied => {
                        if let Some(old_path) = delta.old_file().path() {
                            if delta.status() == git2::Delta::Renamed && is_supported_file(old_path) {
                                let old_path_str = old_path.to_string_lossy().to_string();
                                if seen_deleted.insert(old_path_str.clone()) {
                                    files_to_delete.push(old_path_str);
                                }
                            }
                        }
                        if let Some(new_path) = delta.new_file().path() {
                            if is_supported_file(new_path) {
                                let path_str = new_path.to_string_lossy().to_string();
                                if seen_indexed.insert(path_str.clone()) {
                                    files_to_index.push(path_str);
                                }
                            }
                        }
                    }
                    _ => {}
                }
                true
            },
            None,
            None,
            None,
        )?;
    } else {
        // Initial Full Index Path: Walk working directory
        for entry in walkdir::WalkDir::new(repo_path)
            .into_iter()
            .filter_entry(|e| !is_ignored_dir(e.file_name()))
        {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if entry.file_type().is_file() {
                if let Ok(rel_path) = entry.path().strip_prefix(repo_path) {
                    if is_supported_file(rel_path) {
                        files_to_index.push(rel_path.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    // Process deletions
    let num_files_deleted = files_to_delete.len();
    for del_path in &files_to_delete {
        db.delete_file_cascade(repo_id, del_path)?;
    }

    // Process added / modified files
    let mut num_symbols_indexed = 0;
    let mut num_edges_indexed = 0;
    let mut num_files_indexed = 0;

    for rel_path_str in &files_to_index {
        let full_path = repo_path.join(rel_path_str);
        if !full_path.exists() {
            // File might have been deleted in worktree
            db.delete_file_cascade(repo_id, rel_path_str)?;
            continue;
        }

        let ext = Path::new(rel_path_str)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let lang = match SupportedLanguage::from_extension(ext) {
            Some(l) => l,
            None => continue,
        };

        let metadata = std::fs::metadata(&full_path).map_err(|e| IndexError::Io {
            path: rel_path_str.clone(),
            source: e,
        })?;

        let size_bytes = metadata.len() as i64;
        let mtime_ns = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);

        let content = std::fs::read_to_string(&full_path).map_err(|e| IndexError::Io {
            path: rel_path_str.clone(),
            source: e,
        })?;

        let content_hash = hash::text(&content);

        // Check if file content is unchanged (e.g. timestamp touched only)
        if let Some(existing_file) = db.get_file(repo_id, rel_path_str)? {
            if existing_file.content_hash == content_hash {
                // Same content, skip re-parsing
                continue;
            }
        }

        let extracted = extract_file(repo_id, rel_path_str, &content, lang).map_err(|e| {
            IndexError::Extractor {
                path: rel_path_str.clone(),
                source: e,
            }
        })?;

        let (sym_count, edge_count) = db.index_extracted_file(
            repo_id,
            rel_path_str,
            &content_hash,
            mtime_ns,
            size_bytes,
            &extracted,
        )?;

        num_symbols_indexed += sym_count;
        num_edges_indexed += edge_count;
        num_files_indexed += 1;
    }

    // Update last indexed commit
    if let Some(head_str) = &head_oid_str {
        db.update_repo_commit(repo_id, head_str)?;
    }

    let duration_ms = start_time.elapsed().as_millis() as u64;

    Ok(IndexOutcome {
        repo: repo_id.to_string(),
        files_indexed: num_files_indexed,
        files_deleted: num_files_deleted,
        symbols_indexed: num_symbols_indexed,
        edges_indexed: num_edges_indexed,
        duration_ms,
        skipped_up_to_date: false,
    })
}

fn is_supported_file(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    SupportedLanguage::from_extension(ext).is_some()
}

fn is_ignored_dir(name: &std::ffi::OsStr) -> bool {
    let s = name.to_string_lossy();
    s == ".git" || s == "target" || s == "node_modules" || s == ".ivar"
}

#[cfg(test)]
mod tests {
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
}
