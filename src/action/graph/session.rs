//! Session view detection and resolution for the codebase graph.

use camino::{Utf8Path, Utf8PathBuf};

use crate::action::session::env::SessionEnv;
use crate::action::session::lookup;
use crate::domain::feature::Feature;
use crate::domain::name::{FeatureName, RepoName};
use crate::error::Failure;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Repo};

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

/// The ivar session id that keys usage and miss records: the session the cwd
/// resolves to, else the ambient `IVAR_SESSION_ID`. The guard and the MCP
/// server must agree on it, or misses never pair with graph calls.
pub(crate) fn resolve_session_key(cwd: &Utf8Path) -> Option<String> {
    session_key(cwd, std::env::var("IVAR_SESSION_ID").ok())
}

pub(crate) fn session_key(cwd: &Utf8Path, ambient: Option<String>) -> Option<String> {
    session_key_for(
        SessionEnv::resolve_by_cwd(cwd).ok().flatten().as_ref(),
        ambient,
    )
}

pub(crate) fn session_key_for(env: Option<&SessionEnv>, ambient: Option<String>) -> Option<String> {
    env.map(|env| env.session_id.clone()).or(ambient)
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
    let feat_name = FeatureName::new(branch_or_feat).ok()?;
    Feature::read(layout, &feat_name).ok().flatten()
}

fn mounted_repos(layout: &Layout) -> Result<Vec<RepoName>, Failure> {
    let repos_dir = layout.repos_dir();
    let mut names = Vec::new();
    if fs::is_dir(&repos_dir)? {
        for child in fs::read_dir(&repos_dir)? {
            if fs::is_dir(&child)?
                && let Some(Ok(name)) = child.file_name().map(RepoName::new)
            {
                names.push(name);
            }
        }
    }
    names.sort();
    Ok(names)
}

fn declared_repos(layout: &Layout) -> Result<Vec<Repo>, Failure> {
    let manifest = Manifest::read(layout)
        .map_err(|e| Failure::failed("graph.session_error", e.to_string()))?;
    Ok(manifest.map(|m| m.repos().to_vec()).unwrap_or_default())
}

fn base_repo_view(layout: &Layout, declared: &[Repo], name: &RepoName) -> Option<RepoViewInfo> {
    let repo = declared.iter().find(|repo| repo.name() == name)?;
    Some(RepoViewInfo {
        repo_name: name.to_string(),
        worktree_path: layout.repo_worktree(name, repo.default_branch()),
        is_layer: false,
        base_commit: None,
    })
}

fn build_feature_session_view(
    layout: &Layout,
    feature: &Feature,
    session_id: Option<String>,
) -> Result<SessionView, Failure> {
    let declared = declared_repos(layout)?;
    let repos = mounted_repos(layout)?
        .iter()
        .filter_map(|name| match feature.promotions.get(name) {
            Some(promotion) => Some(RepoViewInfo {
                repo_name: name.to_string(),
                worktree_path: layout.repo_worktree(name, &feature.branch),
                is_layer: true,
                base_commit: promotion.base.as_ref().map(|b| b.as_str().to_owned()),
            }),
            None => base_repo_view(layout, &declared, name),
        })
        .collect();
    Ok(SessionView::FeatureSession {
        feature_name: feature.name.to_string(),
        session_id,
        repos,
    })
}

fn build_base_session_view(layout: &Layout) -> Result<SessionView, Failure> {
    let declared = declared_repos(layout)?;
    let repos = mounted_repos(layout)?
        .iter()
        .filter_map(|name| base_repo_view(layout, &declared, name))
        .collect();
    Ok(SessionView::Base { repos })
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/session.rs"]
mod tests;
