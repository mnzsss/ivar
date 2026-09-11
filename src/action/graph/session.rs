//! Session view detection and resolution for the codebase graph.

use camino::{Utf8Path, Utf8PathBuf};

use crate::action::session::env::SessionEnv;
use crate::action::session::lookup;
use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::Failure;
use crate::infra::fs;
use crate::store::layout::Layout;

/// Information about a repository in a session view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoViewInfo {
    pub repo_name: String,
    pub worktree_path: Utf8PathBuf,
    pub is_layer: bool,
    pub base_commit: Option<String>,
}

/// Represents the active session view for codebase graph queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionView {
    Base {
        repos: Vec<RepoViewInfo>,
    },
    FeatureSession {
        feature_name: String,
        session_id: Option<String>,
        repos: Vec<RepoViewInfo>,
    },
}

/// Resolves the session view from the current working directory and environment.
pub fn resolve_session_view(layout: &Layout, cwd: &Utf8Path) -> Result<SessionView, Failure> {
    // 1. Check if we are inside a session environment (via hook/env)
    if let Ok(Some(env)) = SessionEnv::resolve_by_cwd(cwd)
        && let Some(feat_name) = env.feature
        && let Ok(Some(feature)) = Feature::read(layout, &feat_name)
    {
        return build_feature_session_view(layout, &feature, Some(env.session_id));
    }

    // 2. Check ambient environment variables IVAR_FEATURE or IVAR_SESSION_ID
    if let Ok(feat_str) = std::env::var("IVAR_FEATURE")
        && let Ok(feat_name) = FeatureName::new(&feat_str)
        && let Ok(Some(feature)) = Feature::read(layout, &feat_name)
    {
        let session_id = std::env::var("IVAR_SESSION_ID").ok();
        return build_feature_session_view(layout, &feature, session_id);
    }

    if let Ok(session_id) = std::env::var("IVAR_SESSION_ID")
        && let Ok(session) = lookup::resolve(layout, Some(&session_id), None)
        && let Some(feat_name) = session.feature.as_ref()
        && let Ok(Some(feature)) = Feature::read(layout, feat_name)
    {
        return build_feature_session_view(layout, &feature, Some(session_id));
    }

    // 3. Fall back to discovering feature from worktree path layout
    if let Some(feature) = detect_feature_from_worktree_path(layout, cwd) {
        return build_feature_session_view(layout, &feature, None);
    }

    // 4. Default: Base view
    build_base_session_view(layout)
}

fn detect_feature_from_worktree_path(layout: &Layout, cwd: &Utf8Path) -> Option<Feature> {
    let repos_dir = layout.repos_dir();
    if !cwd.starts_with(&repos_dir) {
        return None;
    }
    let rel = cwd.strip_prefix(&repos_dir).ok()?;
    let mut components = rel.components();
    let _repo_name = components.next()?.as_str();
    let branch_or_feat = components.next()?.as_str();
    if branch_or_feat == "main" || branch_or_feat == "master" {
        return None;
    }
    let feat_name = FeatureName::new(branch_or_feat).ok()?;
    Feature::read(layout, &feat_name).ok().flatten()
}

fn default_branch_fallback() -> Result<BranchName, Failure> {
    BranchName::new("main")
        .or_else(|_| BranchName::new("master"))
        .map_err(|e| {
            Failure::failed(
                "graph.session_error",
                format!("invalid default branch: {e}"),
            )
        })
}

fn build_feature_session_view(
    layout: &Layout,
    feature: &Feature,
    session_id: Option<String>,
) -> Result<SessionView, Failure> {
    let mut repos = Vec::new();
    let repos_dir = layout.repos_dir();
    if fs::is_dir(&repos_dir)? {
        for child in fs::read_dir(&repos_dir)? {
            if !fs::is_dir(&child)? {
                continue;
            }
            let Some(repo_str) = child.file_name() else {
                continue;
            };
            let Ok(repo_name) = RepoName::new(repo_str) else {
                continue;
            };

            if let Some(promotion) = feature.promotions.get(&repo_name) {
                let wt = layout.repo_worktree(&repo_name, &feature.branch);
                repos.push(RepoViewInfo {
                    repo_name: repo_str.to_owned(),
                    worktree_path: wt,
                    is_layer: true,
                    base_commit: promotion.base.as_ref().map(|b| b.as_str().to_owned()),
                });
            } else {
                let default_branch = default_branch_fallback()?;
                let wt = layout.repo_worktree(&repo_name, &default_branch);
                repos.push(RepoViewInfo {
                    repo_name: repo_str.to_owned(),
                    worktree_path: wt,
                    is_layer: false,
                    base_commit: None,
                });
            }
        }
    }
    repos.sort_by(|a, b| a.repo_name.cmp(&b.repo_name));
    Ok(SessionView::FeatureSession {
        feature_name: feature.name.to_string(),
        session_id,
        repos,
    })
}

fn build_base_session_view(layout: &Layout) -> Result<SessionView, Failure> {
    let mut repos = Vec::new();
    let repos_dir = layout.repos_dir();
    if fs::is_dir(&repos_dir)? {
        for child in fs::read_dir(&repos_dir)? {
            if !fs::is_dir(&child)? {
                continue;
            }
            let Some(repo_str) = child.file_name() else {
                continue;
            };
            let Ok(repo_name) = RepoName::new(repo_str) else {
                continue;
            };
            let default_branch = default_branch_fallback()?;
            let wt = layout.repo_worktree(&repo_name, &default_branch);
            repos.push(RepoViewInfo {
                repo_name: repo_str.to_owned(),
                worktree_path: wt,
                is_layer: false,
                base_commit: None,
            });
        }
    }
    repos.sort_by(|a, b| a.repo_name.cmp(&b.repo_name));
    Ok(SessionView::Base { repos })
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/session.rs"]
mod tests;
