//! Hall-root directories indexed on request: one graph repo, `hall`, fed by a
//! filesystem walk rather than git, so untracked folders are first-class.

use std::path::Path;

use walkdir::WalkDir;

use super::{
    IndexError, IndexOutcome, build_extract_jobs, is_ignored_dir, is_ignored_file_path,
    run_extraction_jobs,
};
use crate::action::progress::Progress;
use crate::error::Failure;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, normalise_graph_path};

/// Slash-free on purpose: the base views treat any repo id containing `/` as a layer.
pub const HALL_REPO: &str = "hall";

/// # Errors
///
/// Returns [`IndexError::Walk`] when a directory cannot be read.
pub fn walk_hall_dir(hall_root: &Path, dir: &str) -> Result<Vec<String>, IndexError> {
    let base = hall_root.join(dir);
    let mut files = Vec::new();
    let walker = WalkDir::new(&base).follow_links(false).into_iter();
    for entry in walker.filter_entry(|e| !(e.file_type().is_dir() && is_ignored_dir(e.file_name())))
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(hall_root).unwrap_or(entry.path());
        if is_ignored_file_path(rel) {
            continue;
        }
        files.push(rel.to_string_lossy().replace('\\', "/"));
    }
    files.sort();
    Ok(files)
}

/// # Errors
///
/// Returns [`IndexError`] when the walk, extraction or a database write fails.
pub fn index_hall_path(
    db: &GraphDb,
    hall_root: &Path,
    dir: &str,
    force_full: bool,
    progress: &dyn Progress,
) -> Result<IndexOutcome, IndexError> {
    let start = std::time::Instant::now();
    db.insert_repo(HALL_REPO, &hall_root.to_string_lossy(), "", None)?;
    let files_to_index = walk_hall_dir(hall_root, dir)?;
    let prefix = format!("{}/", dir.trim_end_matches('/'));
    let files_to_delete: Vec<String> = db
        .get_files_for_repo(HALL_REPO)?
        .into_iter()
        .map(|f| f.path)
        .filter(|p| p.starts_with(&prefix) && !files_to_index.contains(p))
        .collect();
    let (jobs, mut files_failed) =
        build_extract_jobs(db, HALL_REPO, hall_root, &files_to_index, force_full, false)?;
    let (files_indexed, symbols_indexed, edges_indexed, job_failures) =
        run_extraction_jobs(&jobs, HALL_REPO, db, progress, &files_to_delete)?;
    files_failed.extend(job_failures);
    Ok(IndexOutcome {
        repo: HALL_REPO.to_owned(),
        files_indexed,
        files_deleted: files_to_delete.len(),
        symbols_indexed,
        edges_indexed,
        duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        skipped_up_to_date: false,
        files_failed,
    })
}

/// # Errors
///
/// Returns a blocked [`Failure`] naming the rule the path breaks.
pub fn validate_hall_path(
    layout: &Layout,
    manifest: &Manifest,
    raw: &str,
) -> Result<String, Failure> {
    if manifest
        .repos()
        .iter()
        .any(|r| r.name().as_str() == HALL_REPO)
    {
        return Err(Failure::blocked(
            "graph.path_repo_clash",
            "the graph keeps hall paths under the repo id `hall`, and this manifest declares a repo named `hall`",
        ));
    }
    let dir = normalise_graph_path(raw)
        .map_err(|e| Failure::blocked("graph.path_invalid", e.to_string()))?;
    let candidate = layout.root().join(&dir);
    let canonical = crate::infra::fs::canonicalize(&candidate).map_err(|_| {
        Failure::blocked(
            "graph.path_missing",
            format!("`{dir}` does not exist under the hall root"),
        )
    })?;
    if !canonical.is_dir() {
        return Err(Failure::blocked(
            "graph.path_missing",
            format!("`{dir}` is not a directory"),
        ));
    }
    let root = crate::infra::fs::canonicalize(layout.root())
        .unwrap_or_else(|_| layout.root().to_path_buf());
    if !canonical.starts_with(&root) {
        return Err(Failure::blocked(
            "graph.path_outside_hall",
            format!("`{dir}` resolves outside the hall root"),
        ));
    }
    let ivar_dir =
        crate::infra::fs::canonicalize(&layout.ivar_dir()).unwrap_or_else(|_| layout.ivar_dir());
    if canonical.starts_with(&ivar_dir) {
        let in_repo = manifest.repos().iter().any(|r| {
            let repo_dir = crate::infra::fs::canonicalize(&layout.repo_dir(r.name()))
                .unwrap_or_else(|_| layout.repo_dir(r.name()));
            canonical.starts_with(&repo_dir)
        });
        return Err(if in_repo {
            Failure::blocked(
                "graph.path_in_repo",
                format!(
                    "`{dir}` is inside a mounted repo; index it with `ivar graph index --repo`"
                ),
            )
        } else {
            Failure::blocked(
                "graph.path_in_ivar",
                format!("`{dir}` is inside `.ivar`, which is never indexed"),
            )
        });
    }
    Ok(dir)
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/hall_path.rs"]
mod tests;
