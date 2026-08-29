//! `ivar feature prune` — delete features whose branches are fully merged.
//!
//! A feature is prunable when **every** promoted repo's feature branch is
//! fully merged into that repo's effective base (no commits ahead) — the
//! base `promote` recorded for that repo, or, absent a recorded base, the
//! repo's default branch — or when no repo is promoted at all. A feature
//! merged into its base is prunable even while that base itself is still
//! open: mergedness is measured against what the feature actually branched
//! from, not against whichever default branch a repo happens to declare. It
//! is never prunable while it has a **live session** — a feature with an
//! open session view dir is off-limits no matter how merged its branches
//! are, because the session is using it.
//!
//! Pruning is best-effort per feature, like every batch verb here: a feature
//! that cannot be judged (its clone is missing, its repo left the manifest)
//! or cannot be torn down is kept, with the reason reported — never a whole
//! run abort. Teardown itself is delegated to `feature delete`, so the two
//! paths cannot drift.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::action::feature::base;
use crate::action::feature::delete::{self as feature_delete, DeleteInput};
use crate::action::session::lookup as session_lookup;
use crate::domain::feature::{
    DeliveryFacts, DeliveryRepoFacts, DeliveryVerdict, Feature, classify_delivery,
};
use crate::domain::name::FeatureName;
use crate::error::{Failure, Outcome, Report, Status, WriteHuman};
use crate::git::{self, TargetState};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::{discover_hall, read_manifest};

/// Why one feature was kept.
#[derive(Debug, Clone, Serialize)]
pub struct KeptFeature {
    /// The feature that was not pruned.
    pub feature: FeatureName,
    /// Why — a live session, unmerged branches, or a check that could not
    /// complete.
    pub reason: String,
}

/// What `ivar feature prune` did.
#[derive(Debug, Clone, Serialize)]
pub struct PruneOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The features that were deleted, in name order.
    pub pruned: Vec<FeatureName>,
    /// The features that were kept, with the reason each was kept, in name
    /// order.
    pub kept: Vec<KeptFeature>,
}

impl WriteHuman for PruneOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.pruned.is_empty() {
            writeln!(w, "No features to prune in {}.", self.root)?;
        } else {
            for feature in &self.pruned {
                writeln!(w, "Pruned feature `{feature}`.")?;
            }
        }
        for kept in &self.kept {
            writeln!(w, "Kept `{}` — {}.", kept.feature, kept.reason)?;
        }
        Ok(())
    }
}

/// Delete every feature whose branches are fully merged, skipping any with a
/// live session.
///
/// A feature that cannot be judged (a promoted repo whose clone is missing,
/// or that left the manifest) or torn down is kept with the reason reported —
/// one bad feature never hides the rest.
pub fn prune(ctx: &Ctx) -> Outcome<PruneOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;
    let mut pruned = Vec::new();
    let mut kept = Vec::new();
    let mut warnings = Vec::new();

    for feature in list_features(&layout)? {
        let name = feature.name.clone();
        match classify(&git, &layout, &manifest, &feature) {
            Verdict::Keep(reason) => kept.push(KeptFeature {
                feature: name,
                reason,
            }),
            Verdict::Prune => match feature_delete::delete(
                ctx,
                DeleteInput {
                    name: name.to_string(),
                },
            ) {
                Ok(report) => {
                    pruned.push(name);
                    warnings.extend(report.warnings);
                }
                Err(failure) if failure.status == Status::Blocked => {
                    kept.push(KeptFeature {
                        feature: name,
                        reason: failure.what,
                    });
                }
                Err(failure) => return Err(failure),
            },
        }
    }

    pruned.sort();
    kept.sort_by(|a, b| a.feature.cmp(&b.feature));

    Ok(Report::with_warnings(
        PruneOutcome {
            root: layout.root().to_path_buf(),
            pruned,
            kept,
        },
        warnings,
    ))
}

/// Whether `feature` may be pruned, and if not, why.
fn classify(
    git: &impl git::Git,
    layout: &Layout,
    manifest: &Manifest,
    feature: &Feature,
) -> Verdict {
    let (live_sessions, session_inspection_error) =
        match session_lookup::list_feature(layout, &feature.name) {
            Ok(sessions) => (
                sessions.into_iter().map(|session| session.id).collect(),
                None,
            ),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };

    let repo_facts: Vec<_> = feature
        .promotions
        .iter()
        .map(|(repo, promotion)| {
            let Some(manifest_repo) = manifest.repos().iter().find(|r| r.name() == repo) else {
                return DeliveryRepoFacts {
                    repo: repo.clone(),
                    effective_base: None,
                    clone_exists: false,
                    unmerged_commits: None,
                    in_manifest: false,
                    inspection_error: None,
                };
            };
            let effective_base = base::resolve(feature, promotion, manifest_repo.default_branch());
            let bare = layout.repo_bare(repo);
            let clone_exists = matches!(git.target_state(&bare), Ok(TargetState::Repository));
            let mut inspection_error = None;
            let unmerged_commits = if clone_exists {
                match git.commits_ahead(&bare, effective_base.as_str(), feature.branch.as_str()) {
                    Ok(ahead) => Some(ahead),
                    Err(error) => {
                        inspection_error = Some(error.to_string());
                        None
                    }
                }
            } else {
                None
            };
            DeliveryRepoFacts {
                repo: repo.clone(),
                effective_base: Some(effective_base),
                clone_exists,
                unmerged_commits,
                in_manifest: true,
                inspection_error,
            }
        })
        .collect();

    let delivery_facts = DeliveryFacts {
        repos: repo_facts,
        live_sessions,
        session_inspection_error,
    };

    match classify_delivery(&delivery_facts) {
        DeliveryVerdict::Delivered => Verdict::Prune,
        DeliveryVerdict::Blocked(blockers) => {
            let reason = blockers
                .first()
                .map(|b| b.to_string())
                .unwrap_or_else(|| "feature is kept".to_owned());
            Verdict::Keep(reason)
        }
    }
}

/// What to do with one feature.
enum Verdict {
    /// It can be deleted.
    Prune,
    /// It cannot — with the reason.
    Keep(String),
}

/// Every feature with a readable `feature.json`, in name order. A feature
/// whose record cannot be parsed is skipped with the rest still listed —
/// this is a batch verb, and one corrupt record should not hide the others.
fn list_features(layout: &Layout) -> Result<Vec<Feature>, Failure> {
    let features_dir = layout.features_dir();
    let mut features = Vec::new();

    if fs::is_dir(&features_dir)? {
        for entry in fs::read_dir(&features_dir)? {
            let Some(name) = entry.file_name() else {
                continue;
            };
            let Ok(feature_name) = FeatureName::new(name) else {
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

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/prune.rs"]
mod tests;
