//! Freshness check and automatic layer refresh for graph database sessions.

use std::time::{Duration, Instant};

use crate::action::graph::layer::ensure_layer_indexed;
use crate::action::graph::session::{RepoViewInfo, SessionView};
use crate::action::graph::watch::worker::base_commit_for;
use crate::action::graph::watch::{LeaderState, WATCH_POLL, WATCH_TIMEOUT};
use crate::error::Failure;
use crate::store::graph::db::{GraphDb, GraphDbError};
use crate::store::layout::Layout;

fn freshness_error(err: &GraphDbError) -> Failure {
    Failure::failed(
        "graph.freshness_error",
        format!("Failed database operation during session freshness check: {err}"),
    )
}
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
            Failure::failed(
                "graph.freshness_error",
                format!("Failed to clear session layers: {e}"),
            )
        }),
        SessionView::FeatureSession {
            feature_name,
            repos,
            ..
        } => {
            let mut active = Vec::new();
            for repo in repos.iter().filter(|repo| repo.is_layer) {
                let base_commit = base_commit_for(db, &repo.repo_name, repo.base_commit.as_deref())
                    .ok_or_else(|| {
                        Failure::failed(
                            "graph.freshness_error",
                            format!("base graph index missing for repo `{}`", repo.repo_name),
                        )
                    })?;
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
                Failure::failed(
                    "graph.freshness_error",
                    format!("Failed to configure session mode: {e}"),
                )
            })?;
            Ok(())
        }
    }
}

/// Leader-aware session freshness check using default watch timeout.
pub fn ensure_session_freshness_watched(
    db: &GraphDb,
    layout: &Layout,
    view: &SessionView,
    leader: Option<LeaderState>,
) -> Result<(), Failure> {
    ensure_session_freshness_watched_within(db, layout, view, leader, WATCH_TIMEOUT)
}

pub(crate) fn ensure_session_freshness_watched_within(
    db: &GraphDb,
    layout: &Layout,
    view: &SessionView,
    leader: Option<LeaderState>,
    timeout: Duration,
) -> Result<(), Failure> {
    let SessionView::FeatureSession {
        feature_name,
        repos,
        ..
    } = view
    else {
        return ensure_session_freshness(db, layout, view);
    };
    if matches!(leader, None | Some(LeaderState::None)) {
        return ensure_session_freshness(db, layout, view);
    }
    let layers: Vec<&RepoViewInfo> = repos.iter().filter(|r| r.is_layer).collect();
    let keys: Vec<String> = layers
        .iter()
        .map(|r| format!("layer:{feature_name}:{}", r.repo_name))
        .collect();
    let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    if layers.is_empty() || !wait_settled(db, &refs, timeout)? {
        return ensure_session_freshness(db, layout, view);
    }
    let mut active = Vec::with_capacity(layers.len());
    for repo in &layers {
        let Some(record) = db
            .get_layer_record(feature_name, &repo.repo_name)
            .map_err(|e| freshness_error(&e))?
        else {
            return ensure_session_freshness(db, layout, view);
        };
        active.push((
            repo.repo_name.clone(),
            format!("{}/{}", repo.repo_name, record.id),
        ));
    }
    let pairs: Vec<(&str, &str)> = active
        .iter()
        .map(|(r, l)| (r.as_str(), l.as_str()))
        .collect();
    db.configure_session_mode(&pairs)
        .map_err(|e| freshness_error(&e))
}

pub(crate) fn wait_settled(
    db: &GraphDb,
    keys: &[&str],
    timeout: Duration,
) -> Result<bool, Failure> {
    let known = db.watch_scopes().map_err(|e| freshness_error(&e))?;
    if !keys.iter().all(|k| known.iter().any(|row| row.scope == *k)) {
        return Ok(false); // the leader does not watch this layer (yet): no wait
    }
    let deadline = Instant::now() + timeout;
    loop {
        if db.watch_settled(keys).map_err(|e| freshness_error(&e))? {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(WATCH_POLL);
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/freshness.rs"]
mod tests;
