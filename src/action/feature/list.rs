//! `ivar feature list` — every feature in the hall and how far it got.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{Feature, WorktreeState};
use crate::domain::name::FeatureName;
use crate::error::{Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use crate::action::Ctx;

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
                "  {}  branch {}  promoted {}/{}",
                feature.name, feature.branch, feature.ready_count, feature.promoted_count,
            )?;
        }
        Ok(())
    }
}

/// List every feature with a `feature.json` under `.ivar/features/`.
///
/// A feature whose record cannot be parsed is skipped with the rest still
/// listed — this is a status command, and one corrupt record should not hide
/// the others. (The skipped name is not reported here; `doctor` is where
/// that lives.)
pub fn list(ctx: &Ctx) -> Outcome<ListOutcome> {
    let layout = discover_hall(ctx)?;

    let features_dir = layout.ivar_dir().join("features");
    let mut features = Vec::new();
    if fs::is_dir(&features_dir)? {
        for entry in fs::read_dir(&features_dir)? {
            let name = match entry.file_name() {
                Some(name) => name.to_owned(),
                None => continue,
            };
            let Some(feature_name) = FeatureName::new(name.as_str()).ok() else {
                continue;
            };
            if let Ok(Some(feature)) = Feature::read(&layout, &feature_name) {
                features.push(summary_of(&feature));
            }
        }
    }
    features.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Report::new(ListOutcome {
        root: layout.root().to_path_buf(),
        features,
    }))
}

fn summary_of(feature: &Feature) -> FeatureSummary {
    FeatureSummary {
        name: feature.name.clone(),
        branch: feature.branch.to_string(),
        promoted_count: feature.promotions.len(),
        ready_count: feature.count_worktrees(WorktreeState::Ready),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/list.rs"]
mod tests;
