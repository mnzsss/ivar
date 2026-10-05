use crate::domain::feature::{
    Feature, IntegrationOverride, IntegrationPolicy, IntegrationStrategy, IntegrationVia,
    RunReceipt,
};
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction};
use crate::git::Git;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::relations;
use super::receipts::ensure_failed_receipt_still_current;

pub(crate) fn default_title(feature: &FeatureName) -> String {
    format!("feat: integrate {feature}")
}

/// Refuse a title that carries AI attribution — the same lines
/// `ivar feature deliver` refuses — before anything moves. Deliberately
/// checked first, ahead of the root, plan-gate and descendant refusals: a
/// bad title is wrong whatever the tree's state.
pub(crate) fn refuse_attribution(feature: &FeatureName, title: &str) -> Result<(), Failure> {
    let findings: Vec<&str> = title
        .lines()
        .filter(|line| super::super::deliver::attribution::is_attribution(line))
        .map(str::trim)
        .collect();
    if findings.is_empty() {
        return Ok(());
    }
    Err(Failure::blocked(
        "integration.ai_attribution",
        format!("the integration title for `{feature}` carries AI attribution"),
    )
    .expected("no AI attribution in the --name title")
    .actual(findings.join("; "))
    .fix(FixAction::safe(
        "integration.remove_ai_attribution",
        "Drop the attribution lines from --name, then integrate again.",
    )))
}

/// An unrestricted live session cannot coexist with a first successful
/// receipt, because it could still write a locked promotion — refused before
/// any repo can gain one. And a non-terminal run already holding the lock on
/// the child feature is refused early, before any policy resolution, git
/// preflight, or parent mutation.
pub(crate) fn ensure_no_conflicting_session_or_run(
    layout: &Layout,
    name: &FeatureName,
    parent: &FeatureName,
    child: &Feature,
) -> Result<(), Failure> {
    if !child.has_any_receipt()
        && !super::super::relations::feature_session_entries(layout, name)?.is_empty()
    {
        return Err(Failure::blocked(
            "integration.session_live",
            format!(
                "feature `{name}` has a live session; integrating would lock a promotion an unrestricted session could still write"
            ),
        )
        .expected("no live feature session before the first successful receipt")
        .actual("a session view dir exists under the feature")
        .fix(
            FixAction::safe(
                "integration.integrate_from_parent",
                format!(
                    "In the child's session run `ivar feature execute finish {name}` and stop \
                     that session, then integrate it from the `{parent}` session: \
                     `ivar feature integrate {name}`."
                ),
            )
            .command(format!("ivar feature integrate {name}")),
        ));
    }
    if let Some(receipt) = RunReceipt::read(layout, name)?
        && receipt.holds_lock()
    {
        return Err(Failure::blocked(
            "integration.run_active",
            format!(
                "feature `{name}` has a {} run (`{}`)",
                receipt.status, receipt.id
            ),
        )
        .expected("a terminal run receipt before integrating the feature")
        .actual("the current run is still active and holds the feature lock")
        .fix(crate::action::execute::finish_or_interrupt_fix(name)));
    }
    Ok(())
}

/// The resolved policy for this run: the first receipt freezes it; otherwise
/// per-field CLI > feature > hall > embedded.
pub(crate) fn resolved_policy(
    child: &Feature,
    hall: IntegrationPolicy,
    via: Option<&str>,
    strategy: Option<&str>,
) -> Result<IntegrationPolicy, Failure> {
    if let Some(first) = child
        .promotions
        .values()
        .find_map(|promotion| promotion.integration_receipt.as_ref())
    {
        return Ok(IntegrationPolicy {
            via: first.via,
            strategy: first.strategy,
        });
    }
    let cli = IntegrationOverride {
        via: via.map(IntegrationVia::parse).transpose()?,
        strategy: strategy.map(IntegrationStrategy::parse).transpose()?,
    };
    Ok(IntegrationPolicy::resolve(cli, child.integration, hall))
}

/// Pass 1 of the whole-run preflight for one repo: every refusal that does
/// not require a parent promotion to already exist — a stale receipt, an
/// unresumable failed receipt, or a dirty worktree refuses the entire run.
/// Returns whether `repo` still needs pass 2's parent-promotion question:
/// recorded here rather than asked, so a later repo's refusal in this same
/// pass can never leave an earlier repo's parent promotion behind.
pub(crate) fn preflight_repo(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
) -> Result<bool, Failure> {
    let bare = layout.repo_bare(repo);
    let Some(receipt) = child
        .promotions
        .get(repo)
        .and_then(|promotion| promotion.integration_receipt.as_ref())
    else {
        // Unreceipted: the child worktree must be clean regardless. When the
        // parent already promotes the repo, its worktree exists too and must
        // be clean; when it doesn't, pass 2 will create it fresh from the
        // base branch — clean by construction — so there is nothing to check
        // yet, and the promotion itself is deferred to pass 2.
        let child_worktree = layout.repo_worktree(repo, &child.branch);
        if git.worktree_dirty(&child_worktree)? {
            return Err(dirty_failure(
                "integration.child_dirty",
                &child_worktree,
                "the child worktree has uncommitted changes",
            ));
        }
        if !parent.is_promoted(repo) {
            return Ok(true);
        }
        let parent_worktree = layout.repo_worktree(repo, &parent.branch);
        if git.worktree_dirty(&parent_worktree)? {
            return Err(dirty_failure(
                "integration.parent_dirty",
                &parent_worktree,
                "the parent worktree has uncommitted changes",
            ));
        }
        return Ok(false);
    };

    if receipt.verification.passed() {
        // A successful receipt must still be fresh — source moved, checks
        // drifted, or the result left the parent's history is a hard refusal
        // with restoration orientation.
        let freshness =
            relations::receipt_freshness(git, layout, manifest, child, parent, repo, receipt)?;
        if let relations::ReceiptFreshness::Stale { reason } = freshness {
            return Err(relations::stale_receipt_failure(
                layout, child, parent, repo, receipt, &reason,
            ));
        }
        return Ok(false);
    }

    // Failed evidence: resumable only while its source and result are
    // unchanged — moved means stale, with restoration orientation.
    ensure_failed_receipt_still_current(git, layout, &bare, child, parent, repo, receipt)?;
    Ok(false)
}

/// Pass 1 of the whole-run preflight for every promoted repo, so a later
/// repo's refusal can never leave an earlier repo's parent promotion behind.
/// Returns the repos that still need pass 2's parent-promotion question.
pub(crate) fn preflight_repos(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    child: &Feature,
    parent: &Feature,
) -> Result<Vec<RepoName>, Failure> {
    let mut needs_parent_promotion = Vec::new();
    for repo in child.promotions.keys() {
        if preflight_repo(layout, manifest, git, child, parent, repo)? {
            needs_parent_promotion.push(repo.clone());
        }
    }
    Ok(needs_parent_promotion)
}

/// The dirty-worktree refusal, shared by the child and parent preflights.
pub(crate) fn dirty_failure(
    code: &'static str,
    worktree: &camino::Utf8Path,
    reason: &str,
) -> Failure {
    Failure::blocked(
        code,
        format!("cannot integrate: the worktree at `{worktree}` has uncommitted changes"),
    )
    .expected("a clean worktree")
    .actual(reason)
    .fix(FixAction::safe(
        "integration.commit_or_stash",
        "Commit or stash the changes, then integrate again.",
    ))
}
