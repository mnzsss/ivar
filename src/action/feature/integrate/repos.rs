use crate::domain::feature::{Feature, IntegrationPolicy, IntegrationVia};
use crate::domain::name::{FeatureName, RepoName};
use crate::error::Failure;
use crate::git::Git;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::relations;
use super::super::verification;
use super::local::integrate_local;
use super::pr::integrate_pr;
use super::receipts::reverify_failed_receipt;
use super::types::{RepoIntegration, RepoIntegrationStatus};

/// Integrate every promoted repo, in name order: reuse, re-verify, or
/// resume. Each result is persisted immediately — partial and resumable,
/// never atomic. `child` is re-read after each repo so the next persist
/// carries every earlier receipt, never clobbering it. A repo that breaks
/// mid-run (a conflict, a refused PR) does not stop the batch, but fails the
/// run once every repo has had its turn.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_integration_repos(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    child: &Feature,
    parent: &Feature,
    policy: IntegrationPolicy,
    title: &str,
    name: &FeatureName,
) -> Result<Vec<RepoIntegration>, Failure> {
    let mut child = child.clone();
    let mut repos_out = Vec::new();
    let mut broken = Vec::new();
    for repo in child.promotions.keys().cloned().collect::<Vec<_>>() {
        match integrate_repo(layout, manifest, git, &child, parent, &repo, policy, title) {
            Ok(entry) => repos_out.push(entry),
            Err(failure) => broken.push(format!("{repo}: {}", failure.what)),
        }
        if let Ok(fresh) = relations::read_feature(layout, name) {
            child = fresh;
        }
    }
    if broken.is_empty() {
        return Ok(repos_out);
    }
    Err(Failure::failed(
        "integration.repo_failed",
        format!(
            "integrating `{name}` into `{}` failed in {} repo(s)",
            parent.name,
            broken.len()
        ),
    )
    .expected("every repo to integrate, or to record its failed checks")
    .actual(broken.join("\n"))
    .fix(
        crate::error::FixAction::safe(
            "integration.retry",
            format!("Fix the cause in the child, then run `ivar feature integrate {name}` again."),
        )
        .command(format!("ivar feature integrate {name}")),
    ))
}

/// One repo's integration: reuse a fresh receipt, re-verify an unchanged
/// failed one, or resume an unreceipted one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn integrate_repo(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    policy: IntegrationPolicy,
    title: &str,
) -> Result<RepoIntegration, Failure> {
    let bare = layout.repo_bare(repo);
    let source_sha = git.revision_commit(&bare, child.branch.as_str())?;

    let Some(receipt) = child
        .promotions
        .get(repo)
        .and_then(|promotion| promotion.integration_receipt.as_ref())
    else {
        return resume_repo(
            layout,
            manifest,
            git,
            child,
            parent,
            repo,
            policy,
            title,
            &source_sha,
        );
    };

    if receipt.verification.passed() {
        // Reuse: the receipt must still be fresh against live state.
        let freshness =
            relations::receipt_freshness(git, layout, manifest, child, parent, repo, receipt)?;
        return match freshness {
            relations::ReceiptFreshness::Fresh => Ok(RepoIntegration {
                repo: repo.clone(),
                source_sha: receipt.source_sha.clone(),
                target_branch: parent.branch.clone(),
                result_sha: Some(receipt.result_sha.clone()),
                status: RepoIntegrationStatus::Reused,
                pr_url: receipt.pr_url.clone(),
                detail: None,
            }),
            relations::ReceiptFreshness::Failed => Ok(RepoIntegration {
                repo: repo.clone(),
                source_sha: receipt.source_sha.clone(),
                target_branch: parent.branch.clone(),
                result_sha: Some(receipt.result_sha.clone()),
                status: RepoIntegrationStatus::Failed,
                pr_url: receipt.pr_url.clone(),
                detail: Some("recorded evidence failed".to_owned()),
            }),
            relations::ReceiptFreshness::Stale { reason } => Err(relations::stale_receipt_failure(
                layout, child, parent, repo, receipt, &reason,
            )),
        };
    }

    // Failed evidence: resumable only when the source and result are
    // unchanged — the change is already in the parent, so only the parent
    // verification is re-run, never the application.
    reverify_failed_receipt(layout, manifest, git, child, parent, repo, receipt)
}

/// Resume an unreceipted repo: the full local or PR integration.
#[allow(clippy::too_many_arguments)]
fn resume_repo(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    policy: IntegrationPolicy,
    title: &str,
    source_sha: &str,
) -> Result<RepoIntegration, Failure> {
    // 7/8. The preflight already guaranteed the parent promotion (or refused
    // with the exact command) and clean child/parent worktrees.

    // 9. The child's own ordered checks run before anything moves.
    let child_worktree = layout.repo_worktree(repo, &child.branch);
    let checks = verification::checks_for(manifest, repo);
    let child_run = verification::run(&checks, &child_worktree)?;
    if !child_run.results.iter().all(|result| result.success) {
        return Ok(RepoIntegration {
            repo: repo.clone(),
            source_sha: source_sha.to_owned(),
            target_branch: parent.branch.clone(),
            result_sha: None,
            status: RepoIntegrationStatus::Failed,
            pr_url: None,
            detail: Some("child checks failed".to_owned()),
        });
    }

    // 10. Execute the selected via, carrying the child-check evidence so the
    // receipt records it.
    let child_results = child_run.results;
    match policy.via {
        IntegrationVia::Local => integrate_local(
            layout,
            manifest,
            git,
            child,
            parent,
            repo,
            policy.strategy,
            title,
            source_sha,
            child_results,
        ),
        IntegrationVia::Pr => integrate_pr(
            layout,
            manifest,
            git,
            child,
            parent,
            repo,
            policy.strategy,
            title,
            source_sha,
            child_results,
        ),
    }
}
