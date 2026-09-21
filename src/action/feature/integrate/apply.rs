//! The git-and-forge plumbing for integration: the local candidate path, the
//! PR path, parent-check verification, and receipt persistence. Orchestration
//! (what to do, and when) stays in the parent module; this is how one repo's
//! change actually lands.

use crate::domain::feature::{
    Feature, IntegrationReceipt, IntegrationStrategy, IntegrationVia, VerificationEvidence,
};
use crate::domain::name::RepoName;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, FixAction};
use crate::git::Git;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::pull_requests;
use super::super::verification;
use super::{RepoIntegration, RepoIntegrationStatus};

/// A staged local candidate, ready to be applied to the real parent once its
/// checks are confirmed passing.
struct StagedCandidate {
    worktree: camino::Utf8PathBuf,
    temp_branch: Option<String>,
    parent_sha: String,
    checks_passed: bool,
}

/// The local candidate path: build and check on a throwaway worktree, and
/// only a passing candidate may move the parent.
#[allow(clippy::too_many_arguments)]
pub(crate) fn integrate_local(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    strategy: IntegrationStrategy,
    source_sha: &str,
    child_results: Vec<crate::domain::feature::VerificationResult>,
) -> Result<RepoIntegration, Failure> {
    let bare = layout.repo_bare(repo);
    let parent_worktree = layout.repo_worktree(repo, &parent.branch);
    let checks = verification::checks_for(manifest, repo);

    let staged = stage_candidate(
        layout, git, child, parent, repo, strategy, source_sha, &checks,
    )?;
    if !staged.checks_passed {
        cleanup_staging(
            layout,
            git,
            repo,
            std::slice::from_ref(&staged.worktree),
            staged.temp_branch.as_deref(),
        )?;
        return Ok(RepoIntegration {
            repo: repo.clone(),
            source_sha: source_sha.to_owned(),
            target_branch: parent.branch.clone(),
            result_sha: None,
            status: RepoIntegrationStatus::Failed,
            pr_url: None,
            detail: Some("candidate checks failed; the parent was not touched".to_owned()),
        });
    }

    // The candidate passed and the parent must still be exactly where it was.
    if git.revision_commit(&bare, parent.branch.as_str())? != staged.parent_sha {
        cleanup_staging(
            layout,
            git,
            repo,
            std::slice::from_ref(&staged.worktree),
            staged.temp_branch.as_deref(),
        )?;
        return Err(Failure::blocked(
            "integration.parent_moved",
            format!(
                "the parent branch `{}` moved while the candidate was being checked",
                parent.branch
            ),
        )
        .expected("the parent to be untouched while the candidate is checked")
        .actual("the parent SHA changed under the integration")
        .fix(FixAction::safe(
            "integration.retry",
            "Run `ivar feature integrate` again — the parent has settled.",
        )));
    }

    let result_sha =
        apply_candidate_to_parent(layout, git, child, parent, repo, strategy, &staged)?;

    let parent_run = verification::run(&checks, &parent_worktree)?;
    let passed = parent_run.results.iter().all(|result| result.success);

    // 11. Persist the receipt immediately — success and post-parent failure
    // alike; a merged-then-failed-parent-check is recorded, never reverted.
    let fingerprint = verification::fingerprint(&checks)?;
    let receipt = IntegrationReceipt {
        source_sha: source_sha.to_owned(),
        target_branch: parent.branch.clone(),
        result_sha: result_sha.clone(),
        via: IntegrationVia::Local,
        strategy,
        pr_url: None,
        verification: VerificationEvidence {
            command_fingerprint: fingerprint,
            child: child_results,
            parent: parent_run.results,
            pr_checks: Vec::new(),
            verified_at: rfc3339_now(),
        },
    };
    persist_receipt(layout, child, repo, receipt)?;

    // Remove only the temporary staging worktrees/refs; the child's branch
    // and worktree are retained.
    cleanup_staging(
        layout,
        git,
        repo,
        std::slice::from_ref(&staged.worktree),
        staged.temp_branch.as_deref(),
    )?;

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
        pr_url: None,
        detail: if passed {
            None
        } else {
            Some("merged, but the parent checks failed after the merge".to_owned())
        },
    })
}

/// Stage a local candidate for `repo` — a rebase temp branch and worktree, or
/// a detached candidate worktree with the child merged in — and run the
/// parent's checks against it.
#[allow(clippy::too_many_arguments)]
fn stage_candidate(
    layout: &Layout,
    git: &impl Git,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    strategy: IntegrationStrategy,
    source_sha: &str,
    checks: &[String],
) -> Result<StagedCandidate, Failure> {
    let bare = layout.repo_bare(repo);
    let parent_sha = git.revision_commit(&bare, parent.branch.as_str())?;
    let candidate = layout.integration_candidate(&child.name, repo);

    // The rebase strategy stages in a temporary source worktree and
    // fast-forwards the parent; merge/squash stage in a detached candidate.
    if strategy == IntegrationStrategy::Rebase {
        let temp_branch = format!("ivar-integrate/{}/{}", child.name, repo);
        git.create_branch(&bare, &temp_branch, source_sha)?;
        let source_wt = layout.integration_source(&child.name, repo);
        git.add_worktree(&bare, &source_wt, &temp_branch)?;
        git.rebase_branch(&source_wt, parent.branch.as_str())?;
        let checks_passed = parent_checks_pass(&source_wt, checks)?;
        Ok(StagedCandidate {
            worktree: source_wt,
            temp_branch: Some(temp_branch),
            parent_sha,
            checks_passed,
        })
    } else {
        git.add_detached_worktree(&bare, &candidate, &parent_sha)?;
        match strategy {
            IntegrationStrategy::Squash => git.squash_merge(
                &candidate,
                child.branch.as_str(),
                &squash_message(child, repo),
            )?,
            IntegrationStrategy::Merge => git.merge_no_ff(&candidate, child.branch.as_str())?,
            IntegrationStrategy::Rebase => unreachable!("handled above"),
        }
        let checks_passed = parent_checks_pass(&candidate, checks)?;
        Ok(StagedCandidate {
            worktree: candidate,
            temp_branch: None,
            parent_sha,
            checks_passed,
        })
    }
}

/// Apply a passing staged candidate to the real parent worktree, returning
/// the resulting parent SHA.
fn apply_candidate_to_parent(
    layout: &Layout,
    git: &impl Git,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    strategy: IntegrationStrategy,
    staged: &StagedCandidate,
) -> Result<String, Failure> {
    let bare = layout.repo_bare(repo);
    let parent_worktree = layout.repo_worktree(repo, &parent.branch);
    match strategy {
        IntegrationStrategy::Rebase => {
            let temp_branch = staged.temp_branch.as_ref().ok_or_else(|| {
                Failure::failed(
                    "integration.rebase_staging_missing",
                    "the rebase staging branch vanished before the parent could be advanced",
                )
            })?;
            git.fast_forward_to(&parent_worktree, temp_branch)?
        }
        IntegrationStrategy::Squash => git.squash_merge(
            &parent_worktree,
            child.branch.as_str(),
            &squash_message(child, repo),
        )?,
        IntegrationStrategy::Merge => git.merge_no_ff(&parent_worktree, child.branch.as_str())?,
    }
    Ok(git.revision_commit(&bare, parent.branch.as_str())?)
}

/// What waiting on a PR's required checks found: ready to merge, or already
/// blocked with the `RepoIntegration` to return.
enum PrCheckOutcome {
    Ready(Vec<crate::domain::feature::PrCheckResult>),
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
    source_sha: &str,
    child_results: Vec<crate::domain::feature::VerificationResult>,
) -> Result<RepoIntegration, Failure> {
    let bare = layout.repo_bare(repo);

    let pr = push_and_resolve_pr(git, &bare, manifest, child, parent, repo)?;

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
        Some(pr) => Ok(pr),
        None => pull_requests::create_pull_request(
            bare,
            &child.branch,
            &parent.branch,
            &child.name,
            None,
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
    source_sha: &str,
    pr: &pull_requests::PullRequest,
    pr_checks: Vec<crate::domain::feature::PrCheckResult>,
    child_results: Vec<crate::domain::feature::VerificationResult>,
) -> Result<RepoIntegration, Failure> {
    let bare = layout.repo_bare(repo);

    // Merge, observe, then bring the parent up to the observed result.
    pull_requests::request_merge(&bare, &pr.url, source_sha, strategy)?;
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

    // The parent's checks run after the observed merge.
    let checks = verification::checks_for(manifest, repo);
    let parent_run = verification::run(&checks, &parent_worktree)?;
    let passed = parent_run.results.iter().all(|result| result.success);

    let receipt = IntegrationReceipt {
        source_sha: source_sha.to_owned(),
        target_branch: parent.branch.clone(),
        result_sha: result_sha.clone(),
        via: IntegrationVia::Pr,
        strategy,
        pr_url: Some(pr.url.clone()),
        verification: VerificationEvidence {
            command_fingerprint: verification::fingerprint(&checks)?,
            child: child_results,
            parent: parent_run.results,
            pr_checks,
            verified_at: rfc3339_now(),
        },
    };
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

/// Whether the ordered parent checks pass in `worktree`.
fn parent_checks_pass(worktree: &camino::Utf8Path, checks: &[String]) -> Result<bool, Failure> {
    Ok(verification::run(checks, worktree)?
        .results
        .iter()
        .all(|result| result.success))
}

/// The squash commit message: traceable back to the child.
fn squash_message(child: &Feature, repo: &RepoName) -> String {
    format!("Integrate `{}` ({}) into its parent", child.name, repo)
}

/// Persist `receipt` onto `child`'s promotion for `repo`, in one feature
/// write. The first receipt of any kind freezes structure from then on — the
/// mutation guards enforce that.
pub(crate) fn persist_receipt(
    layout: &Layout,
    child: &Feature,
    repo: &RepoName,
    receipt: IntegrationReceipt,
) -> Result<(), Failure> {
    let mut updated = child.clone();
    if let Some(promotion) = updated.promotions.get_mut(repo) {
        promotion.integration_receipt = Some(receipt);
    }
    updated.write(layout)
}

/// Remove the temporary staging worktrees that were actually created (and the
/// rebase temp branch, when there was one) — never the child's own branch or
/// worktree.
fn cleanup_staging(
    layout: &Layout,
    git: &impl Git,
    repo: &RepoName,
    worktrees: &[camino::Utf8PathBuf],
    temp_branch: Option<&str>,
) -> Result<(), Failure> {
    let bare = layout.repo_bare(repo);
    for wt in worktrees {
        if fs::is_dir(wt)? {
            let _ = git.remove_worktree(&bare, wt);
        }
    }
    if let Some(branch) = temp_branch {
        let _ = git.delete_branch(&bare, branch);
    }
    Ok(())
}
