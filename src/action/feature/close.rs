//! `ivar feature close <name> --outcome delivered|abandoned` — close a feature.
//!
//! Closing stops the feature's live executor sessions (their view dirs under
//! `features/<name>/sessions/`), removes its execution board
//! (`features/<name>/execution/`), and records the outcome on `plan.md`'s
//! frontmatter — the one committed artifact that says the feature is done and
//! how it ended. The promotion record (`feature.json`) is deliberately left
//! alone: the feature's worktrees and branches stay on disk until a human
//! removes them, exactly as `demote` keeps a worktree.
//!
//! # Idempotency
//!
//! A feature whose `plan.md` frontmatter already carries an `outcome` is
//! already closed; a second `close` is a no-op report, never an overwrite of
//! the recorded outcome or another pass at the session dirs.

use std::io;

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::domain::feature::{Feature, PromotionOutcome};
use crate::domain::name::FeatureName;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::{frontmatter, fs};

use super::super::discover_hall;
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

/// The slice of `plan.md`'s frontmatter `close` reads and writes.
///
/// `outcome` and `closed_at` are plain strings here — the frontmatter module's
/// own test shape — so a `plan.md` closed by any tool (or a hand-written
/// `outcome: shipped`) still reads back as "already closed" instead of failing
/// the parse. The validated [`PromotionOutcome`] is what `close` serializes.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
struct PlanFrontmatter {
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    closed_at: Option<String>,
}

/// Close `input.name` with `input.outcome`.
///
/// Blocked when the feature does not exist, or when `input.outcome` is not one
/// of the two known outcomes — each names its way out before anything is
/// touched. Already-closed features are a no-op report.
pub fn close(ctx: &Ctx, input: CloseInput) -> Outcome<CloseOutcome> {
    let layout = discover_hall(ctx)?;
    let name = FeatureName::new(input.name)?;
    let outcome = PromotionOutcome::parse(&input.outcome)?;

    // Closing is a lifecycle act on an existing feature; a feature that was
    // never created has nothing to close.
    Feature::read(&layout, &name)?.ok_or_else(|| {
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

    let plan_path = layout.plan_dir(&name).join("plan.md");
    let plan_source = fs::read_text(&plan_path)?.unwrap_or_default();

    // Idempotency gate: an outcome already recorded means the feature is
    // already closed, so the rest of this verb must not run again.
    let frontmatter = frontmatter::parse::<PlanFrontmatter>(&plan_source)?;
    if frontmatter.outcome.is_some() {
        return Ok(Report::new(CloseOutcome {
            root: layout.root().to_path_buf(),
            name,
            outcome,
            closed_at: frontmatter.closed_at.unwrap_or_default(),
            already_closed: true,
        }));
    }

    // Stop live executor sessions and drop the execution board. Removing the
    // sessions tree is what stops the sessions — liveness is a filesystem
    // fact (a view dir exists), so removing it is the stop.
    fs::remove_path(&layout.feature_sessions_dir(&name))?;
    fs::remove_path(&layout.execution_dir(&name))?;

    // Record the outcome, keeping the plan body byte-for-byte.
    let closed_at = rfc3339_now();
    let updated = PlanFrontmatter {
        outcome: Some(outcome.to_string()),
        closed_at: Some(closed_at.clone()),
    };
    let rendered = frontmatter::replace(&plan_source, &updated)?;
    fs::ensure_dir(&layout.plan_dir(&name))?;
    fs::write_text(&plan_path, &rendered)?;

    Ok(Report::new(CloseOutcome {
        root: layout.root().to_path_buf(),
        name,
        outcome,
        closed_at,
        already_closed: false,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/close.rs"]
mod tests;
