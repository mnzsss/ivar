//! `ivar feature list` — every feature in the hall and how far it got.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{Feature, FeatureIntegrationState, WorktreeState};
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Outcome, Report, WriteHuman};
use crate::git::{self, Git};
use crate::infra::fs;

use super::super::{discover_hall, read_manifest};
use super::relations;
use crate::action::Ctx;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

/// One feature's summary.
#[derive(Debug, Clone, Serialize)]
pub struct FeatureSummary {
    /// The feature's name.
    pub name: FeatureName,
    /// The branch every promoted repo's worktree is on.
    pub branch: String,
    /// How many repos are promoted into it.
    pub promoted_count: usize,
    /// How many of those promotions are fully `Ready`.
    pub ready_count: usize,
    /// The feature's parent, if it is a subfeature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<FeatureName>,
    /// Its depth in the tree — 0 for a root.
    pub depth: usize,
    /// The derived integration state.
    pub state: FeatureIntegrationState,
    /// The names of descendants that block this feature's integration, each
    /// rendered as `name (state)`.
    pub blockers: Vec<String>,
    /// The promoted repos, in name order.
    pub repos: Vec<RepoName>,
}

/// What `ivar feature list` found.
#[derive(Debug, Clone, Serialize)]
pub struct ListOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// One entry per feature, sorted by name.
    pub features: Vec<FeatureSummary>,
}

impl WriteHuman for ListOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.features.is_empty() {
            writeln!(w, "No features in {}.", self.root)?;
            return Ok(());
        }
        writeln!(w, "Features in {}:", self.root)?;
        for feature in &self.features {
            writeln!(
                w,
                "  {}  branch {}  promoted {}/{}  state {}",
                feature.name,
                feature.branch,
                feature.ready_count,
                feature.promoted_count,
                feature.state,
            )?;
        }
        Ok(())
    }
}

/// List every feature with a `feature.json` under `.ivar/features/`, with its
/// derived tree position, integration state, and blockers.
///
/// A feature whose record cannot be parsed is skipped with the rest still
/// listed — this is a status command, and one corrupt record should not hide
/// the others. (The skipped name is not reported here; `doctor` is where
/// that lives.)
pub fn list(ctx: &Ctx) -> Outcome<ListOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let features_dir = layout.ivar_dir().join("features");
    let mut features = Vec::new();
    if fs::is_dir(&features_dir)? {
        for entry in fs::read_dir(&features_dir)? {
            let Some(name) = entry.file_name() else {
                continue;
            };
            let Some(feature_name) = FeatureName::new(name.to_owned()).ok() else {
                continue;
            };
            if let Ok(Some(feature)) = Feature::read(&layout, &feature_name)
                && let Some(summary) = summary_of(&git, &layout, &manifest, &feature)
            {
                features.push(summary);
            }
        }
    }
    features.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Report::new(ListOutcome {
        root: layout.root().to_path_buf(),
        features,
    }))
}

/// Build one feature's summary; `None` when its tree position cannot be
/// derived (a corrupt parent reference) — the per-feature skip the command
/// contract promises.
fn summary_of(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    feature: &Feature,
) -> Option<FeatureSummary> {
    let state = relations::feature_state(git, layout, manifest, feature).ok()?;
    let depth = relations::depth(layout, feature).ok()?;
    let blockers = relations::blocking_descendants(git, layout, manifest, feature)
        .ok()?
        .into_iter()
        .map(|entry| format!("{} ({})", entry.feature, entry.state))
        .collect();
    Some(FeatureSummary {
        name: feature.name.clone(),
        branch: feature.branch.to_string(),
        promoted_count: feature.promotions.len(),
        ready_count: feature.count_worktrees(WorktreeState::Ready),
        parent: feature.parent.clone(),
        depth,
        state,
        blockers,
        repos: feature.promotions.keys().cloned().collect(),
    })
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/list.rs"]
mod tests;
