//! Fast incremental Git diff codebase graph indexer.
//!
//! Synchronously detects repository changes using `git2` diff delta against the last indexed
//! commit OID, parses modified/added files via Tree-sitter, deletes obsolete files/symbols/edges,
//! and updates dangling references in SQLite.

pub mod types;

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use camino::Utf8Path;

use crate::git::Git;
use crate::infra::graph::parser::SupportedLanguage;
use crate::infra::hash;
use crate::infra::progress::Progress;
use crate::store::graph::db::GraphDb;
use crate::store::graph::extractor::extract_file;
pub use types::*;

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
    force_full: bool,
    progress: &dyn Progress,
) -> Result<IndexOutcome, IndexError> {
    let start_time = Instant::now();
    let git = crate::git::System;
    let repo_utf8 = Utf8Path::from_path(repo_path).ok_or_else(|| {
        crate::git::Error::NotUtf8 {
            display: repo_path.to_string_lossy().to_string(),
        }
    })?;

    let default_branch = git
        .head_branch(repo_utf8)
        .unwrap_or_else(|_| "main".to_owned());
    db.insert_repo(repo_id, &repo_path.to_string_lossy(), &default_branch, None)?;

    let head_commit = git.head_commit(repo_utf8).ok();
    let last_indexed = db.get_repo_last_commit(repo_id)?;

    // Fast Path: HEAD unchanged and working tree clean
    if !force_full
        && let (Some(head_sha), Some(last_commit_str)) = (&head_commit, &last_indexed)
        && head_sha == last_commit_str
        && !git.worktree_dirty(repo_utf8).unwrap_or(true)
    {
        let duration_ms = start_time.elapsed().as_millis() as u64;
        return Ok(IndexOutcome {
            repo: repo_id.to_owned(),
            files_indexed: 0,
            files_deleted: 0,
            symbols_indexed: 0,
            edges_indexed: 0,
            duration_ms,
            skipped_up_to_date: true,
        });
    }

    let mut files_to_index = Vec::new();
    let mut files_to_delete = Vec::new();

    if !force_full && let Some(last_head_str) = &last_indexed {
        match git.diff_worktree_files(repo_utf8, Some(last_head_str)) {
            Ok(diff) => {
                for p in diff.modified_or_added {
                    let p_std = p.as_std_path();
                    if is_supported_file(p_std) && !is_ignored_path(p_std) {
                        files_to_index.push(p.to_string());
                    }
                }
                for p in diff.deleted {
                    let p_std = p.as_std_path();
                    if is_supported_file(p_std) && !is_ignored_path(p_std) {
                        files_to_delete.push(p.to_string());
                    }
                }
                if let Some(head_sha) = &head_commit
                    && head_sha == last_head_str
                    && files_to_index.is_empty()
                    && files_to_delete.is_empty()
                {
                    let duration_ms = start_time.elapsed().as_millis() as u64;
                    return Ok(IndexOutcome {
                        repo: repo_id.to_owned(),
                        files_indexed: 0,
                        files_deleted: 0,
                        symbols_indexed: 0,
                        edges_indexed: 0,
                        duration_ms,
                        skipped_up_to_date: true,
                    });
                }
            }
            Err(_) => {
                files_to_index.clear();
                files_to_delete.clear();
            }
        }
    }

    if force_full
        || last_indexed.is_none()
        || (files_to_index.is_empty()
            && files_to_delete.is_empty()
            && !matches!(&head_commit, Some(h) if last_indexed.as_deref() == Some(h)))
    {
        db.delete_repo_files(repo_id)?;
        let mut seen = HashSet::new();

        for entry in walkdir::WalkDir::new(repo_path)
            .into_iter()
            .filter_entry(|e| !is_ignored_dir(e.file_name()))
            .filter_map(Result::ok)
        {
            if entry.file_type().is_file()
                && let Ok(rel) = entry.path().strip_prefix(repo_path)
                && is_supported_file(rel)
                && !is_ignored_path(rel)
                && let Some(rel_utf8) = Utf8Path::from_path(rel)
                && !git.is_path_ignored(repo_utf8, rel_utf8).unwrap_or(false)
            {
                let rel_str = rel.to_string_lossy().to_string();
                if seen.insert(rel_str.clone()) {
                    files_to_index.push(rel_str);
                }
            }
        }
    }
    let num_files_deleted = files_to_delete.len();
    for del in &files_to_delete {
        db.delete_file_cascade(repo_id, del)?;
    }

    let total_to_index = files_to_index.len();
    let mut num_files_indexed = 0;
    let mut num_symbols_indexed = 0;
    let mut num_edges_indexed = 0;

    let index_res = (|| -> Result<(), IndexError> {
        for (idx, rel_path_str) in files_to_index.iter().enumerate() {
            progress.step(&format!(
                "[{}/{total_to_index}] {repo_id}: {rel_path_str}",
                idx + 1
            ));

            let full_path = repo_path.join(rel_path_str);
            if !full_path.exists() {
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

            let content =
                std::fs::read_to_string(&full_path).map_err(|e| IndexError::Io {
                    path: rel_path_str.clone(),
                    source: e,
                })?;

            let content_hash = hash::text(&content);

            if let Some(existing_file) = db.get_file(repo_id, rel_path_str)?
                && existing_file.content_hash == content_hash
            {
                continue;
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
        Ok(())
    })();

    progress.clear();
    index_res?;

    if num_files_indexed > 0 || num_files_deleted > 0 {
        db.relink_dangling_edges(repo_id)?;
    }

    if let Some(head_str) = &head_commit {
        db.update_repo_commit(repo_id, head_str)?;
    }

    let duration_ms = start_time.elapsed().as_millis() as u64;
    Ok(IndexOutcome {
        repo: repo_id.to_owned(),
        files_indexed: num_files_indexed,
        files_deleted: num_files_deleted,
        symbols_indexed: num_symbols_indexed,
        edges_indexed: num_edges_indexed,
        duration_ms,
        skipped_up_to_date: false,
    })
}

/// Indexes all mounted repositories in an ivar hall.
pub fn index_hall(
    db: &GraphDb,
    hall_root: &Path,
    force_full: bool,
    progress: &dyn Progress,
) -> Result<Vec<IndexOutcome>, IndexError> {
    let repos_dir = hall_root.join(".ivar").join("repos");
    let mut outcomes = Vec::new();

    if repos_dir.exists() && repos_dir.is_dir() {
        let entries = std::fs::read_dir(&repos_dir).map_err(|e| IndexError::Io {
            path: repos_dir.to_string_lossy().to_string(),
            source: e,
        })?;
        for entry in entries.filter_map(std::result::Result::ok) {
            let repo_name = entry.file_name().to_string_lossy().to_string();
            let repo_worktree = entry.path().join("main");
            let target_path = if repo_worktree.exists() {
                repo_worktree
            } else {
                entry.path()
            };

            if (target_path.join(".git").exists() || target_path.is_dir())
                && let Ok(outcome) =
                    index_repo(db, &repo_name, &target_path, force_full, progress)
            {
                outcomes.push(outcome);
            }
        }
    }

    Ok(outcomes)
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/index.rs"]
mod tests;
