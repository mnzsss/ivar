use crate::domain::feature::{
    Feature, IntegrationReceipt, IntegrationStrategy, IntegrationVia, PrCheckResult,
    VerificationEvidence, VerificationResult,
};
use crate::domain::name::RepoName;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, FixAction};
use crate::git::Git;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::pull_requests;
use super::super::verification;
use super::receipts::{parent_checks_pending, persist_receipt};
use super::types::{RepoIntegration, RepoIntegrationStatus};

/// What waiting on a PR's required checks found: ready to merge, or already
/// blocked with the `RepoIntegration` to return.
enum PrCheckOutcome {
    Ready(Vec<PrCheckResult>),
    Blocked(RepoIntegration),
}

/// The PR path: push, reuse/create the PR against the parent's branch, check,
/// merge, observe, fetch the parent, and record the evidence.
#[allow(clippy::too_many_arguments)]
pub(crate) fn integrate_pr(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    strategy: IntegrationStrategy,
    title: &str,
    source_sha: &str,
    child_results: Vec<VerificationResult>,
) -> Result<RepoIntegration, Failure> {
    let bare = layout.repo_bare(repo);

    let pr = push_and_resolve_pr(git, &bare, manifest, child, parent, repo, title)?;
    let mut updated = child.clone();
    if let Some(promotion) = updated.promotions.get_mut(repo) {
        promotion.pr_url = Some(pr.url.clone());
    }
    updated.write(layout)?;

    let pr_checks = match wait_for_pr_checks(&bare, &pr, source_sha, repo, parent)? {
        PrCheckOutcome::Ready(checks) => checks,
        PrCheckOutcome::Blocked(integration) => return Ok(integration),
    };

    merge_pr_and_advance_parent(
        layout,
        git,
        manifest,
        child,
        parent,
        repo,
        strategy,
        title,
        source_sha,
        &pr,
        pr_checks,
        child_results,
    )
}

/// Push the child branch and reuse an existing PR (any state) or create one
/// against the immediate parent's branch — never an ancestor, never a
/// default branch.
fn push_and_resolve_pr(
    git: &impl Git,
    bare: &camino::Utf8Path,
    manifest: &Manifest,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    title: &str,
) -> Result<pull_requests::PullRequest, Failure> {
    let url = manifest
        .repos()
        .iter()
        .find(|candidate| candidate.name() == repo)
        .map(|candidate| candidate.url().to_owned())
        .unwrap_or_default();

    // Push the child branch so the forge has something to open a PR from.
    git.push(
        bare,
        &url,
        child.branch.as_str(),
        &format!("refs/heads/{}", child.branch),
    )?;

    match pull_requests::find_pull_request(bare, child.branch.as_str(), "all")? {
        Some(pr) => {
            pull_requests::edit_pull_request(bare, &pr.url, Some(title), None)?;
            Ok(pr)
        }
        None => pull_requests::create_pull_request(
            bare,
            &child.branch,
            &parent.branch,
            &child.name,
            Some(title),
            None,
            false,
        ),
    }
}

/// Confirm the PR's head still matches the recorded source, then check its
/// required checks — a failing or pending check blocks with the
/// `RepoIntegration` to return; otherwise the checks to merge with.
fn wait_for_pr_checks(
    bare: &camino::Utf8Path,
    pr: &pull_requests::PullRequest,
    source_sha: &str,
    repo: &RepoName,
    parent: &Feature,
) -> Result<PrCheckOutcome, Failure> {
    // The PR's head must still be the recorded source — `gh` enforces this at
    // merge time too via `--match-head-commit`, but refusing early is clearer.
    if let Some(head_oid) = &pr.head_oid
        && head_oid != source_sha
    {
        return Err(Failure::blocked(
            "integration.pr_head_moved",
            format!(
                "the PR for `{repo}` is on head {head_oid}, not the recorded source {source_sha}"
            ),
        )
        .expected("the PR head to match the child branch tip")
        .actual("the head moved after the PR was opened")
        .fix(FixAction::safe(
            "integration.push_source",
            "Push the child branch again to update the PR, or restore the recorded source.",
        )));
    }

    // Required checks gate the merge request.
    let pr_checks = pull_requests::required_checks(bare, &pr.url)?;
    if pr_checks.iter().any(|check| check.bucket == "fail") {
        return Ok(PrCheckOutcome::Blocked(RepoIntegration {
            repo: repo.clone(),
            source_sha: source_sha.to_owned(),
            target_branch: parent.branch.clone(),
            result_sha: None,
            status: RepoIntegrationStatus::Failed,
            pr_url: Some(pr.url.clone()),
            detail: Some("a required PR check failed".to_owned()),
        }));
    }
    if pr_checks.iter().any(|check| check.bucket == "pending") {
        return Ok(PrCheckOutcome::Blocked(RepoIntegration {
            repo: repo.clone(),
            source_sha: source_sha.to_owned(),
            target_branch: parent.branch.clone(),
            result_sha: None,
            status: RepoIntegrationStatus::Pending,
            pr_url: Some(pr.url.clone()),
            detail: Some("a required PR check is pending".to_owned()),
        }));
    }

    Ok(PrCheckOutcome::Ready(pr_checks))
}

/// Merge the PR, observe the result, then bring the parent up to it and
/// record the evidence.
#[allow(clippy::too_many_arguments)]
fn merge_pr_and_advance_parent(
    layout: &Layout,
    git: &impl Git,
    manifest: &Manifest,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    strategy: IntegrationStrategy,
    title: &str,
    source_sha: &str,
    pr: &pull_requests::PullRequest,
    pr_checks: Vec<PrCheckResult>,
    child_results: Vec<VerificationResult>,
) -> Result<RepoIntegration, Failure> {
    let bare = layout.repo_bare(repo);

    // Merge, observe, then bring the parent up to the observed result.
    pull_requests::request_merge(&bare, &pr.url, source_sha, strategy, title)?;
    let merged = pull_requests::observe_merge(&bare, &pr.url)?;
    let result_sha = merged.merge_commit.ok_or_else(|| {
        Failure::failed(
            "integration.merge_result_missing",
            format!("the merged PR {} reported no merge commit", pr.url),
        )
    })?;

    let parent_worktree = layout.repo_worktree(repo, &parent.branch);
    git.fetch_branch(&parent_worktree, parent.branch.as_str())?;
    git.fast_forward(&parent_worktree)?;

    // Written once the local parent carries the merge, so a resumed run finds
    // `result_sha` in the parent's history.
    let checks = verification::checks_for(manifest, repo);
    let mut receipt = IntegrationReceipt {
        source_sha: source_sha.to_owned(),
        target_branch: parent.branch.clone(),
        result_sha: result_sha.clone(),
        via: IntegrationVia::Pr,
        strategy,
        pr_url: Some(pr.url.clone()),
        verification: VerificationEvidence {
            command_fingerprint: verification::fingerprint(&checks)?,
            child: child_results,
            parent: vec![parent_checks_pending()],
            pr_checks,
            verified_at: rfc3339_now(),
        },
    };
    persist_receipt(layout, child, repo, receipt.clone())?;

    // The parent's checks run after the observed merge.
    let parent_run = verification::run(&checks, &parent_worktree)?;
    let passed = parent_run.results.iter().all(|result| result.success);
    receipt.verification.parent = parent_run.results;
    receipt.verification.verified_at = rfc3339_now();
    persist_receipt(layout, child, repo, receipt)?;

    Ok(RepoIntegration {
        repo: repo.clone(),
        source_sha: source_sha.to_owned(),
        target_branch: parent.branch.clone(),
        result_sha: Some(result_sha),
        status: if passed {
            RepoIntegrationStatus::Integrated
        } else {
            RepoIntegrationStatus::Failed
        },
        pr_url: Some(pr.url.clone()),
        detail: if passed {
            None
        } else {
            Some("merged, but the parent checks failed after the merge".to_owned())
        },
    })
}
