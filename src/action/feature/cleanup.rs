//! `ivar feature cleanup` — build the side-effect-free cleanup preview.

use std::collections::BTreeMap;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::action::Ctx;
use crate::action::feature::delete;
use crate::action::memory::delivery::Emitter;
use crate::action::memory::review::{ReviewState, review_state};
use crate::action::session::lookup as session_lookup;
use crate::domain::delivery_event::{DeliveryEvent, DeliveryScope, ObservedBy};
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
use crate::store::layout::Layout;

use super::super::{discover_hall, read_manifest};
use super::pull_requests::{self, PullRequest};
use super::{base, relations};

mod forge;
mod preview;
mod record;
mod teardown;

use self::forge::collect_repo_facts;
#[cfg(test)]
use self::forge::{merged_pull_request, read_forge_answer};
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
    let apply_record = (!input.preview)
        .then(|| {
            input.record.as_ref().ok_or_else(|| {
                Failure::blocked(
                    "feature.cleanup_record_required",
                    "cleanup apply requires `--record <path>`",
                )
            })
        })
        .transpose()?;

    let layout = discover_hall(ctx)?;
    let emitter = ctx.emitter(&layout);
    if let Some(record_path) = apply_record {
        return apply_cleanup(
            &layout,
            &emitter,
            &input.feature,
            record_path,
            input.session_id.as_deref(),
        );
    }

    let manifest = read_manifest(&layout)?;
    let name = FeatureName::new(input.feature)?;
    let feature = Feature::read(&layout, &name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
    })?;
    let git = git::System;
    // Before the preview: it reads the review of a feature with no repo.
    let (_, memory_warning) = memory_review(&layout, &name);
    let previewed = preview_cleanup(
        &git,
        &layout,
        &manifest,
        &feature,
        input.session_id.as_deref(),
    )?;
    emit_merged(&emitter, &feature, &previewed.merged);

    Ok(Report::with_warnings(
        CleanupOutcome {
            root: layout.root().to_path_buf(),
            preview: previewed.preview,
            apply_outcome: None,
        },
        memory_warning,
    ))
}

/// What the memory review of `name` says for `delivery.cleaned`, and the
/// warnings when the spool cannot be drained or the feature was neither
/// reviewed nor skipped. Never refuses.
fn memory_review(layout: &Layout, name: &FeatureName) -> (&'static str, Vec<Warning>) {
    let mut warnings = Vec::new();
    if let Err(failure) = crate::action::memory::drain::drain(layout) {
        warnings.push(Warning::new(
            "cleanup.memory_drain_failed",
            name.to_string(),
            format!(
                "the capture spool could not be drained, so a recent review may be missed: {}",
                failure.what
            ),
        ));
    }
    let cause = match review_state(layout, name) {
        Ok(ReviewState::Reviewed { .. }) => return ("reviewed", warnings),
        Ok(ReviewState::Skipped) => return ("skipped", warnings),
        Ok(ReviewState::Missing) => String::new(),
        Err(failure) => format!(" (memory could not be read: {})", failure.what),
    };
    warnings.push(Warning::new(
        "cleanup.memory_not_reviewed",
        name.to_string(),
        format!(
            "`{name}` has neither `memory.reviewed` nor `memory.review_skipped`: its memory was not reviewed before delivery{cause}"
        ),
    ));
    ("missing", warnings)
}

fn emit_merged(emitter: &Emitter<'_>, feature: &Feature, merged: &[(RepoName, PullRequest)]) {
    for (repo, pr) in merged {
        emitter.emit(
            DeliveryScope::for_repo(
                feature,
                repo.as_str(),
                pr.head_oid.as_deref().unwrap_or_default(),
            ),
            pull_requests::merged_event(pr, ObservedBy::Cleanup),
        );
    }
}

fn emit_cleaned(
    emitter: &Emitter<'_>,
    feature: &Feature,
    record_path: &Utf8Path,
    docs: &str,
    outcome: &CleanupApplyOutcome,
    descendants: &[(Feature, WorktreeLookups)],
) {
    let cleaned = || DeliveryEvent::Cleaned {
        record_path: record_path.to_string(),
        docs: docs.to_owned(),
    };
    emitter.emit(DeliveryScope::for_feature(feature), cleaned());
    for node in outcome
        .descendants
        .iter()
        .filter(|node| node.feature_removed)
    {
        if let Some((descendant, _)) = descendants.iter().find(|(f, _)| f.name == node.feature) {
            emitter.emit(DeliveryScope::for_feature(descendant), cleaned());
        }
    }
}

fn apply_cleanup(
    layout: &Layout,
    emitter: &Emitter<'_>,
    feature_arg: &str,
    record_path: &Utf8Path,
    own_session: Option<&str>,
) -> Result<Report<CleanupOutcome>, Failure> {
    let record = load_and_validate_record(record_path, layout)?;

    let manifest = read_manifest(layout)?;
    let name = FeatureName::new(feature_arg.to_owned())?;
    let feature = Feature::read(layout, &name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
    })?;
    let (review, memory_warning) = memory_review(layout, &name);
    let git = git::System;
    let PreviewedCleanup {
        preview,
        forge_consulted,
        worktrees,
        descendants,
        merged,
    } = preview_cleanup(&git, layout, &manifest, &feature, own_session)?;
    emit_merged(emitter, &feature, &merged);

    validate_record_against_preview(&record, &preview, forge_consulted)?;
    preflight_deletable(layout, &feature, &descendants)?;

    let (descendant_outcomes, mut warnings, all_descendants_removed) =
        teardown_descendants(layout, &git, &descendants)?;
    warnings.extend(memory_warning);
    if !all_descendants_removed {
        return Ok(partial_cleanup(
            layout,
            feature,
            preview,
            (Vec::new(), Vec::new()),
            descendant_outcomes,
            warnings,
        ));
    }

    let (worktree_removals, root_warnings, all_worktrees_removed) =
        teardown_worktrees(layout, &git, &feature, &worktrees)?;
    let (branch_deletions, branch_warnings, all_branches_deleted) =
        teardown_branches(layout, &git, &feature, &worktree_removals);
    warnings.extend(root_warnings);
    warnings.extend(branch_warnings);

    let complete_success = all_worktrees_removed && all_branches_deleted;
    if !complete_success {
        return Ok(partial_cleanup(
            layout,
            feature,
            preview,
            (worktree_removals, branch_deletions),
            descendant_outcomes,
            warnings,
        ));
    }

    // Complete success: remove feature directory, then update durable record outcome
    if let Some(warning) = remove_node_dir(layout, &feature.name)? {
        warnings.push(warning);
    }

    let apply_outcome = CleanupApplyOutcome {
        feature: feature.name.clone(),
        branch: feature.branch.clone(),
        fingerprint: preview.fingerprint.clone(),
        worktrees: worktree_removals,
        branches: branch_deletions,
        feature_removed: true,
        descendants: descendant_outcomes,
    };

    let docs = record
        .approvals
        .documentation
        .as_ref()
        .map_or_else(|| review.to_owned(), |doc| doc.decision.to_string());
    write_record_outcome(layout, record, record_path, &apply_outcome)?;

    emit_cleaned(
        emitter,
        &feature,
        record_path,
        &docs,
        &apply_outcome,
        &descendants,
    );

    Ok(Report::with_warnings(
        CleanupOutcome {
            root: layout.root().to_path_buf(),
            preview,
            apply_outcome: Some(apply_outcome),
        },
        warnings,
    ))
}

/// The report of an apply that stopped before removing the feature, so its
/// record keeps no outcome and the cleanup can be retried.
fn partial_cleanup(
    layout: &Layout,
    feature: Feature,
    preview: CleanupPreview,
    (worktrees, branches): (Vec<WorktreeRemoval>, Vec<BranchDeletion>),
    descendants: Vec<CleanupDescendantOutcome>,
    warnings: Vec<Warning>,
) -> Report<CleanupOutcome> {
    let apply_outcome = CleanupApplyOutcome {
        feature: feature.name,
        branch: feature.branch,
        fingerprint: preview.fingerprint.clone(),
        worktrees,
        branches,
        feature_removed: false,
        descendants,
    };
    Report::with_warnings(
        CleanupOutcome {
            root: layout.root().to_path_buf(),
            preview,
            apply_outcome: Some(apply_outcome),
        },
        warnings,
    )
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/cleanup.rs"]
mod tests;
