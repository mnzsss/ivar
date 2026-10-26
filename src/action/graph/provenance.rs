//! Which commit each graph answer came from, and whether its checkout has moved on.
//!
//! The index records the commit it was built at. An agent that cannot see that commit
//! cannot tell a stale answer from a current one, so every MCP answer carries it, with
//! a warning when the checkout's HEAD differs or the checkout trails its upstream.

use std::fmt::Write as _;

use crate::action::graph::session::{RepoViewInfo, SessionView};
use crate::git::Git;
use crate::store::graph::db::GraphDb;

/// Commit id characters shown in a notice.
const SHORT_LEN: usize = 7;

/// The commit one repo's graph was indexed at, next to the state of its checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoIndexState {
    pub repo: String,
    /// The commit the index (base index, or feature layer) records, if any.
    pub indexed: Option<String>,
    /// HEAD of the checkout the answer is served for, if it could be read.
    pub head: Option<String>,
    /// How many commits the checkout lacks from its upstream, as of the last fetch.
    pub behind_upstream: usize,
}

impl RepoIndexState {
    fn is_behind_head(&self) -> bool {
        matches!((&self.indexed, &self.head), (Some(indexed), Some(head)) if indexed != head)
    }
}

/// The notice for the repos `view` serves: the indexed commit of each, plus a warning
/// per repo whose index lags its checkout or whose checkout lags its upstream.
///
/// `None` when the view has no repos to report on.
pub fn index_notice(db: &GraphDb, git: &dyn Git, view: &SessionView) -> Option<String> {
    render(&collect(db, git, view))
}

/// Reads the indexed commit and checkout state of every repo in `view`.
pub fn collect(db: &GraphDb, git: &dyn Git, view: &SessionView) -> Vec<RepoIndexState> {
    let (feature, repos) = match view {
        SessionView::Base { repos } => (None, repos),
        SessionView::FeatureSession {
            feature_name,
            repos,
            ..
        } => (Some(feature_name.as_str()), repos),
    };
    repos
        .iter()
        .map(|repo| repo_state(db, git, feature, repo))
        .collect()
}

fn repo_state(
    db: &GraphDb,
    git: &dyn Git,
    feature: Option<&str>,
    repo: &RepoViewInfo,
) -> RepoIndexState {
    let indexed = match feature {
        Some(feature) if repo.is_layer => db
            .get_layer_record(feature, &repo.repo_name)
            .ok()
            .flatten()
            .and_then(|record| record.head_commit),
        _ => db.get_repo_last_commit(&repo.repo_name).ok().flatten(),
    };
    let path = repo.worktree_path.as_path();
    let head = git.revision_commit(path, "HEAD").ok();
    let behind_upstream = head
        .as_deref()
        .and_then(|head| {
            let upstream = git.revision_commit(path, "@{upstream}").ok()?;
            if upstream == head {
                return Some(0);
            }
            git.divergence(path, head, &upstream)
                .ok()
                .map(|d| d.remote_only.len())
        })
        .unwrap_or(0);
    RepoIndexState {
        repo: repo.repo_name.clone(),
        indexed,
        head,
        behind_upstream,
    }
}

fn short(commit: &str) -> &str {
    commit.get(..SHORT_LEN).unwrap_or(commit)
}

/// Renders the notice: a line naming every indexed commit, then one warning per problem.
pub fn render(repos: &[RepoIndexState]) -> Option<String> {
    if repos.is_empty() {
        return None;
    }
    let commits: Vec<String> = repos
        .iter()
        .map(|state| match &state.indexed {
            Some(commit) => format!("{}@{}", state.repo, short(commit)),
            None => format!("{}@none", state.repo),
        })
        .collect();
    let mut notice = format!("Graph index commits: {}.", commits.join(", "));
    for state in repos {
        let repo = &state.repo;
        match (&state.indexed, &state.head) {
            (None, Some(head)) => {
                let _ = write!(
                    notice,
                    "\nWarning: `{repo}` has no indexed commit (checkout HEAD is {}), so its \
                     answers may be missing or out of date. Call `refresh_index` for it.",
                    short(head)
                );
            }
            (Some(indexed), Some(head)) if state.is_behind_head() => {
                let _ = write!(
                    notice,
                    "\nWarning: `{repo}` index is at {} but its checkout HEAD is {}, so this \
                     answer may describe older code. Call `refresh_index` for it, or Read the \
                     files directly.",
                    short(indexed),
                    short(head)
                );
            }
            _ => {}
        }
        if state.behind_upstream > 0 {
            let _ = write!(
                notice,
                "\nWarning: `{repo}` checkout is {} commit{} behind its upstream as of the \
                 last fetch, so work already merged there is not in this answer. Fetch and \
                 update the checkout, then call `refresh_index`.",
                state.behind_upstream,
                if state.behind_upstream == 1 { "" } else { "s" }
            );
        }
    }
    Some(notice)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/provenance.rs"]
mod tests;
