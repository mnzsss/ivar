//! `ivar feature close <name> --outcome delivered|integrated|abandoned` —
//! close a feature.
//!
//! Closing stops the feature's live executor sessions (their view dirs under
//! `features/<name>/sessions/`) and records the outcome on `plan.md`'s
//! frontmatter. Run receipts and their archive are durable execution evidence:
//! close preserves them, and refuses while a receipt still holds the run lock.
//! The promotion record (`feature.json`) is deliberately left alone: the
//! feature's worktrees and branches stay on disk until a human removes them.
//!
//! # Idempotency
//!
//! A feature whose `plan.md` frontmatter already carries an `outcome` is
//! already closed; a second `close` is a no-op report, never an overwrite of
//! the recorded outcome or another pass at the session dirs.
//!
//! # The `integrated` outcome
//!
//! Only a child with a fresh passing receipt on every promotion may close as
//! `integrated` — a direct close must not fabricate integration evidence.
//! The frontmatter read/write itself lives in [`super::lifecycle`], shared
//! with tree classification.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{Feature, PromotionOutcome, RunReceipt};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;
use crate::store::feature::run;

use super::super::discover_hall;
use super::lifecycle::{read_close, write_close};
use crate::action::Ctx;

/// What `ivar feature close` needs.
#[derive(Debug, Clone)]
pub struct CloseInput {
    /// The feature's name.
    pub name: String,
    /// The outcome, unvalidated — [`PromotionOutcome`] is this module's job.
    pub outcome: String,
}

/// What `ivar feature close` did.
#[derive(Debug, Clone, Serialize)]
pub struct CloseOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature that was closed.
    pub name: FeatureName,
    /// The recorded outcome.
    pub outcome: PromotionOutcome,
    /// When it was closed, as an RFC 3339 timestamp.
    pub closed_at: String,
    /// Whether the feature was already closed — this run was a no-op.
    pub already_closed: bool,
}

impl WriteHuman for CloseOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.already_closed {
            writeln!(
                w,
                "Feature `{}` was already closed ({}) at {} — nothing to do",
                self.name, self.outcome, self.closed_at
            )
        } else {
            writeln!(
                w,
                "Closed feature `{}` ({}) at {}",
                self.name, self.outcome, self.closed_at
            )
        }
    }
}

/// Close `input.name` with `input.outcome`.
///
/// Blocked when the feature does not exist, when `input.outcome` is not one
/// of the three known outcomes, or — for `integrated` — when the feature is
/// not a child carrying a passing receipt on every promotion. Each names its
/// way out before anything is touched. Already-closed features are a no-op
/// report that never overwrites the recorded outcome.
pub fn close(ctx: &Ctx, input: CloseInput) -> Outcome<CloseOutcome> {
    let layout = discover_hall(ctx)?;
    let name = FeatureName::new(input.name)?;
    let outcome = PromotionOutcome::parse(&input.outcome)?;

    // Closing is a lifecycle act on an existing feature; a feature that was
    // never created has nothing to close.
    let feature = Feature::read(&layout, &name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
        .expected("an existing feature")
        .actual(format!("`{name}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.create_first",
            format!("Create it first with `ivar feature create {name}`."),
        ))
    })?;

    // Preserve legacy execution evidence before considering close state.
    let _ = run::import(
        &layout,
        &name,
        layout.plan_dir(&name).join("plan.md"),
        crate::domain::feature::RunId::new(uuid::Uuid::new_v4().to_string())?,
        &crate::domain::session::rfc3339_now(),
    )?;
    if let Some(receipt) = RunReceipt::read(&layout, &name)?
        && receipt.holds_lock()
    {
        return Err(Failure::blocked(
            "feature.close_run_active",
            format!(
                "feature `{name}` has a {} run (`{}`)",
                receipt.status, receipt.id
            ),
        )
        .expected("a terminal run receipt before closing the feature")
        .actual("the current run is still resumable and holds the feature lock")
        .fix(FixAction::safe(
            "execute.finish_or_interrupt",
            "Finish, accept the revision, or interrupt the run before closing the feature.",
        )));
    }

    // Idempotency gate: an outcome already recorded means the feature is
    // already closed, so the rest of this verb must not run again. The
    // recorded outcome (not the requested one) is what gets reported.
    if let Some(record) = read_close(&layout, &name)? {
        return Ok(Report::new(CloseOutcome {
            root: layout.root().to_path_buf(),
            name,
            outcome: record
                .known_outcome()
                .unwrap_or(PromotionOutcome::Abandoned),
            closed_at: record.closed_at,
            already_closed: true,
        }));
    }

    // A direct close may not fabricate integration evidence: `integrated`
    // requires a child whose every promotion carries a passing receipt.
    if outcome == PromotionOutcome::Integrated {
        if feature.parent.is_none() {
            return Err(Failure::blocked(
                "feature.close_integrated_child_required",
                format!("feature `{name}` is not a child, so it cannot close as `integrated`"),
            )
            .expected("a child feature (one with a parent) to close as integrated")
            .actual("this feature has no parent")
            .fix(FixAction::safe(
                "feature.close_delivered_or_abandoned",
                "A root closes as `delivered` (or `abandoned`). A child closes as `integrated` only through `ivar feature integrate`.",
            )));
        }
        if !feature.all_promotions_have_passing_receipts() {
            return Err(Failure::blocked(
                "feature.close_integrated_receipts_required",
                format!(
                    "feature `{name}` has no passing integration receipt on every promotion, so it cannot close as `integrated`"
                ),
            )
            .expected("a fresh passing receipt for every promoted repo")
            .actual("at least one promotion is unreceipted or carries failed evidence")
            .fix(FixAction::safe(
                "feature.integrate_first",
                format!("Integrate the child first with `ivar feature integrate {name}`."),
            )));
        }
    }

    // Removing session views stops live sessions. Execution evidence remains
    // intact for audit and history, even after a terminal run.
    fs::remove_path(&layout.feature_sessions_dir(&name))?;

    // Record the outcome, keeping the plan body byte-for-byte.
    let record = write_close(&layout, &name, outcome)?;

    Ok(Report::new(CloseOutcome {
        root: layout.root().to_path_buf(),
        name,
        outcome,
        closed_at: record.closed_at,
        already_closed: false,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/close.rs"]
mod tests;
