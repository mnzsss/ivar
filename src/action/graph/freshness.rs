//! Freshness check and automatic layer refresh for graph database sessions.

use crate::error::Failure;
use crate::action::graph::layer::ensure_layer_indexed;
use crate::action::graph::session::SessionView;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;

/// Ensures the active session view is up-to-date in the graph database.
///
/// In `SessionView::Base`, clears the connection's session layers mapping.
/// In `SessionView::FeatureSession`, calls `ensure_layer_indexed` for each layer repo,
/// then configures the database session mode with the active layer IDs.
pub fn ensure_session_freshness(
    db: &GraphDb,
    layout: &Layout,
    view: &SessionView,
) -> Result<(), Failure> {
    match view {
        SessionView::Base { .. } => db.clear_session_layers().map_err(|e| {
            Failure::failed("graph.freshness_error", format!("Failed to clear session layers: {e}"))
        }),
        SessionView::FeatureSession {
            feature_name,
            repos,
            ..
        } => {
            let mut active = Vec::new();
            for repo in repos.iter().filter(|repo| repo.is_layer) {
                let base_commit = match db.get_repo_last_commit(&repo.repo_name).map_err(|e| {
                    Failure::failed("graph.freshness_error", format!("Failed to get repo last commit: {e}"))
                })? {
                    Some(commit) => commit,
                    None => match &repo.base_commit {
                        Some(commit) => commit.clone(),
                        None => {
                            return Err(Failure::failed(
                                "graph.freshness_error",
                                format!("base graph index missing for repo `{}`", repo.repo_name),
                            ));
                        }
                    },
                };
                let result = ensure_layer_indexed(
                    db,
                    layout,
                    feature_name,
                    &repo.repo_name,
                    &repo.worktree_path,
                    &base_commit,
                )?;
                active.push((
                    repo.repo_name.clone(),
                    format!("{}/{}", repo.repo_name, result.layer_id),
                ));
            }
            let refs: Vec<(&str, &str)> = active
                .iter()
                .map(|(repo, layer)| (repo.as_str(), layer.as_str()))
                .collect();
            db.configure_session_mode(&refs).map_err(|e| {
                Failure::failed("graph.freshness_error", format!("Failed to configure session mode: {e}"))
            })?;
            Ok(())
        }
    }
}
