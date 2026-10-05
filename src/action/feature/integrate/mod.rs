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
    Feature, FeatureIntegrationState, GateState, IntegrationOverride, IntegrationPolicy,
    IntegrationStrategy, IntegrationVia, RunReceipt, VerificationEvidence,
};
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::git::{self, Git};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::{discover_hall, read_manifest};
use super::close::{self, CloseInput};
use super::lifecycle::read_close;
use super::parent_promotion;
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
    /// The integration title — the squash or merge commit message on the
    /// parent, and with `--via pr` the PR title and merge subject. Defaults
    /// to `feat: integrate <child>`.
    pub name: Option<String>,
}

pub(crate) fn default_title(feature: &FeatureName) -> String {
    format!("feat: integrate {feature}")
}

/// Refuse a title that carries AI attribution — the same lines
/// `ivar feature deliver` refuses — before anything moves. Deliberately
/// checked first, ahead of the root, plan-gate and descendant refusals: a
/// bad title is wrong whatever the tree's state.
fn refuse_attribution(feature: &FeatureName, title: &str) -> Result<(), Failure> {
    let findings: Vec<&str> = title
        .lines()
        .filter(|line| super::deliver::attribution::is_attribution(line))
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
            writeln!(
                w,
                "Closed `{}` as integrated; the outcome is final.",
                self.feature
            )?;
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
    let title = input.name.unwrap_or_else(|| default_title(&name));
    refuse_attribution(&name, &title)?;
    // 1. The child and its immediate parent. The tree is validated by the
    // read: a missing parent or a cycle refuses before anything else.
    relations::read_all(&layout)?;
    let parent_name = relations::read_feature(&layout, &name)?
        .parent
        .ok_or_else(|| {
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
    // Everything below reads and moves the parent, so it runs under the
    // parent's lock, and the child is re-read under it.
    let _parent_lock = parent_integration_lock(&layout, &parent_name)?;
    let child = relations::read_feature(&layout, &name)?;
    let parent = relations::read_feature(&layout, &parent_name)?;

    // 2. The plan gate must be approved — integration is a planned act, and
    // the artifact a human crossed is the gate (see ARCHITECTURE.md, seam 7).
    let plan_gate = crate::action::plan::effective_plan_gate(&layout, &name)?;
    if plan_gate != GateState::Approved {
        return Err(Failure::blocked(
            "integration.plan_not_approved",
            format!("integrating `{name}` needs its plan gate approved"),
        )
        .expected("the `plan` gate in state approved")
        .actual(format!("the plan gate is `{plan_gate}`"))
        .fix(
            FixAction::safe(
                "integration.approve_plan",
                format!("Approve it with `ivar plan approve {name} plan`, then integrate again."),
            )
            .command(format!("ivar plan approve {name} plan")),
        ));
    }

    // 3. Leaves first: every blocking descendant refuses the whole run, and
    // the failure names each blocker.
    let blockers = relations::blocking_descendants(&git, &layout, &manifest, &child)?;
    if !blockers.is_empty() {
        return Err(relations::tree_block_failure(&name, &blockers));
    }

    // 4 & 5. No live session that could still write a first receipt, and no
    // non-terminal run already holding the child's lock.
    ensure_no_conflicting_session_or_run(&layout, &name, &parent_name, &child)?;

    // 6. Resolve the policy once. The resolved relationship/base/policy is
    // frozen by the first persisted receipt: a rerun reuses each receipt's
    // own via/strategy instead of re-resolving.
    let policy = resolved_policy(
        &child,
        manifest.integration(),
        input.via.as_deref(),
        input.strategy.as_deref(),
    )?;

    // 7. Preflight every repo in two passes, so a later repo's refusal can
    // never leave an earlier repo's parent promotion behind.
    let needs_parent_promotion = preflight_repos(&layout, &manifest, &git, &child, &parent)?;
    needs_parent_promotion.iter().try_for_each(|repo| {
        parent_promotion::ensure(
            ctx,
            parent_promotion::Caller::Integrate,
            &child,
            &parent,
            repo,
        )
        .map(|_| ())
    })?;

    // 7. Per-repo, in name order: reuse, re-verify, or resume. Each result is
    // persisted immediately — partial and resumable, never atomic. The child
    // is re-read after each repo so the next persist carries every earlier
    // receipt, never clobbering it.
    let repos_out = run_integration_repos(
        &layout, &manifest, &git, &child, &parent, policy, &title, &name,
    )?;
    let child = relations::read_feature(&layout, &name)?;

    // 13. Close as integrated only when every receipt is fresh and passing.
    let (state, closed_integrated) = final_state(ctx, &layout, &manifest, &git, &child, &parent)?;

    Ok(Report::new(IntegrateOutcome {
        root: layout.root().to_path_buf(),
        feature: name,
        parent: parent_name,
        policy,
        repos: repos_out,
        state,
        closed_integrated,
    }))
}

const INTEGRATE_LOCK: &str = "integrate.lock";

/// Block until this process holds `parent`'s integration lock, which
/// serializes every integrate into that parent. Released when dropped.
///
/// # Errors
///
/// Returns [`Failure`] if the lock file cannot be created or locked.
pub(crate) fn parent_integration_lock(
    layout: &Layout,
    parent: &FeatureName,
) -> Result<std::fs::File, Failure> {
    Ok(fs::lock_exclusive(
        &layout.feature_dir(parent).join(INTEGRATE_LOCK),
    )?)
}

/// Whether an integrate into `parent` holds its lock right now.
pub(crate) fn parent_integration_running(layout: &Layout, parent: &FeatureName) -> bool {
    std::fs::File::open(layout.feature_dir(parent).join(INTEGRATE_LOCK))
        .is_ok_and(|file| matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)))
}

/// Pass 1 of the whole-run preflight for one repo: every refusal that does
/// not require a parent promotion to already exist — a stale receipt, an
/// unresumable failed receipt, or a dirty worktree refuses the entire run.
/// Returns whether `repo` still needs pass 2's parent-promotion question:
/// recorded here rather than asked, so a later repo's refusal in this same
/// pass can never leave an earlier repo's parent promotion behind.
fn preflight_repo(
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
fn preflight_repos(
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

/// Integrate every promoted repo, in name order: reuse, re-verify, or
/// resume. Each result is persisted immediately — partial and resumable,
/// never atomic. `child` is re-read after each repo so the next persist
/// carries every earlier receipt, never clobbering it. A repo that breaks
/// mid-run (a conflict, a refused PR) does not stop the batch, but fails the
/// run once every repo has had its turn.
#[allow(clippy::too_many_arguments)]
fn run_integration_repos(
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
        FixAction::safe(
            "integration.retry",
            format!("Fix the cause in the child, then run `ivar feature integrate {name}` again."),
        )
        .command(format!("ivar feature integrate {name}")),
    ))
}

/// A failed receipt is resumable only while its source and result are
/// unchanged; either moving means the evidence is stale, refused with
/// restoration orientation. Shared by the preflight check and the actual
/// integration, which both re-derive freshness before reusing failed evidence.
fn ensure_failed_receipt_still_current(
    git: &impl Git,
    layout: &Layout,
    bare: &camino::Utf8Path,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    receipt: &crate::domain::feature::IntegrationReceipt,
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

/// An unrestricted live session cannot coexist with a first successful
/// receipt, because it could still write a locked promotion — refused before
/// any repo can gain one. And a non-terminal run already holding the lock on
/// the child feature is refused early, before any policy resolution, git
/// preflight, or parent mutation.
fn ensure_no_conflicting_session_or_run(
    layout: &Layout,
    name: &FeatureName,
    parent: &FeatureName,
    child: &Feature,
) -> Result<(), Failure> {
    if !child.has_any_receipt()
        && !super::relations::feature_session_entries(layout, name)?.is_empty()
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
    close::close(
        ctx,
        CloseInput {
            name: child.name.to_string(),
            outcome: "integrated".to_owned(),
        },
    )?;
    Ok((FeatureIntegrationState::Integrated, true))
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/feature/integrate.rs"]
mod tests;
