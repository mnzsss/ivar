use crate::domain::feature::{
    Feature, IntegrationReceipt, IntegrationStrategy, IntegrationVia, VerificationEvidence,
    VerificationResult,
};
use crate::domain::name::RepoName;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, FixAction};
use crate::git::Git;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::verification;
use super::receipts::{clear_staging, parent_checks_pending, persist_receipt, rebase_branch};
use super::types::{RepoIntegration, RepoIntegrationStatus};

/// A staged local candidate, ready to be applied to the real parent once its
/// checks are confirmed passing.
struct StagedCandidate {
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
    title: &str,
    source_sha: &str,
    child_results: Vec<VerificationResult>,
) -> Result<RepoIntegration, Failure> {
    clear_staging(layout, git, &child.name, repo);
    let integrated = integrate_staged(
        layout,
        manifest,
        git,
        child,
        parent,
        repo,
        strategy,
        title,
        source_sha,
        child_results,
    );
    clear_staging(layout, git, &child.name, repo);
    integrated
}

/// [`integrate_local`]'s body; staging is cleared around it on every path.
#[allow(clippy::too_many_arguments)]
fn integrate_staged(
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
    let parent_worktree = layout.repo_worktree(repo, &parent.branch);
    let checks = verification::checks_for(manifest, repo);

    let staged = stage_candidate(
        layout, git, child, parent, repo, strategy, title, source_sha, &checks,
    )?;
    if !staged.checks_passed {
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
        apply_candidate_to_parent(layout, git, child, parent, repo, strategy, title, &staged)?;

    let mut receipt = IntegrationReceipt {
        source_sha: source_sha.to_owned(),
        target_branch: parent.branch.clone(),
        result_sha: result_sha.clone(),
        via: IntegrationVia::Local,
        strategy,
        pr_url: None,
        verification: VerificationEvidence {
            command_fingerprint: verification::fingerprint(&checks)?,
            child: child_results,
            parent: vec![parent_checks_pending()],
            pr_checks: Vec::new(),
            verified_at: rfc3339_now(),
        },
    };
    persist_receipt(layout, child, repo, receipt.clone())?;

    // Success and post-parent failure alike are recorded, never reverted.
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
    title: &str,
    source_sha: &str,
    checks: &[String],
) -> Result<StagedCandidate, Failure> {
    let bare = layout.repo_bare(repo);
    let parent_sha = git.revision_commit(&bare, parent.branch.as_str())?;
    let candidate = layout.integration_candidate(&child.name, repo);

    // The rebase strategy stages in a temporary source worktree and
    // fast-forwards the parent; merge/squash stage in a detached candidate.
    if strategy == IntegrationStrategy::Rebase {
        let temp_branch = rebase_branch(&child.name, repo);
        git.create_branch(&bare, &temp_branch, source_sha)?;
        let source_wt = layout.integration_source(&child.name, repo);
        git.add_worktree(&bare, &source_wt, &temp_branch)?;
        git.rebase_branch(&source_wt, parent.branch.as_str())?;
        let checks_passed = parent_checks_pass(&source_wt, checks)?;
        Ok(StagedCandidate {
            temp_branch: Some(temp_branch),
            parent_sha,
            checks_passed,
        })
    } else {
        git.add_detached_worktree(&bare, &candidate, &parent_sha)?;
        match strategy {
            IntegrationStrategy::Squash => {
                git.squash_merge(&candidate, child.branch.as_str(), title)?;
            }
            IntegrationStrategy::Merge => {
                git.merge_no_ff(&candidate, child.branch.as_str(), title)?;
            }
            IntegrationStrategy::Rebase => unreachable!("handled above"),
        }
        let checks_passed = parent_checks_pass(&candidate, checks)?;
        Ok(StagedCandidate {
            temp_branch: None,
            parent_sha,
            checks_passed,
        })
    }
}

/// Apply a passing staged candidate to the real parent worktree, returning
/// the resulting parent SHA.
#[allow(clippy::too_many_arguments)]
fn apply_candidate_to_parent(
    layout: &Layout,
    git: &impl Git,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    strategy: IntegrationStrategy,
    title: &str,
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
        IntegrationStrategy::Squash => {
            git.squash_merge(&parent_worktree, child.branch.as_str(), title)?;
        }
        IntegrationStrategy::Merge => {
            git.merge_no_ff(&parent_worktree, child.branch.as_str(), title)?;
        }
    }
    Ok(git.revision_commit(&bare, parent.branch.as_str())?)
}

/// Whether the ordered parent checks pass in `worktree`.
fn parent_checks_pass(worktree: &camino::Utf8Path, checks: &[String]) -> Result<bool, Failure> {
    Ok(verification::run(checks, worktree)?
        .results
        .iter()
        .all(|result| result.success))
}
