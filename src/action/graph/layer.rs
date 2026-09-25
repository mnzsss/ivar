//! Layer delta build, fingerprint computation, per-feature locking, and layer indexing.

use camino::Utf8Path;
use sha2::{Digest, Sha256};

use crate::action::graph::index::types::{
    TextClassification, classify_text, is_ignored_file_path, is_ignored_path,
};
use crate::error::Failure;
use crate::git::{Git, System as GitSystem, WorktreeDiff};
use crate::infra::graph::parser::SupportedLanguage;
use crate::store::graph::db::{FileRow, GraphDb};
use crate::store::graph::extractor::{ExtractedFile, extract_file};
use crate::store::layout::Layout;

/// Result of ensuring a layer is indexed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerBuildResult {
    pub layer_id: i64,
    pub indexed_files: usize,
    pub tombstoned_files: usize,
    pub skipped: bool,
}

/// Fingerprints the diff's shape: base commit, head commit and the changed
/// and deleted path sets. Content changes within that shape are caught by the
/// per-file stat and hash comparison against the layer's file rows.
fn compute_layer_fingerprint(base_commit: &str, head: Option<&str>, diff: &WorktreeDiff) -> String {
    let mut hasher = Sha256::new();
    hasher.update(base_commit.as_bytes());
    hasher.update(head.unwrap_or_default().as_bytes());
    for path in &diff.deleted {
        hasher.update(b"deleted\0");
        hasher.update(path.as_str().as_bytes());
    }
    for path in &diff.modified_or_added {
        hasher.update(b"changed\0");
        hasher.update(path.as_str().as_bytes());
    }
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn layer_error(message: String) -> Failure {
    Failure::failed("graph.layer_error", message)
}

/// Ensures a feature layer is indexed into the GraphDb for the given worktree and base commit.
///
/// 1. Acquires a per-feature file lock to prevent concurrent layer indexing races.
/// 2. Ensures layer record exists in SQLite.
/// 3. Computes fingerprint and checks if up-to-date.
/// 4. If dirty, indexes modified/added files into pseudo-repo `repo_name/layer_id` and records tombstones.
pub fn ensure_layer_indexed(
    db: &GraphDb,
    layout: &Layout,
    feature: &str,
    repo_name: &str,
    worktree: &Utf8Path,
    base_commit: &str,
) -> Result<LayerBuildResult, Failure> {
    // 1. Acquire per-feature lock to prevent concurrent layer indexing races without blocking base indexing
    let locks_dir = layout.ivar_dir().join("locks");
    std::fs::create_dir_all(&locks_dir).map_err(|e| {
        Failure::failed(
            "graph.layer_error",
            format!("Failed to create locks dir: {e}"),
        )
    })?;
    let lock_path = locks_dir.join(format!("{feature}.lock"));
    let lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| {
            Failure::failed(
                "graph.layer_error",
                format!("Failed to open feature lock {lock_path}: {e}"),
            )
        })?;
    lock_file.lock().map_err(|e| {
        Failure::failed(
            "graph.layer_error",
            format!("Failed to acquire feature lock {lock_path}: {e}"),
        )
    })?;

    // 2. Ensure layer record in DB
    let layer_id = db
        .ensure_layer_record(feature, repo_name, worktree.as_str(), base_commit)
        .map_err(|e| {
            Failure::failed(
                "graph.layer_error",
                format!("Failed to ensure layer record: {e}"),
            )
        })?;
    let layer_repo = format!("{repo_name}/{layer_id}");

    let git = GitSystem;
    let head_commit = git.head_commit(worktree).ok();

    let diff = git
        .diff_worktree_files(worktree, Some(base_commit))
        .map_err(|e| Failure::failed("graph.layer_error", format!("Git diff error: {e}")))?;

    let fingerprint = compute_layer_fingerprint(base_commit, head_commit.as_deref(), &diff);
    let fingerprint_matches = db
        .get_layer_record(feature, repo_name)
        .map_err(|e| layer_error(format!("Failed to get layer record: {e}")))?
        .is_some_and(|existing| existing.fingerprint.as_deref() == Some(&fingerprint));

    let mut stale: std::collections::HashMap<String, FileRow> = db
        .get_files_for_repo(&layer_repo)
        .map_err(|e| layer_error(format!("Failed to list layer files: {e}")))?
        .into_iter()
        .map(|row| (row.path.clone(), row))
        .collect();

    if !fingerprint_matches {
        db.insert_repo(
            &layer_repo,
            worktree.as_str(),
            feature,
            head_commit.as_deref(),
        )
        .map_err(|e| layer_error(format!("Failed to insert layer repo: {e}")))?;
        let tombstones: Vec<&str> = diff.deleted.iter().map(|p| p.as_str()).collect();
        db.set_layer_tombstones(layer_id, &tombstones)
            .map_err(|e| layer_error(format!("Failed to set layer tombstones: {e}")))?;
    }

    let mut indexed_count = 0;
    for rel_path in &diff.modified_or_added {
        let p_std = rel_path.as_std_path();
        if is_ignored_file_path(p_std) || is_ignored_path(p_std) {
            continue;
        }
        let existing = stale.remove(rel_path.as_str());
        if index_layer_file(db, &layer_repo, worktree, rel_path, existing.as_ref())? {
            indexed_count += 1;
        }
    }

    let removed_count = stale.len();
    for path in stale.keys() {
        db.delete_file(&layer_repo, path)
            .map_err(|e| layer_error(format!("Failed to drop layer file {path}: {e}")))?;
    }

    if !fingerprint_matches {
        db.update_layer_fingerprint_and_head(layer_id, &fingerprint, head_commit.as_deref())
            .map_err(|e| layer_error(format!("Failed to update layer fingerprint: {e}")))?;
    }

    Ok(LayerBuildResult {
        layer_id,
        indexed_files: indexed_count,
        tombstoned_files: diff.deleted.len(),
        skipped: fingerprint_matches && indexed_count == 0 && removed_count == 0,
    })
}

fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Brings one changed file's layer rows up to date and reports whether it
/// was re-extracted. Files that are not indexable are removed from the layer.
fn index_layer_file(
    db: &GraphDb,
    layer_repo: &str,
    worktree: &Utf8Path,
    rel_path: &Utf8Path,
    existing: Option<&FileRow>,
) -> Result<bool, Failure> {
    let full_path = worktree.join(rel_path);
    let meta = std::fs::metadata(&full_path).ok().filter(|m| m.is_file());
    let Some(meta) = meta else {
        return drop_layer_file(db, layer_repo, rel_path, existing);
    };

    let mtime_ns = mtime_ns(&meta);
    let size_bytes = i64::try_from(meta.len()).unwrap_or(i64::MAX);
    if let Some(row) = existing
        && row.mtime_ns == mtime_ns
        && row.size_bytes == size_bytes
        && row.is_stat_trustworthy()
    {
        return Ok(false);
    }

    let Ok(bytes) = std::fs::read(&full_path) else {
        return drop_layer_file(db, layer_repo, rel_path, existing);
    };

    let hash = hex(&Sha256::digest(&bytes));
    if let Some(row) = existing
        && row.content_hash == hash
    {
        db.upsert_file(layer_repo, rel_path.as_str(), &hash, mtime_ns, size_bytes)
            .map_err(|e| layer_error(format!("Failed to refresh stat for {rel_path}: {e}")))?;
        return Ok(false);
    }

    let (content, indexed_len, truncated) = match classify_text(bytes) {
        TextClassification::Binary => return drop_layer_file(db, layer_repo, rel_path, existing),
        TextClassification::Text {
            content,
            indexed_len,
            truncated,
        } => (content, indexed_len, truncated),
    };

    let lang = rel_path
        .extension()
        .and_then(SupportedLanguage::from_extension);

    let extracted = match lang {
        Some(lang) => extract_file(layer_repo, rel_path.as_str(), &content, lang)
            .map_err(|e| layer_error(format!("Failed to extract {rel_path}: {e}")))?,
        None => ExtractedFile::default(),
    };

    let indexed_content = &content[..indexed_len];

    db.index_extracted_file(
        layer_repo,
        rel_path.as_str(),
        &hash,
        mtime_ns,
        size_bytes,
        indexed_content,
        truncated,
        &extracted,
    )
    .map_err(|e| layer_error(format!("Failed to index {rel_path}: {e}")))?;
    Ok(true)
}

fn drop_layer_file(
    db: &GraphDb,
    layer_repo: &str,
    rel_path: &Utf8Path,
    existing: Option<&FileRow>,
) -> Result<bool, Failure> {
    if existing.is_some() {
        db.delete_file(layer_repo, rel_path.as_str())
            .map_err(|e| layer_error(format!("Failed to drop layer file {rel_path}: {e}")))?;
    }
    Ok(false)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/layer.rs"]
mod tests;
