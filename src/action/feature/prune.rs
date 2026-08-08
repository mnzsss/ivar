//! `ivar feature prune` — delete features whose branches are fully merged.
//!
//! A feature is prunable when **every** promoted repo's feature branch is
//! fully merged into that repo's default branch (no commits ahead), or when
//! no repo is promoted at all. It is never prunable while it has a **live
//! session** — a feature with an open session view dir is off-limits no
//! matter how merged its branches are, because the session is using it.
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

    for repo in feature.promotions.keys() {
        let Some(manifest_repo) = manifest.repos().iter().find(|r| r.name() == repo) else {
            return Verdict::Keep(format!("repo `{repo}` is no longer in ivar.json"));
        };
        let default_branch = manifest_repo.default_branch();
        let bare = layout.repo_bare(repo);
        if !matches!(git.target_state(&bare), Ok(TargetState::Repository)) {
            return Verdict::Keep(format!(
                "cannot check `{repo}` — its clone is missing (run `ivar sync`)"
            ));
        }
        match git.commits_ahead(&bare, default_branch.as_str(), feature.branch.as_str()) {
            Ok(0) => {}
            Ok(ahead) => {
                return Verdict::Keep(format!(
                    "`{repo}` has {ahead} commit(s) not merged into `{default_branch}`"
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
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::action::feature::create::CreateInput;
    use crate::action::feature::create::create as create_action;
    use crate::action::feature::promote::{self, PromoteInput};
    use crate::action::hall::{self, InitInput};
    use crate::domain::name::{BranchName, HallName, RepoName, SessionId};
    use crate::domain::provider::Provider;
    use crate::store::manifest::{Manifest, Providers, Repo};
    use crate::test_support::{git, hall_root, seeded_repo};

    /// A hall with one synced repo and one promoted feature (`checkout`) —
    /// its branch is off `main` with no new commits, so it is immediately
    /// merged.
    fn hall_with_promoted_feature() -> (tempfile::TempDir, Utf8PathBuf) {
        let (guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());
        hall::init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("."),
                name: Some("acme".to_owned()),
                provider: None,
            },
        )
        .unwrap();

        let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            vec![Repo::new(
                RepoName::new("api").unwrap(),
                origin.as_str(),
                BranchName::new("main").unwrap(),
            )],
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        create_action(
            &ctx,
            CreateInput {
                name: "checkout".to_owned(),
            },
        )
        .unwrap();
        crate::action::sync::sync(&ctx, Default::default()).unwrap();
        promote::promote(
            &ctx,
            PromoteInput {
                feature: "checkout".to_owned(),
                repo: "api".to_owned(),
            },
        )
        .unwrap();
        (guard, root)
    }

    fn feature_dir(root: &Utf8PathBuf) -> Utf8PathBuf {
        root.join(".ivar/features/checkout")
    }

    #[test]
    fn prune_deletes_a_feature_whose_branch_is_merged() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());

        let report = prune(&ctx).unwrap();

        assert!(report.is_clean());
        assert_eq!(
            report.value.pruned,
            vec![FeatureName::new("checkout").unwrap()]
        );
        assert!(report.value.kept.is_empty());
        assert!(
            !feature_dir(&root).exists(),
            "the feature directory is gone"
        );
        assert!(
            !root.join(".ivar/repos/api/checkout").exists(),
            "the worktree is gone"
        );
    }

    /// The hard guard: a feature with a live session is never touched, no
    /// matter how merged its branches are.
    #[test]
    fn prune_never_touches_a_feature_with_a_live_session() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());

        // A live session: its view dir exists under the feature's sessions.
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        let session = SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap();
        fs::ensure_dir(&layout.feature_session(&feature, &session)).unwrap();

        let report = prune(&ctx).unwrap();

        assert!(report.is_clean());
        assert!(report.value.pruned.is_empty());
        assert_eq!(report.value.kept.len(), 1);
        assert_eq!(report.value.kept[0].feature.as_str(), "checkout");
        assert!(report.value.kept[0].reason.contains("live session"));
        assert!(
            feature_dir(&root).join("feature.json").exists(),
            "a live-session feature must be left fully intact"
        );
        assert!(root.join(".ivar/repos/api/checkout").exists());
    }

    #[test]
    fn prune_keeps_a_feature_with_unmerged_commits() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());

        // A commit on the feature branch that `main` does not have.
        let worktree = root.join(".ivar/repos/api/checkout");
        std::fs::write(worktree.join("work.md"), "work\n").unwrap();
        git(&worktree, &["add", "work.md"]);
        git(&worktree, &["commit", "-m", "work"]);

        let report = prune(&ctx).unwrap();

        assert!(report.is_clean());
        assert!(report.value.pruned.is_empty());
        assert_eq!(report.value.kept.len(), 1);
        assert!(
            report.value.kept[0].reason.contains("not merged"),
            "reason was: {}",
            report.value.kept[0].reason
        );
        assert!(feature_dir(&root).join("feature.json").exists());
    }

    #[test]
    fn prune_keeps_a_feature_whose_clone_is_missing() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());
        // Delete the bare clone behind ivar's back — merge can no longer be
        // judged, so the feature must be kept, not pruned on a guess.
        fs::remove_path(&root.join(".ivar/repos/api/.bare")).unwrap();

        let report = prune(&ctx).unwrap();

        assert!(report.is_clean());
        assert!(report.value.pruned.is_empty());
        assert_eq!(report.value.kept.len(), 1);
        assert!(
            report.value.kept[0].reason.contains("cannot check"),
            "reason was: {}",
            report.value.kept[0].reason
        );
        assert!(feature_dir(&root).join("feature.json").exists());
    }

    #[test]
    fn the_human_surface_names_pruned_and_kept() {
        let outcome = PruneOutcome {
            root: Utf8PathBuf::from("/hall"),
            pruned: vec![FeatureName::new("checkout").unwrap()],
            kept: vec![KeptFeature {
                feature: FeatureName::new("checkout").unwrap(),
                reason: "has a live session".to_owned(),
            }],
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Pruned feature `checkout`.\nKept `checkout` — has a live session.\n"
        );
    }
}
