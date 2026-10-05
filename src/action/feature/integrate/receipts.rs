use crate::domain::feature::{Feature, IntegrationReceipt, VerificationEvidence};
use crate::domain::name::{FeatureName, RepoName};
use crate::domain::session::rfc3339_now;
use crate::error::Failure;
use crate::git::Git;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::relations;
use super::super::verification;
use super::types::{RepoIntegration, RepoIntegrationStatus};

/// Persist `receipt` onto `child`'s promotion for `repo`, in one feature
/// write. The first receipt of any kind freezes structure from then on — the
/// mutation guards enforce that.
pub(crate) fn persist_receipt(
    layout: &Layout,
    child: &Feature,
    repo: &RepoName,
    receipt: IntegrationReceipt,
) -> Result<(), Failure> {
    Feature::update(layout, &child.name, |updated| {
        if let Some(promotion) = updated.promotions.get_mut(repo) {
            promotion.integration_receipt = Some(receipt);
        }
        Ok(())
    })
}

/// The placeholder parent evidence of a provisional receipt: written the
/// moment the parent branch moves, it reads as failed evidence, so a rerun
/// resumes at parent verification instead of merging again.
pub(crate) fn parent_checks_pending() -> crate::domain::feature::VerificationResult {
    crate::domain::feature::VerificationResult::failed(
        "parent checks",
        None,
        "the parent branch moved but its checks have not run yet; run `ivar feature integrate` again",
    )
}

/// A failed receipt is resumable only while its source and result are
/// unchanged; either moving means the evidence is stale, refused with
/// restoration orientation. Shared by the preflight check and the actual
/// integration, which both re-derive freshness before reusing failed evidence.
pub(crate) fn ensure_failed_receipt_still_current(
    git: &impl Git,
    layout: &Layout,
    bare: &camino::Utf8Path,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    receipt: &IntegrationReceipt,
) -> Result<(), Failure> {
    let source_unchanged = git
        .revision_commit(bare, child.branch.as_str())
        .is_ok_and(|tip| tip == receipt.source_sha);
    let result_unchanged = git
        .is_ancestor(bare, &receipt.result_sha, parent.branch.as_str())
        .unwrap_or(false);
    if !source_unchanged || !result_unchanged {
        return Err(relations::stale_receipt_failure(
            layout,
            child,
            parent,
            repo,
            receipt,
            "the failed receipt's source or result has moved",
        ));
    }
    Ok(())
}

/// Re-verify an unchanged failed receipt: re-run parent checks and update receipt.
pub(crate) fn reverify_failed_receipt(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    receipt: &IntegrationReceipt,
) -> Result<RepoIntegration, Failure> {
    let bare = layout.repo_bare(repo);
    ensure_failed_receipt_still_current(git, layout, &bare, child, parent, repo, receipt)?;

    let parent_checks = verification::checks_for(manifest, repo);
    let parent_worktree = layout.repo_worktree(repo, &parent.branch);
    let verification_run = verification::run(&parent_checks, &parent_worktree)?;
    let passed = verification_run.results.iter().all(|result| result.success);
    let mut updated = receipt.clone();
    updated.verification = VerificationEvidence {
        command_fingerprint: verification_run.command_fingerprint,
        child: receipt.verification.child.clone(),
        parent: verification_run.results,
        pr_checks: receipt.verification.pr_checks.clone(),
        verified_at: rfc3339_now(),
    };
    persist_receipt(layout, child, repo, updated)?;

    Ok(RepoIntegration {
        repo: repo.clone(),
        source_sha: receipt.source_sha.clone(),
        target_branch: parent.branch.clone(),
        result_sha: Some(receipt.result_sha.clone()),
        status: if passed {
            RepoIntegrationStatus::Reused
        } else {
            RepoIntegrationStatus::Failed
        },
        pr_url: receipt.pr_url.clone(),
        detail: if passed {
            None
        } else {
            Some("parent re-verification failed".to_owned())
        },
    })
}

/// The temporary branch the rebase strategy replays the child onto.
pub(crate) fn rebase_branch(child: &FeatureName, repo: &RepoName) -> String {
    format!("ivar-integrate/{child}/{repo}")
}

/// Remove every staging worktree and the rebase branch `child` can leave in
/// `repo` — never the child's own branch or worktree. Best effort: whatever
/// does not exist is skipped.
pub(crate) fn clear_staging(layout: &Layout, git: &impl Git, child: &FeatureName, repo: &RepoName) {
    let bare = layout.repo_bare(repo);
    for worktree in [
        layout.integration_candidate(child, repo),
        layout.integration_source(child, repo),
    ] {
        let _ = git.remove_worktree(&bare, &worktree);
    }
    let _ = git.delete_branch(&bare, &rebase_branch(child, repo));
}
