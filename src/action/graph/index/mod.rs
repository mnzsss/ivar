//! Codebase graph indexing orchestrator.
//!
//! Traverses repository source files, parses definitions and references via tree-sitter,
//! and persists symbol tables, reference edges, and FTS indexes to SQLite.

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use camino::{Utf8Path, Utf8PathBuf};
use sha2::{Digest, Sha256};
use types::classify_text;

use crate::action::progress::Progress;
use crate::git::Git;
use crate::infra::graph::parser::SupportedLanguage;
use crate::store::graph::db::GraphDb;
use crate::store::graph::extractor::{ExtractedFile, extract_file};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

pub mod types;
pub use types::{
    FileFailure, HallIndex, IndexError, IndexOutcome, RepoFailure, TextClassification,
    is_ignored_dir, is_ignored_file_path, is_ignored_path,
};

/// Indexes a single repository, returning statistics on what was added/updated.
///
/// If `force_full` is false and the repository HEAD matches the indexed HEAD,
/// indexing is skipped entirely.
///
/// # Errors
///
/// Returns [`IndexError`] if Git discovery, AST parsing, or SQLite transactions fail.
pub fn index_repo(
    db: &GraphDb,
    repo_id: &str,
    repo_path: &Path,
    force_full: bool,
    progress: &dyn Progress,
) -> Result<IndexOutcome, IndexError> {
    let start_time = Instant::now();
    let repo_utf8 = Utf8Path::from_path(repo_path).ok_or_else(|| {
        IndexError::Git(crate::git::Error::NotARepository {
            path: Utf8PathBuf::from(repo_path.to_string_lossy().to_string()),
            detail: "Path is not valid UTF-8".to_owned(),
        })
    })?;
    let git = crate::git::System;

    let head_commit = git.head_commit(repo_utf8).ok();
    let last_indexed = db.get_repo_last_commit(repo_id)?;

    if !force_full
        && let Some(head) = &head_commit
        && last_indexed.as_deref() == Some(head.as_str())
    {
        return Ok(up_to_date_outcome(repo_id, start_time));
    }

    let (files_to_index, files_to_delete, listed_by_git_diff) = discover_changed_files(
        &git,
        db,
        repo_id,
        repo_path,
        repo_utf8,
        force_full,
        &last_indexed,
        &head_commit,
    )?;

    if !force_full && files_to_index.is_empty() && files_to_delete.is_empty() {
        return Ok(up_to_date_outcome(repo_id, start_time));
    }

    let default_branch = git
        .head_branch(repo_utf8)
        .unwrap_or_else(|_| "main".to_owned());
    db.insert_repo(
        repo_id,
        repo_utf8.as_str(),
        &default_branch,
        head_commit.as_deref(),
    )?;

    let (jobs, mut files_failed) = build_extract_jobs(
        db,
        repo_id,
        repo_path,
        &files_to_index,
        force_full,
        listed_by_git_diff,
    )?;

    let (num_files_indexed, num_symbols_indexed, num_edges_indexed, job_failures) =
        run_extraction_jobs(&jobs, repo_id, db, progress, &files_to_delete)?;
    files_failed.extend(job_failures);

    let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(IndexOutcome {
        repo: repo_id.to_owned(),
        files_indexed: num_files_indexed,
        files_deleted: files_to_delete.len(),
        symbols_indexed: num_symbols_indexed,
        edges_indexed: num_edges_indexed,
        duration_ms,
        skipped_up_to_date: false,
        files_failed,
    })
}

fn up_to_date_outcome(repo_id: &str, start_time: Instant) -> IndexOutcome {
    let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
    IndexOutcome {
        repo: repo_id.to_owned(),
        files_indexed: 0,
        files_deleted: 0,
        symbols_indexed: 0,
        edges_indexed: 0,
        duration_ms,
        skipped_up_to_date: true,
        files_failed: Vec::new(),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "threads through the same params index_repo already gathered"
)]
fn discover_changed_files(
    git: &dyn crate::git::Git,
    db: &GraphDb,
    repo_id: &str,
    _repo_path: &Path,
    repo_utf8: &Utf8Path,
    force_full: bool,
    last_indexed: &Option<String>,
    head_commit: &Option<String>,
) -> Result<(Vec<String>, Vec<String>, bool), IndexError> {
    let mut files_to_index = Vec::new();
    let mut files_to_delete = Vec::new();
    let mut listed_by_git_diff = false;

    if !force_full && let Some(last_head_str) = last_indexed {
        match git.diff_worktree_files(repo_utf8, Some(last_head_str)) {
            Ok(diff) => {
                listed_by_git_diff = true;
                for p in diff.modified_or_added {
                    let p_std = p.as_std_path();
                    if !is_ignored_file_path(p_std) && !is_ignored_path(p_std) {
                        files_to_index.push(p.to_string());
                    }
                }
                for p in diff.deleted {
                    let p_std = p.as_std_path();
                    if !is_ignored_file_path(p_std) && !is_ignored_path(p_std) {
                        files_to_delete.push(p.to_string());
                    }
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
            && !matches!(head_commit, Some(h) if last_indexed.as_deref() == Some(h.as_str())))
    {
        listed_by_git_diff = false;
        files_to_index.clear();
        files_to_delete.clear();
        let mut seen = HashSet::new();

        let tracked = git.tracked_files(repo_utf8)?;
        for path in tracked {
            let p_std = path.as_std_path();
            if !is_ignored_file_path(p_std) && !is_ignored_path(p_std) {
                let path_str = path.to_string();
                if seen.insert(path_str.clone()) {
                    files_to_index.push(path_str);
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

    Ok((files_to_index, files_to_delete, listed_by_git_diff))
}

fn build_extract_jobs(
    db: &GraphDb,
    repo_id: &str,
    repo_path: &Path,
    files_to_index: &[String],
    force_full: bool,
    listed_by_git_diff: bool,
) -> Result<(Vec<ExtractJob>, Vec<FileFailure>), IndexError> {
    let mut jobs = Vec::new();
    let mut files_failed = Vec::new();

    for rel_path in files_to_index {
        let full_path = repo_path.join(rel_path);
        if !full_path.exists() {
            db.delete_file_cascade(repo_id, rel_path)?;
            continue;
        }

        let lang = Path::new(rel_path)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(SupportedLanguage::from_extension);

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

    Ok((jobs, files_failed))
}

fn run_extraction_jobs(
    jobs: &[ExtractJob],
    repo_id: &str,
    db: &GraphDb,
    progress: &dyn Progress,
    files_to_delete: &[String],
) -> Result<(usize, usize, usize, Vec<FileFailure>), IndexError> {
    let mut num_files_indexed = 0;
    let mut num_symbols_indexed = 0;
    let mut num_edges_indexed = 0;
    let mut files_failed = Vec::new();

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
                Extraction::Binary => {}
                Extraction::Failed(reason) => files_failed.push(FileFailure {
                    path: job.rel_path.clone(),
                    reason,
                }),
                Extraction::Extracted {
                    content_hash,
                    indexed_content,
                    truncated,
                    extracted,
                } => {
                    let (sym_count, edge_count) = db.index_extracted_file(
                        repo_id,
                        &job.rel_path,
                        &content_hash,
                        job.mtime_ns,
                        job.size_bytes,
                        &indexed_content,
                        truncated,
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
    for del in files_to_delete {
        db.delete_file_cascade(repo_id, del)?;
    }

    if total > 0 {
        progress.clear();
    }

    Ok((
        num_files_indexed,
        num_symbols_indexed,
        num_edges_indexed,
        files_failed,
    ))
}

struct ExtractJob {
    rel_path: String,
    full_path: PathBuf,
    lang: Option<SupportedLanguage>,
    size_bytes: i64,
    mtime_ns: i64,
    known_hash: Option<String>,
}

enum Extraction {
    Unchanged,
    Binary,
    Extracted {
        content_hash: String,
        indexed_content: String,
        truncated: bool,
        extracted: ExtractedFile,
    },
    Failed(String),
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn extract_job(repo_id: &str, job: &ExtractJob) -> Extraction {
    let bytes = match std::fs::read(&job.full_path) {
        Ok(bytes) => bytes,
        Err(error) => return Extraction::Failed(error.to_string()),
    };

    let content_hash = hex(&Sha256::digest(&bytes));
    if job.known_hash.as_deref() == Some(content_hash.as_str()) {
        return Extraction::Unchanged;
    }

    let (content, indexed_len, truncated) = match classify_text(bytes) {
        TextClassification::Binary => return Extraction::Binary,
        TextClassification::Text {
            content,
            indexed_len,
            truncated,
        } => (content, indexed_len, truncated),
    };

    let extracted = match job.lang {
        Some(lang) => match extract_file(repo_id, &job.rel_path, &content, lang) {
            Ok(extracted) => extracted,
            Err(error) => return Extraction::Failed(error.to_string()),
        },
        None => ExtractedFile::default(),
    };

    let indexed_content = content[..indexed_len].to_string();

    Extraction::Extracted {
        content_hash,
        indexed_content,
        truncated,
        extracted,
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
