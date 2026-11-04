//! `ivar feature cleanup` — build the side-effect-free cleanup preview.

use std::collections::BTreeMap;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::action::Ctx;
use crate::action::feature::delete;
use crate::action::session::lookup as session_lookup;
use crate::domain::feature::{
    BranchDeletion, CleanupApplyOutcome, CleanupBlocker, CleanupDescendant, CleanupDescendantFacts,
    CleanupDescendantOutcome, CleanupFacts, CleanupPreview, CleanupRecord, CleanupRepo,
    CleanupRepoFacts, Feature, ForgeDelivery, WorktreeRemoval, classify_cleanup,
    classify_descendant_cleanup,
};
use crate::domain::name::{FeatureName, RepoName, SessionId};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git, TargetState, WorktreeEntry};
use crate::infra::{fs, hash, json};

use super::super::{discover_hall, read_manifest};
use super::pull_requests::{self, PullRequest};
use super::{base, relations};

mod forge;
mod preview;
mod record;
mod teardown;

use self::forge::collect_repo_facts;
#[cfg(test)]
use self::forge::{ask_forge, read_forge_answer};
use self::preview::{PreviewedCleanup, WorktreeLookups, preview_cleanup};
#[cfg(test)]
use self::preview::{cleanup_repo, fingerprint_for};
use self::record::{
    load_and_validate_record, validate_record_against_preview, write_record_outcome,
};
use self::teardown::{
    preflight_deletable, remove_node_dir, teardown_branches, teardown_descendants,
    teardown_worktrees,
};

#[derive(Debug, Clone)]
pub struct CleanupInput {
    pub feature: String,
    pub preview: bool,
    pub record: Option<Utf8PathBuf>,
    /// The session running cleanup (`$IVAR_SESSION_ID`); never counted as a live-session blocker.
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanupOutcome {
    pub root: Utf8PathBuf,
    pub preview: CleanupPreview,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_outcome: Option<CleanupApplyOutcome>,
}

impl WriteHuman for CleanupOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if let Some(apply) = &self.apply_outcome {
            if apply.feature_removed {
                writeln!(w, "Cleaned up feature `{}` in {}", apply.feature, self.root)?;
            } else {
                writeln!(
                    w,
                    "Cleaned up feature `{}` in {} — partially; record kept for retry",
                    apply.feature, self.root
                )?;
                for removal in &apply.worktrees {
                    if !removal.removed {
                        writeln!(w, "  {}: worktree not removed", removal.repo)?;
                    }
                }
                for deletion in &apply.branches {
                    if !deletion.deleted {
                        writeln!(w, "  {}: local branch not deleted", deletion.repo)?;
                    }
                }
            }
            Ok(())
        } else {
            writeln!(w, "Cleanup preview for feature `{}`:", self.preview.feature)?;
            for repo in &self.preview.repos {
                let state = if repo.is_delivered {
                    "delivered"
                } else {
                    "not delivered"
                };
                writeln!(w, "  `{}`: {state}", repo.repo)?;
            }
            if self.preview.blockers.is_empty() {
                writeln!(w, "Eligible for cleanup.")?;
            } else {
                writeln!(w, "Blocked by:")?;
                for blocker in &self.preview.blockers {
                    writeln!(w, "  {blocker:?}")?;
                }
            }
            if !self.preview.paths_to_remove.is_empty() {
                writeln!(w, "Paths to remove:")?;
                for path in &self.preview.paths_to_remove {
                    writeln!(w, "  {path}")?;
                }
            }
            writeln!(w, "Fingerprint: {}", self.preview.fingerprint)
        }
    }
}

pub fn cleanup(ctx: &Ctx, input: CleanupInput) -> Outcome<CleanupOutcome> {
    if !input.preview {
        let Some(record_path) = &input.record else {
            return Err(Failure::blocked(
                "feature.cleanup_record_required",
                "cleanup apply requires `--record <path>`",
            ));
        };

        return apply_cleanup(
            ctx,
            &input.feature,
            record_path,
            input.session_id.as_deref(),
        );
    }

    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let name = FeatureName::new(input.feature)?;
    let feature = Feature::read(&layout, &name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
    })?;
    let git = git::System;
    let previewed = preview_cleanup(
        &git,
        &layout,
        &manifest,
        &feature,
        input.session_id.as_deref(),
    )?;

    Ok(Report::new(CleanupOutcome {
        root: layout.root().to_path_buf(),
        preview: previewed.preview,
        apply_outcome: None,
    }))
}

fn apply_cleanup(
    ctx: &Ctx,
    feature_arg: &str,
    record_path: &Utf8Path,
    own_session: Option<&str>,
) -> Result<Report<CleanupOutcome>, Failure> {
    let layout = discover_hall(ctx)?;
    let record = load_and_validate_record(record_path, &layout)?;

    let manifest = read_manifest(&layout)?;
    let name = FeatureName::new(feature_arg.to_owned())?;
    let feature = Feature::read(&layout, &name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
    })?;
    let git = git::System;
    let PreviewedCleanup {
        preview,
        forge_consulted,
        worktrees,
        descendants,
    } = preview_cleanup(&git, &layout, &manifest, &feature, own_session)?;

    validate_record_against_preview(&record, &preview, forge_consulted)?;
    preflight_deletable(&layout, &feature, &descendants)?;

    let (descendant_outcomes, mut warnings, all_descendants_removed) =
        teardown_descendants(&layout, &git, &descendants)?;
    if !all_descendants_removed {
        let apply_outcome = CleanupApplyOutcome {
            feature: feature.name.clone(),
            branch: feature.branch,
            fingerprint: preview.fingerprint.clone(),
            worktrees: Vec::new(),
            branches: Vec::new(),
            feature_removed: false,
            descendants: descendant_outcomes,
        };
        return Ok(Report::with_warnings(
            CleanupOutcome {
                root: layout.root().to_path_buf(),
                preview,
                apply_outcome: Some(apply_outcome),
            },
            warnings,
        ));
    }

    let (worktree_removals, root_warnings, all_worktrees_removed) =
        teardown_worktrees(&layout, &git, &feature, &worktrees)?;
    let (branch_deletions, branch_warnings, all_branches_deleted) =
        teardown_branches(&layout, &git, &feature, &worktree_removals);
    warnings.extend(root_warnings);
    warnings.extend(branch_warnings);

    let complete_success = all_worktrees_removed && all_branches_deleted;
    if !complete_success {
        let apply_outcome = CleanupApplyOutcome {
            feature: feature.name.clone(),
            branch: feature.branch,
            fingerprint: preview.fingerprint.clone(),
            worktrees: worktree_removals,
            branches: branch_deletions,
            feature_removed: false,
            descendants: descendant_outcomes,
        };
        return Ok(Report::with_warnings(
            CleanupOutcome {
                root: layout.root().to_path_buf(),
                preview,
                apply_outcome: Some(apply_outcome),
            },
            warnings,
        ));
    }

    // Complete success: remove feature directory, then update durable record outcome
    if let Some(warning) = remove_node_dir(&layout, &feature.name)? {
        warnings.push(warning);
    }

    let apply_outcome = CleanupApplyOutcome {
        feature: feature.name.clone(),
        branch: feature.branch,
        fingerprint: preview.fingerprint.clone(),
        worktrees: worktree_removals,
        branches: branch_deletions,
        feature_removed: true,
        descendants: descendant_outcomes,
    };

    write_record_outcome(&layout, record, record_path, &apply_outcome)?;

    Ok(Report::with_warnings(
        CleanupOutcome {
            root: layout.root().to_path_buf(),
            preview,
            apply_outcome: Some(apply_outcome),
        },
        warnings,
    ))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/cleanup.rs"]
mod tests;
