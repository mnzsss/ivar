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
//! that cannot be checked (its clone is missing, its repo left the manifest)
//! or cannot be torn down is kept, with the reason reported — never a whole
//! run abort. Teardown itself is delegated to `feature delete`, so the two
//! paths cannot drift.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::Feature;
use crate::domain::name::FeatureName;
use crate::error::{Failure, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, TargetState};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::{discover_hall, read_manifest};
use super::base;
use super::delete::{self as feature_delete, DeleteInput};
use crate::action::Ctx;
use crate::action::session::lookup as session_lookup;

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
                Err(failure) => {
                    warnings.push(Warning::new(
                        "feature.prune_delete_failed",
                        name.to_string(),
                        failure.what.clone(),
                    ));
                    kept.push(KeptFeature {
                        feature: name,
                        reason: format!("could not be deleted: {}", failure.what),
                    });
                }
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
    // The hard guard: never touch a feature with a live session.
    match session_lookup::list_feature(layout, &feature.name) {
        Ok(sessions) if !sessions.is_empty() => {
            return Verdict::Keep("has a live session".to_owned());
        }
        Err(error) => {
            return Verdict::Keep(format!("cannot check its sessions: {error}"));
        }
        _ => {}
    }

    // Nothing promoted — nothing to be unmerged.
    if feature.promotions.is_empty() {
        return Verdict::Prune;
    }

    for (repo, promotion) in &feature.promotions {
        let Some(manifest_repo) = manifest.repos().iter().find(|r| r.name() == repo) else {
            return Verdict::Keep(format!("repo `{repo}` is no longer in ivar.json"));
        };
        let effective_base = base::resolve(feature, promotion, manifest_repo.default_branch());
        let bare = layout.repo_bare(repo);
        if !matches!(git.target_state(&bare), Ok(TargetState::Repository)) {
            return Verdict::Keep(format!(
                "cannot check `{repo}` — its clone is missing (run `ivar sync`)"
            ));
        }
        match git.commits_ahead(&bare, effective_base.as_str(), feature.branch.as_str()) {
            Ok(0) => {}
            Ok(ahead) => {
                return Verdict::Keep(format!(
                    "`{repo}` has {ahead} commit(s) not merged into `{effective_base}`"
                ));
            }
            Err(error) => {
                return Verdict::Keep(format!("cannot check `{repo}`: {error}"));
            }
        }
    }

    Verdict::Prune
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
