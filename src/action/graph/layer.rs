//! Layer delta build, fingerprint computation, per-feature locking, and layer indexing.

use camino::Utf8Path;
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::error::Failure;
use crate::git::{Git, System as GitSystem};
use crate::infra::graph::parser::SupportedLanguage;
use crate::store::graph::db::GraphDb;
use crate::store::graph::extractor::extract_file;
use crate::store::layout::Layout;

/// Result of ensuring a layer is indexed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerBuildResult {
    pub layer_id: i64,
    pub indexed_files: usize,
    pub tombstoned_files: usize,
    pub skipped: bool,
}

/// Computes a content-aware layer fingerprint combining base commit, head commit,
/// deleted files, and sha256 digests of modified/added files.
pub fn compute_layer_fingerprint(
    worktree: &Utf8Path,
    base_commit: &str,
) -> Result<String, Failure> {
    let git = GitSystem;
    let head = git.head_commit(worktree).map_err(|error| {
        Failure::failed(
            "graph.layer_error",
            format!("Git HEAD read failed: {error}"),
        )
    })?;
    let diff = git
        .diff_worktree_files(worktree, Some(base_commit))
        .map_err(|error| {
            Failure::failed("graph.layer_error", format!("Git diff failed: {error}"))
        })?;

    let mut hasher = Sha256::new();
    hasher.update(base_commit.as_bytes());
    hasher.update(head.as_bytes());

    for path in &diff.deleted {
        hasher.update(b"deleted\0");
        hasher.update(path.as_str().as_bytes());
    }

    for path in &diff.modified_or_added {
        hasher.update(b"changed\0");
        hasher.update(path.as_str().as_bytes());
        let full_path = worktree.join(path);
        if let Ok(bytes) = std::fs::read(&full_path) {
            hasher.update(Sha256::digest(bytes));
        }
    }
    use std::fmt::Write;
    let mut hex = String::with_capacity(32);
    for b in hasher.finalize().as_slice() {
        let _ = write!(hex, "{b:02x}");
    }
    Ok(hex)
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
    let _lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| {
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

    // 3. Diff worktree against base commit
    let diff = git
        .diff_worktree_files(worktree, Some(base_commit))
        .map_err(|e| Failure::failed("graph.layer_error", format!("Git diff error: {e}")))?;

    // 4. Content hashing detects changes even when mtime is preserved
    let fingerprint = compute_layer_fingerprint(worktree, base_commit)?;

    // 5. Check cache: if fingerprint matches existing record, skip build.
    if let Some(existing) = db.get_layer_record(feature, repo_name).map_err(|e| {
        Failure::failed(
            "graph.layer_error",
            format!("Failed to get layer record: {e}"),
        )
    })? && existing.fingerprint.as_deref() == Some(&fingerprint)
    {
        return Ok(LayerBuildResult {
            layer_id,
            indexed_files: 0,
            tombstoned_files: 0,
            skipped: true,
        });
    }

    // 6. Apply layer indexing: clear old layer rows in pseudo-repo
    let conn = db.conn();
    conn.execute(
        "DELETE FROM repos WHERE id = ?1",
        rusqlite::params![layer_repo],
    )
    .map_err(|e| {
        Failure::failed(
            "graph.layer_error",
            format!("Failed to delete layer repo: {e}"),
        )
    })?;
    db.insert_repo(
        &layer_repo,
        worktree.as_str(),
        feature,
        head_commit.as_deref(),
    )
    .map_err(|e| {
        Failure::failed(
            "graph.layer_error",
            format!("Failed to insert layer repo: {e}"),
        )
    })?;

    // Record tombstones
    let tombstone_strs: Vec<&str> = diff.deleted.iter().map(|p| p.as_str()).collect();
    db.set_layer_tombstones(layer_id, &tombstone_strs)
        .map_err(|e| {
            Failure::failed(
                "graph.layer_error",
                format!("Failed to set layer tombstones: {e}"),
            )
        })?;

    // Index modified or added files
    let mut indexed_count = 0;
    for rel_path in &diff.modified_or_added {
        let full_path = worktree.join(rel_path);
        if !full_path.exists() || !full_path.is_file() {
            continue;
        }

        let ext = Path::new(rel_path.as_str())
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let lang = match SupportedLanguage::from_extension(ext) {
            Some(l) => l,
            None => continue,
        };

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let meta = std::fs::metadata(&full_path).map_err(|e| {
            Failure::failed(
                "graph.layer_error",
                format!("Failed to read metadata for {rel_path}: {e}"),
            )
        })?;
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        let size_bytes = meta.len() as i64;

        let mut content_hasher = Sha256::new();
        content_hasher.update(content.as_bytes());
        let mut hash = String::with_capacity(32);
        for b in content_hasher.finalize().as_slice() {
            use std::fmt::Write;
            let _ = write!(hash, "{b:02x}");
        }
        let extracted =
            extract_file(&layer_repo, rel_path.as_str(), &content, lang).map_err(|e| {
                Failure::failed(
                    "graph.layer_error",
                    format!("Failed to extract {rel_path}: {e}"),
                )
            })?;

        db.index_extracted_file(
            &layer_repo,
            rel_path.as_str(),
            &hash,
            mtime_ns,
            size_bytes,
            &extracted,
        )
        .map_err(|e| {
            Failure::failed(
                "graph.layer_error",
                format!("Failed to index {rel_path}: {e}"),
            )
        })?;

        indexed_count += 1;
    }

    // 7. Update layer record fingerprint
    db.update_layer_fingerprint_and_head(layer_id, &fingerprint, head_commit.as_deref())
        .map_err(|e| {
            Failure::failed(
                "graph.layer_error",
                format!("Failed to update layer fingerprint: {e}"),
            )
        })?;

    Ok(LayerBuildResult {
        layer_id,
        indexed_files: indexed_count,
        tombstoned_files: tombstone_strs.len(),
        skipped: false,
    })
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/layer.rs"]
mod tests;
