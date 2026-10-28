//! Command-level checks on the hall directories a user asks the graph to index.

use super::index::HALL_REPO;
use crate::error::Failure;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, normalise_graph_path};

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
    let skills_dir = crate::infra::fs::canonicalize(&layout.hall_skills())
        .unwrap_or_else(|_| layout.hall_skills());
    if canonical.starts_with(&ivar_dir) && !canonical.starts_with(&skills_dir) {
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
                format!("`{dir}` is inside `.ivar`, which is never indexed except `.ivar/skills`"),
            )
        });
    }
    Ok(dir)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/hall_paths.rs"]
mod tests;
