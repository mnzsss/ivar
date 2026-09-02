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
    let parent_sha = git.revision_commit(&bare, parent.branch.as_str())?;
    let parent_worktree = layout.repo_worktree(repo, &parent.branch);
    let checks = verification::checks_for(manifest, repo);
    let candidate = layout.integration_candidate(&child.name, repo);

    // The rebase strategy stages in a temporary source worktree and
    // fast-forwards the parent; merge/squash stage in a detached candidate.
    let mut temporary: Option<(camino::Utf8PathBuf, String)> = None;
    if strategy == IntegrationStrategy::Rebase {
        let temp_branch = format!("ivar-integrate/{}/{}", child.name, repo);
        git.create_branch(&bare, &temp_branch, source_sha)?;
        let source_wt = layout.integration_source(&child.name, repo);
        git.add_worktree(&bare, &source_wt, &temp_branch)?;
        git.rebase_branch(&source_wt, parent.branch.as_str())?;
        if !parent_checks_pass(&source_wt, &checks)? {
            cleanup_staging(
                layout,
                git,
                repo,
                std::slice::from_ref(&source_wt),
                Some(&temp_branch),
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
        temporary = Some((source_wt, temp_branch));
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
        if !parent_checks_pass(&candidate, &checks)? {
            cleanup_staging(layout, git, repo, std::slice::from_ref(&candidate), None)?;
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
    }

    // The candidate passed and the parent must still be exactly where it was.
    if git.revision_commit(&bare, parent.branch.as_str())? != parent_sha {
        let worktree = temporary
            .as_ref()
            .map(|(wt, _)| wt.clone())
            .unwrap_or_else(|| candidate.clone());
        let temp_branch = temporary.as_ref().map(|(_, branch)| branch.as_str());
        cleanup_staging(
            layout,
            git,
            repo,
            std::slice::from_ref(&worktree),
            temp_branch,
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

    // Apply to the real parent, then check the real parent again.
    match strategy {
        IntegrationStrategy::Rebase => {
            let temp_branch = temporary
                .as_ref()
                .map(|(_, branch)| branch)
                .ok_or_else(|| {
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
    let result_sha = git.revision_commit(&bare, parent.branch.as_str())?;

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
    let worktree = temporary
        .as_ref()
        .map(|(wt, _)| wt.clone())
        .unwrap_or_else(|| candidate.clone());
    let temp_branch = temporary.as_ref().map(|(_, branch)| branch.as_str());
    cleanup_staging(
        layout,
        git,
        repo,
        std::slice::from_ref(&worktree),
        temp_branch,
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
    let url = manifest
        .repos()
        .iter()
        .find(|candidate| candidate.name() == repo)
        .map(|candidate| candidate.url().to_owned())
        .unwrap_or_default();

    // Push the child branch so the forge has something to open a PR from.
    git.push(
        &bare,
        &url,
        child.branch.as_str(),
        &format!("refs/heads/{}", child.branch),
    )?;

    // Reuse an existing PR (any state) or create one against the immediate
    // parent's branch — never an ancestor, never a default branch.
    let pr = match pull_requests::find_pull_request(&bare, child.branch.as_str(), "all")? {
        Some(pr) => pr,
        None => pull_requests::create_pull_request(
            &bare,
            &child.branch,
            &parent.branch,
            &child.name,
            None,
            None,
            false,
        )?,
    };

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
    let pr_checks = pull_requests::required_checks(&bare, &pr.url)?;
    if pr_checks.iter().any(|check| check.bucket == "fail") {
        return Ok(RepoIntegration {
            repo: repo.clone(),
            source_sha: source_sha.to_owned(),
            target_branch: parent.branch.clone(),
            result_sha: None,
            status: RepoIntegrationStatus::Failed,
            pr_url: Some(pr.url.clone()),
            detail: Some("a required PR check failed".to_owned()),
        });
    }
    if pr_checks.iter().any(|check| check.bucket == "pending") {
        return Ok(RepoIntegration {
            repo: repo.clone(),
            source_sha: source_sha.to_owned(),
            target_branch: parent.branch.clone(),
            result_sha: None,
            status: RepoIntegrationStatus::Pending,
            pr_url: Some(pr.url.clone()),
            detail: Some("a required PR check is pending".to_owned()),
        });
    }

    // Merge, observe, then bring the parent up to the observed result.
    pull_requests::request_merge(&bare, &pr.url, source_sha, strategy)?;
    let merged = pull_requests::observe_merge(&bare, &pr.url)?;
    let result_sha = merged.merge_commit.clone().ok_or_else(|| {
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
        pr_url: Some(pr.url),
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
