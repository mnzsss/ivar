//! Fast incremental Git diff codebase graph indexer.
//!
//! Synchronously detects repository changes using `git2` diff delta against the last indexed
//! commit OID, parses modified/added files via Tree-sitter, deletes obsolete files/symbols/edges,
//! and updates dangling references in SQLite.

pub mod types;

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use camino::Utf8Path;

use crate::git::Git;
use crate::infra::graph::parser::SupportedLanguage;
use crate::infra::hash;
use crate::infra::progress::Progress;
use crate::store::graph::db::GraphDb;
use crate::store::graph::extractor::{ExtractedFile, extract_file};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
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
    let repo_utf8 = Utf8Path::from_path(repo_path).ok_or_else(|| crate::git::Error::NotUtf8 {
        display: repo_path.to_string_lossy().to_string(),
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
        let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
        return Ok(IndexOutcome {
            repo: repo_id.to_owned(),
            files_indexed: 0,
            files_deleted: 0,
            symbols_indexed: 0,
            edges_indexed: 0,
            duration_ms,
            skipped_up_to_date: true,
            files_failed: Vec::new(),
        });
    }

    let mut files_to_index = Vec::new();
    let mut files_to_delete = Vec::new();
    let mut listed_by_git_diff = false;

    if !force_full && let Some(last_head_str) = &last_indexed {
        match git.diff_worktree_files(repo_utf8, Some(last_head_str)) {
            Ok(diff) => {
                listed_by_git_diff = true;
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
                    let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
                    return Ok(IndexOutcome {
                        repo: repo_id.to_owned(),
                        files_indexed: 0,
                        files_deleted: 0,
                        symbols_indexed: 0,
                        edges_indexed: 0,
                        duration_ms,
                        skipped_up_to_date: true,
                        files_failed: Vec::new(),
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
        listed_by_git_diff = false;
        files_to_index.clear();
        files_to_delete.clear();
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

        let existing_files = db.get_files_for_repo(repo_id)?;
        for existing in existing_files {
            if !seen.contains(&existing.path) {
                files_to_delete.push(existing.path);
            }
        }
    }
    let num_files_deleted = files_to_delete.len();

    let mut num_files_indexed = 0;
    let mut num_symbols_indexed = 0;
    let mut num_edges_indexed = 0;
    let mut files_failed = Vec::new();

    let index_res = (|| -> Result<(), IndexError> {
        let mut jobs = Vec::new();
        for rel_path in &files_to_index {
            let full_path = repo_path.join(rel_path);
            if !full_path.exists() {
                db.delete_file_cascade(repo_id, rel_path)?;
                continue;
            }

            let Some(lang) = Path::new(rel_path)
                .extension()
                .and_then(|e| e.to_str())
                .and_then(SupportedLanguage::from_extension)
            else {
                continue;
            };

            let metadata = match std::fs::metadata(&full_path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    files_failed.push(FileFailure::new(rel_path, error));
                    continue;
                }
            };

            let size_bytes = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
            let mtime_ns = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
                .unwrap_or(0);

            let existing_file = if force_full {
                None
            } else {
                db.get_file(repo_id, rel_path)?
            };
            // Same size and modification time means unchanged, the check git trusts
            // for its own index, so an unchanged file is neither read nor hashed.
            // A path git's diff reports as changed is hashed regardless.
            if !listed_by_git_diff
                && existing_file.as_ref().is_some_and(|file| {
                    file.size_bytes == size_bytes
                        && file.mtime_ns == mtime_ns
                        && file.is_stat_trustworthy()
                })
            {
                continue;
            }

            jobs.push(ExtractJob {
                rel_path: rel_path.clone(),
                full_path,
                lang,
                size_bytes,
                mtime_ns,
                known_hash: existing_file.map(|file| file.content_hash),
            });
        }

        let total = jobs.len();
        let workers = std::thread::available_parallelism()
            .map_or(1, NonZeroUsize::get)
            .min(total);
        let next_job = AtomicUsize::new(0);
        std::thread::scope(|scope| -> Result<(), IndexError> {
            // SQLite writes stay on this thread; the bound keeps extracted files
            // from piling up in memory while the writer catches up.
            let (sender, receiver) = mpsc::sync_channel(workers * 2);
            for _ in 0..workers {
                let sender = sender.clone();
                let (jobs, next_job) = (&jobs, &next_job);
                scope.spawn(move || {
                    while let Some(job) = jobs.get(next_job.fetch_add(1, Ordering::Relaxed)) {
                        if sender.send((job, extract_job(repo_id, job))).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(sender);

            for (done, (job, extraction)) in receiver.into_iter().enumerate() {
                progress.step(&format!(
                    "[{}/{total}] {repo_id}: {}",
                    done + 1,
                    job.rel_path
                ));
                match extraction {
                    Extraction::Unchanged => {}
                    Extraction::Failed(reason) => files_failed.push(FileFailure {
                        path: job.rel_path.clone(),
                        reason,
                    }),
                    Extraction::Extracted {
                        content_hash,
                        extracted,
                    } => {
                        let (sym_count, edge_count) = db.index_extracted_file(
                            repo_id,
                            &job.rel_path,
                            &content_hash,
                            job.mtime_ns,
                            job.size_bytes,
                            &extracted,
                        )?;
                        num_symbols_indexed += sym_count;
                        num_edges_indexed += edge_count;
                        num_files_indexed += 1;
                    }
                }
            }
            Ok(())
        })?;
        for del in &files_to_delete {
            db.delete_file_cascade(repo_id, del)?;
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

    let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(IndexOutcome {
        repo: repo_id.to_owned(),
        files_indexed: num_files_indexed,
        files_deleted: num_files_deleted,
        symbols_indexed: num_symbols_indexed,
        edges_indexed: num_edges_indexed,
        duration_ms,
        skipped_up_to_date: false,
        files_failed,
    })
}

struct ExtractJob {
    rel_path: String,
    full_path: PathBuf,
    lang: SupportedLanguage,
    size_bytes: i64,
    mtime_ns: i64,
    known_hash: Option<String>,
}

enum Extraction {
    Unchanged,
    Extracted {
        content_hash: String,
        extracted: ExtractedFile,
    },
    Failed(String),
}

fn extract_job(repo_id: &str, job: &ExtractJob) -> Extraction {
    let content = match std::fs::read_to_string(&job.full_path) {
        Ok(content) => content,
        Err(error) => return Extraction::Failed(error.to_string()),
    };
    let content_hash = hash::text(&content);
    if job.known_hash.as_deref() == Some(content_hash.as_str()) {
        return Extraction::Unchanged;
    }
    match extract_file(repo_id, &job.rel_path, &content, job.lang) {
        Ok(extracted) => Extraction::Extracted {
            content_hash,
            extracted,
        },
        Err(error) => Extraction::Failed(error.to_string()),
    }
}

/// Indexes every base repository of the hall whose default-branch worktree
/// exists. A repository that fails is recorded and the rest still index.
pub fn index_hall(
    db: &GraphDb,
    layout: &Layout,
    manifest: &Manifest,
    force_full: bool,
    progress: &dyn Progress,
) -> HallIndex {
    let mut hall = HallIndex::default();
    for repo in manifest.repos() {
        let worktree = layout.repo_worktree(repo.name(), repo.default_branch());
        if !worktree.as_std_path().exists() {
            continue;
        }
        match index_repo(
            db,
            repo.name().as_str(),
            worktree.as_std_path(),
            force_full,
            progress,
        ) {
            Ok(outcome) => hall.repos.push(outcome),
            Err(error) => hall.repos_failed.push(RepoFailure {
                repo: repo.name().to_string(),
                reason: error.to_string(),
            }),
        }
    }
    hall
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/index.rs"]
mod tests;
