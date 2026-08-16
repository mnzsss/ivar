//! `ivar feature integrate <child> [--via pr|local] [--strategy …]` — a
//! child's changes land in its immediate parent, leaves first, partially,
//! durably, and resumably.
//!
//! # What this verb is
//!
//! Integration is valid **only for a child**, and only when every descendant
//! is integrated, verified, or abandoned — the leaves-first rule. Each
//! promoted repo is integrated into the immediate parent's branch, one at a
//! time, and each result is persisted as a receipt the moment it lands —
//! success *and* a post-parent failure — so a multi-repo integration is
//! explicitly partial and resumable, never atomic.
//!
//! # The receipt is the memory
//!
//! A rerun of `integrate` reads each promotion's receipt and decides: a fresh
//! passing receipt is reused; a failed-evidence receipt whose source and
//! result are unchanged is re-verified (parent checks only — the change is
//! already in the parent); anything stale is refused with restoration
//! orientation. Only when every receipt is fresh and passing does the child
//! close with outcome `integrated` — freezing the whole child.
//!
//! # Policy
//!
//! Per-field precedence: CLI override > feature override > hall default >
//! embedded default (`local`/`squash`). The resolved policy is frozen by the
//! first persisted receipt; a rerun uses the receipt's own via/strategy.
//!
//! # The parent-promotion question
//!
//! A repo the child promotes but the parent does not must be promoted into
//! the parent before it can receive the child's work. Interactive runs ask;
//! a `--json`, `$CI`, or non-tty run refuses with the exact command
//! `ivar feature promote <parent> <repo>`.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{
    ApprovalState, Feature, FeatureIntegrationState, Gate, GateState, IntegrationOverride,
    IntegrationPolicy, IntegrationStrategy, IntegrationVia, VerificationEvidence,
};
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::{discover_hall, read_manifest};
use super::close::{self, CloseInput};
use super::lifecycle::read_close;
use super::promote::{self, PromoteInput};
use super::relations;
use super::verification;
use crate::action::Ctx;

mod apply;

use apply::{integrate_local, integrate_pr, persist_receipt};

/// What `ivar feature integrate` needs.
#[derive(Debug, Clone)]
pub struct IntegrateInput {
    /// The child feature to integrate into its immediate parent.
    pub feature: String,
    /// A via override — `pr` or `local`, unvalidated.
    pub via: Option<String>,
    /// A strategy override — `squash`, `merge`, or `rebase`, unvalidated.
    pub strategy: Option<String>,
}

/// One repo's integration result within a run.
#[derive(Debug, Clone, Serialize)]
pub struct RepoIntegration {
    /// The repo.
    pub repo: RepoName,
    /// The child branch's tip this repo was integrated at.
    pub source_sha: String,
    /// The immediate parent's branch — the only target a child ever has.
    pub target_branch: BranchName,
    /// The result commit on the parent's branch, once applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_sha: Option<String>,
    /// What happened to this repo this run.
    pub status: RepoIntegrationStatus,
    /// The pull request that carried the change, when `via=pr`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    /// Why this repo is pending, failed, or stale.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// What happened to one repo this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoIntegrationStatus {
    /// A fresh passing receipt was validated and reused; nothing moved.
    Reused,
    /// The repo was integrated now.
    Integrated,
    /// Waiting on something resumable (a pending PR check, an observe
    /// timeout).
    Pending,
    /// The integration failed — failed evidence, or a refused merge.
    Failed,
    /// The receipt no longer matches live state.
    Stale,
}

/// What `ivar feature integrate` did.
#[derive(Debug, Clone, Serialize)]
pub struct IntegrateOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The child that was integrated.
    pub feature: FeatureName,
    /// The immediate parent it integrated into.
    pub parent: FeatureName,
    /// The resolved integration policy for this run.
    pub policy: IntegrationPolicy,
    /// One entry per promoted repo, in name order.
    pub repos: Vec<RepoIntegration>,
    /// The child's derived integration state after the run.
    pub state: FeatureIntegrationState,
    /// Whether the run closed the child with outcome `integrated`.
    pub closed_integrated: bool,
}

impl std::fmt::Display for RepoIntegrationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Reused => "reused",
            Self::Integrated => "integrated",
            Self::Pending => "pending",
            Self::Failed => "failed",
            Self::Stale => "stale",
        };
        f.pad(name)
    }
}

impl std::fmt::Display for IntegrationPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.via, self.strategy)
    }
}

impl WriteHuman for IntegrateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Integrated `{}` into `{}` ({}):",
            self.feature, self.parent, self.policy
        )?;
        for repo in &self.repos {
            let detail = repo
                .detail
                .as_deref()
                .map(|d| format!(" — {d}"))
                .unwrap_or_default();
            let result = repo
                .result_sha
                .as_deref()
                .map(|sha| format!(" at {sha}"))
                .unwrap_or_default();
            writeln!(w, "  {}  {}{}{detail}", repo.repo, repo.status, result)?;
        }
        if self.closed_integrated {
            writeln!(w, "Closed `{}` as integrated.", self.feature)?;
        }
        Ok(())
    }
}

/// Integrate `input.feature` into its immediate parent, leaves first.
///
/// Refused when the feature is a root (roots deliver), its plan gate is not
/// approved, any descendant blocks, or an unrestricted live session would
/// gain its first successful receipt. See the module doc for the partial,
/// resumable receipt model.
pub fn integrate(ctx: &Ctx, input: IntegrateInput) -> Outcome<IntegrateOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;
    let name = FeatureName::new(input.feature)?;

    // 1. The child and its immediate parent. The tree is validated by the
    // read: a missing parent or a cycle refuses before anything else.
    relations::read_all(&layout)?;
    let child = relations::read_feature(&layout, &name)?;
    let parent_name = child.parent.clone().ok_or_else(|| {
        Failure::blocked(
            "integration.root_refused",
            format!("feature `{name}` is a root and cannot be integrated"),
        )
        .expected("a child feature (one with a parent) to integrate")
        .actual("this feature has no parent")
        .fix(
            FixAction::safe(
                "integration.deliver_root",
                format!("Deliver the root instead: `ivar feature deliver {name}`."),
            )
            .command(format!("ivar feature deliver {name}")),
        )
    })?;
    let parent = relations::read_feature(&layout, &parent_name)?;

    // 2. The plan gate must be approved — integration is a planned act, and
    // the artifact a human crossed is the gate (see ARCHITECTURE.md, seam 7).
    let plan_gate = ApprovalState::read(&layout, &name)?
        .unwrap_or_default()
        .state(Gate::Plan)
        .unwrap_or(GateState::Pending);
    if plan_gate != GateState::Approved {
        return Err(Failure::blocked(
            "integration.plan_not_approved",
            format!("integrating `{name}` needs its plan gate approved"),
        )
        .expected("the `plan` gate in state approved")
        .actual(format!("the plan gate is `{plan_gate}`"))
        .fix(FixAction::safe(
            "integration.approve_plan",
            format!("Approve it with `ivar plan approve {name} plan`, then integrate again."),
        )));
    }

    // 3. Leaves first: every blocking descendant refuses the whole run, and
    // the failure names each blocker.
    let blockers = relations::blocking_descendants(&git, &layout, &manifest, &child)?;
    if !blockers.is_empty() {
        return Err(relations::tree_block_failure(&name, &blockers));
    }

    // 4. An unrestricted live session cannot coexist with a first successful
    // receipt: the session has no write contract, so a locked promotion would
    // be writable from it. Refused before any repo can gain one.
    if !child.has_any_receipt() && has_live_sessions(&layout, &name)? {
        return Err(Failure::blocked(
            "integration.session_live",
            format!(
                "feature `{name}` has a live session; integrating would lock a promotion an unrestricted session could still write"
            ),
        )
        .expected("no live feature session before the first successful receipt")
        .actual("a session view dir exists under the feature")
        .fix(FixAction::safe(
            "integration.stop_session_first",
            format!("Stop the session first, then run `ivar feature integrate {name}` again."),
        )));
    }

    // 5. Resolve the policy once. The resolved relationship/base/policy is
    // frozen by the first persisted receipt: a rerun reuses each receipt's
    // own via/strategy instead of re-resolving.
    let policy = resolved_policy(
        &child,
        manifest.integration(),
        input.via.as_deref(),
        input.strategy.as_deref(),
    )?;

    // 6. Preflight every repo before anything moves: a stale receipt, a
    // missing parent promotion (with nobody to ask), or a dirty worktree is a
    // hard refusal of the whole run — nothing is persisted, nothing is
    // exposed. Only the *work* of a resume (checks, candidate, merge) is a
    // per-repo warning that lets the batch continue.
    for repo in child.promotions.keys() {
        preflight_repo(ctx, &layout, &manifest, &git, &child, &parent, repo)?;
    }

    // 7. Per-repo, in name order: reuse, re-verify, or resume. Each result is
    // persisted immediately — partial and resumable, never atomic. The child
    // is re-read after each repo so the next persist carries every earlier
    // receipt, never clobbering it.
    let mut child = child;
    let mut repos_out = Vec::new();
    let mut warnings = Vec::new();
    for repo in child.promotions.keys().cloned().collect::<Vec<_>>() {
        match integrate_repo(&layout, &manifest, &git, &child, &parent, &repo, policy) {
            Ok(entry) => repos_out.push(entry),
            Err(failure) => {
                // A repo that breaks mid-run (checks failed, candidate failed,
                // PR refused) stops that repo but lets the batch continue with
                // a warning — successful receipts stay reused, and the
                // resumable ones stay resumable.
                warnings.push(Warning::new(
                    "integration.repo_blocked",
                    repo.as_str(),
                    failure.to_string(),
                ));
                repos_out.push(RepoIntegration {
                    repo: repo.clone(),
                    source_sha: git
                        .revision_commit(&layout.repo_bare(&repo), child.branch.as_str())
                        .unwrap_or_default(),
                    target_branch: parent.branch.clone(),
                    result_sha: None,
                    status: RepoIntegrationStatus::Failed,
                    pr_url: None,
                    detail: Some(failure.what.clone()),
                });
            }
        }
        child = relations::read_feature(&layout, &name)?;
    }

    // 13. Close as integrated only when every receipt is fresh and passing.
    let (state, closed_integrated) = final_state(
        ctx,
        &layout,
        &manifest,
        &git,
        &child,
        &parent,
        &mut warnings,
    )?;

    Ok(Report::with_warnings(
        IntegrateOutcome {
            root: layout.root().to_path_buf(),
            feature: name,
            parent: parent_name,
            policy,
            repos: repos_out,
            state,
            closed_integrated,
        },
        warnings,
    ))
}

/// The whole-run preflight for one repo: every refusal that must happen
/// before anything is persisted or exposed. A stale receipt, a missing
/// parent promotion, or a dirty worktree refuses the entire run.
#[allow(clippy::too_many_arguments)]
fn preflight_repo(
    ctx: &Ctx,
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
) -> Result<(), Failure> {
    let bare = layout.repo_bare(repo);
    let Some(receipt) = child
        .promotions
        .get(repo)
        .and_then(|promotion| promotion.integration_receipt.as_ref())
    else {
        // Unreceipted: the parent must promote the repo (interactive, or the
        // exact command), and both worktrees must be clean.
        if !parent.is_promoted(repo) {
            ensure_parent_promotion(ctx, child, parent, repo)?;
        }
        let child_worktree = layout.repo_worktree(repo, &child.branch);
        let parent_worktree = layout.repo_worktree(repo, &parent.branch);
        if git.worktree_dirty(&child_worktree)? {
            return Err(dirty_failure(
                "integration.child_dirty",
                &child_worktree,
                "the child worktree has uncommitted changes",
            ));
        }
        if git.worktree_dirty(&parent_worktree)? {
            return Err(dirty_failure(
                "integration.parent_dirty",
                &parent_worktree,
                "the parent worktree has uncommitted changes",
            ));
        }
        return Ok(());
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
        return Ok(());
    }

    // Failed evidence: resumable only while its source and result are
    // unchanged — moved means stale, with restoration orientation.
    let source_unchanged = git
        .revision_commit(&bare, child.branch.as_str())
        .is_ok_and(|tip| tip == receipt.source_sha);
    let result_unchanged = git
        .is_ancestor(&bare, &receipt.result_sha, parent.branch.as_str())
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

/// The dirty-worktree refusal, shared by the child and parent preflights.
fn dirty_failure(code: &'static str, worktree: &camino::Utf8Path, reason: &str) -> Failure {
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

/// One repo's integration: reuse a fresh receipt, re-verify an unchanged
/// failed one, or resume an unreceipted one.
#[allow(clippy::too_many_arguments)]
fn integrate_repo(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    policy: IntegrationPolicy,
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
    let source_unchanged = git
        .revision_commit(&bare, child.branch.as_str())
        .is_ok_and(|tip| tip == receipt.source_sha);
    let result_unchanged = git
        .is_ancestor(&bare, &receipt.result_sha, parent.branch.as_str())
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
    persist_receipt(layout, child, repo, updated.clone())?;

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
            source_sha,
            child_results,
        ),
    }
}

/// Ask about (or refuse with the exact command for) promoting `repo` into the
/// parent. Interactive runs ask; everything else refuses before any mutation.
fn ensure_parent_promotion(
    ctx: &Ctx,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
) -> Result<(), Failure> {
    let question = format!(
        "Feature `{}` does not promote `{repo}`, but `{repo}`'s work will land on its branch. Promote `{repo}` into `{}`?",
        parent.name, parent.name
    );
    if !ctx.confirm(
        &question,
        Some("This promotes the repo into the parent feature."),
    )? {
        return Err(parent_promotion_required(child, parent, repo));
    }
    promote::promote(
        ctx,
        PromoteInput {
            feature: parent.name.to_string(),
            repo: repo.to_string(),
            base: None,
        },
    )
    .map(|_| ())
    .map_err(|failure| {
        Failure::failed(
            "integration.parent_promotion_failed",
            format!(
                "promoting `{repo}` into `{}` failed; no receipt was recorded",
                parent.name
            ),
        )
        .actual(failure.what.clone())
        .fix(
            FixAction::safe(
                "integration.promote_manually",
                format!(
                    "Run `ivar feature promote {} {repo}`, then integrate again.",
                    parent.name
                ),
            )
            .command(format!("ivar feature promote {} {repo}", parent.name)),
        )
    })
}

/// The refusal for a missing parent promotion on a non-interactive run: the
/// exact safe fix command, and nothing mutated.
fn parent_promotion_required(child: &Feature, parent: &Feature, repo: &RepoName) -> Failure {
    Failure::blocked(
        "integration.parent_promotion_required",
        format!(
            "`{repo}` is not promoted into `{}`, which must receive `{}`'s work",
            parent.name, child.name
        ),
    )
    .expected("the parent to promote every repo the child promotes")
    .actual(format!("`{repo}` is missing from `{}`", parent.name))
    .fix(
        FixAction::safe(
            "integration.promote_parent",
            format!(
                "Promote `{repo}` into `{}`, then integrate again.",
                parent.name
            ),
        )
        .command(format!("ivar feature promote {} {repo}", parent.name)),
    )
}

/// The resolved policy for this run: the first receipt freezes it; otherwise
/// per-field CLI > feature > hall > embedded.
fn resolved_policy(
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

/// Whether a feature has any session view dir — live or detached, the
/// unrestricted-session fact.
fn has_live_sessions(layout: &Layout, feature: &FeatureName) -> Result<bool, Failure> {
    let dir = layout.feature_sessions_dir(feature);
    if !fs::is_dir(&dir)? {
        return Ok(false);
    }
    Ok(!fs::read_dir(&dir)?.is_empty())
}

/// Re-validate every receipt after the per-repo pass; close as integrated
/// only when all are fresh and passing. Never reopens; a rerun with reused
/// receipts reports without closing.
fn final_state(
    ctx: &Ctx,
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    child: &Feature,
    parent: &Feature,
    warnings: &mut Vec<Warning>,
) -> Result<(FeatureIntegrationState, bool), Failure> {
    if read_close(layout, &child.name)?.is_some() {
        return Ok((FeatureIntegrationState::Integrated, false));
    }

    let mut all_fresh = !child.promotions.is_empty();
    for (repo, promotion) in &child.promotions {
        let Some(receipt) = &promotion.integration_receipt else {
            all_fresh = false;
            continue;
        };
        let freshness =
            relations::receipt_freshness(git, layout, manifest, child, parent, repo, receipt)?;
        if freshness != relations::ReceiptFreshness::Fresh {
            all_fresh = false;
        }
    }

    if !all_fresh {
        return Ok((FeatureIntegrationState::Active, false));
    }

    // Every promotion is receipted, fresh, and passing: close as integrated.
    let report = close::close(
        ctx,
        CloseInput {
            name: child.name.to_string(),
            outcome: "integrated".to_owned(),
        },
    )?;
    if !report.value.already_closed {
        warnings.push(Warning::new(
            "integration.closed_integrated",
            child.name.to_string(),
            "Closed the child as integrated; the outcome is final.".to_owned(),
        ));
    }
    Ok((FeatureIntegrationState::Integrated, true))
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/feature/integrate.rs"]
mod tests;
