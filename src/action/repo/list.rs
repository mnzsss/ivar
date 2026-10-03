//! `ivar repo list` — show every repo the hall knows about, and its state.
//!
//! Read-only. It looks at what `ivar.json` declares and what exists under
//! `.ivar/`, and never mutates either.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::Feature;
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::git::{self, TargetState};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Repo;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// One repo's observed state.
#[derive(Debug, Clone, Serialize)]
pub struct RepoStatus {
    /// The repo's name, as declared in `ivar.json`.
    pub name: RepoName,
    /// The git remote URL.
    pub url: String,
    /// The branch a fresh worktree defaults to.
    pub default_branch: String,
    /// Whether the bare clone exists under `.ivar/`.
    pub bare_cloned: bool,
    /// Whether the default-branch worktree exists.
    pub default_worktree: bool,
    /// Every feature with this repo promoted into it, sorted.
    pub features: Vec<FeatureName>,
}

/// What `ivar repo list` found.
#[derive(Debug, Clone, Serialize)]
pub struct ListOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// One entry per repo in `ivar.json`, in manifest order.
    pub repos: Vec<RepoStatus>,
}

impl WriteHuman for ListOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.repos.is_empty() {
            writeln!(w, "No repos in {}.", self.root)?;
            return Ok(());
        }
        writeln!(w, "Repos in {}:", self.root)?;
        let mut table =
            crate::infra::table::new(&["REPO", "CLONE", "BRANCH", "REMOTE", "FEATURES"]);
        for repo in &self.repos {
            let bare = if repo.bare_cloned {
                "cloned"
            } else {
                "missing"
            };
            let branch = if repo.default_worktree {
                repo.default_branch.clone()
            } else {
                format!("{} (no worktree)", repo.default_branch)
            };
            let features = if repo.features.is_empty() {
                "-".to_owned()
            } else {
                repo.features
                    .iter()
                    .map(FeatureName::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            table.add_row(vec![
                repo.name.as_str(),
                bare,
                &branch,
                &repo.url,
                &features,
            ]);
        }
        crate::infra::table::write(w, &table)
    }
}

/// List every repo declared in `ivar.json`, with its on-disk state and the
/// features it is promoted into.
///
/// A repo whose bare clone cannot be read (corrupt, or gone mid-listing)
/// reports `bare_cloned: false` rather than failing the whole listing, and a
/// feature record that cannot be parsed is left out of every repo's list —
/// this is a status command, and one broken entry should not hide the rest.
pub fn list(ctx: &Ctx) -> Outcome<ListOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;
    let features = read_features(&layout)?;

    let repos = manifest
        .repos()
        .iter()
        .map(|repo| {
            let mut status = status_of(&git, &layout, repo);
            status.features = features
                .iter()
                .filter(|feature| feature.promotions.contains_key(repo.name()))
                .map(|feature| feature.name.clone())
                .collect();
            status
        })
        .collect();

    Ok(Report::new(ListOutcome {
        root: layout.root().to_path_buf(),
        repos,
    }))
}

/// Every readable feature record in the hall, sorted by name. Unparseable
/// records are skipped, as `feature list` does.
fn read_features(layout: &Layout) -> Result<Vec<Feature>, Failure> {
    let features_dir = layout.features_dir();
    let mut features = Vec::new();
    if fs::is_dir(&features_dir)? {
        for entry in fs::read_dir(&features_dir)? {
            let Some(name) = entry.file_name() else {
                continue;
            };
            let Ok(feature_name) = FeatureName::new(name.to_owned()) else {
                continue;
            };
            if let Ok(Some(feature)) = Feature::read(layout, &feature_name) {
                features.push(feature);
            }
        }
    }
    features.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(features)
}

/// Observe one repo's on-disk state without letting any single probe fail
/// the listing. `features` is left empty; [`list`] fills it.
pub(super) fn status_of(git: &impl git::Git, layout: &Layout, repo: &Repo) -> RepoStatus {
    let bare = layout.repo_bare(repo.name());
    let worktree = layout.repo_worktree(repo.name(), repo.default_branch());

    let bare_state = git.target_state(&bare).unwrap_or(TargetState::Absent);
    let worktree_state = git.target_state(&worktree).unwrap_or(TargetState::Absent);

    RepoStatus {
        name: repo.name().clone(),
        url: repo.url().to_owned(),
        default_branch: repo.default_branch().to_string(),
        bare_cloned: matches!(bare_state, TargetState::Repository),
        default_worktree: matches!(worktree_state, TargetState::Repository),
        features: Vec::new(),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/repo/list.rs"]
mod tests;
