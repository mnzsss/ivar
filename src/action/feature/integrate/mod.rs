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

use crate::domain::feature::{Feature, FeatureIntegrationState, GateState};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report};
use crate::git::{self, Git};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::{discover_hall, read_manifest};
use super::close::{self, CloseInput};
use super::lifecycle::read_close;
use super::parent_promotion;
use super::relations;
use crate::action::Ctx;

mod local;
mod pr;
mod preflight;
mod receipts;
mod repos;
mod types;

pub(crate) use preflight::default_title;
pub use types::{IntegrateInput, IntegrateOutcome, RepoIntegration, RepoIntegrationStatus};

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
    preflight::refuse_attribution(&name, &title)?;
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
    preflight::ensure_no_conflicting_session_or_run(&layout, &name, &parent_name, &child)?;

    // 6. Resolve the policy once. The resolved relationship/base/policy is
    // frozen by the first persisted receipt: a rerun reuses each receipt's
    // own via/strategy instead of re-resolving.
    let policy = preflight::resolved_policy(
        &child,
        manifest.integration(),
        input.via.as_deref(),
        input.strategy.as_deref(),
    )?;

    // 7. Preflight every repo in two passes, so a later repo's refusal can
    // never leave an earlier repo's parent promotion behind.
    let needs_parent_promotion =
        preflight::preflight_repos(&layout, &manifest, &git, &child, &parent)?;
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
    let repos_out = repos::run_integration_repos(
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
