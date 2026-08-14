//! The shared plan-frontmatter close seam: one interpretation of "how did
//! this feature end".
//!
//! `ivar feature close` records the outcome on `plan.md`'s frontmatter — the
//! one committed artifact that says a feature is done and how it ended. That
//! record is read back in three places, and all three must agree on what it
//! means:
//!
//! - `close` itself, for idempotency (an outcome already recorded is a
//!   no-op, never an overwrite);
//! - tree classification (`action::feature::relations`), where a close
//!   record's outcome classifies the feature's derived integration state;
//! - the fully-integrated guard (`is_fully_integrated`), which Task 8's
//!   mutation module and the integration close path both consult.
//!
//! The read shape keeps `outcome` as a plain string on purpose: a `plan.md`
//! closed by any tool — or hand-written — still reads back as "already
//! closed" instead of failing the parse. [`CloseRecord::known_outcome`] is
//! where the string becomes a [`PromotionOutcome`], and `None` there means
//! "closed, but not by one of our outcomes" — still a close, never a
//! classification.

use crate::domain::feature::{Feature, PromotionOutcome};
use crate::domain::name::FeatureName;
use crate::domain::session::rfc3339_now;
use crate::error::Failure;
use crate::infra::{frontmatter, fs};
use crate::store::layout::Layout;

/// The slice of `plan.md`'s frontmatter the close seam reads and writes.
///
/// `outcome` and `closed_at` are plain strings here — the frontmatter module's
/// own test shape — so a `plan.md` closed by any tool (or a hand-written
/// `outcome: shipped`) still reads back as "already closed" instead of failing
/// the parse. The validated [`PromotionOutcome`] is what `write_close`
/// serializes.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct PlanFrontmatter {
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    closed_at: Option<String>,
}

/// A recorded close: the outcome string exactly as it appears in the
/// frontmatter, and when it was closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CloseRecord {
    /// The outcome string, verbatim — see the module doc for why it stays a
    /// string on read.
    pub outcome: String,
    /// When the feature was closed, as an RFC 3339 timestamp.
    pub closed_at: String,
}

impl CloseRecord {
    /// The recorded outcome as a known [`PromotionOutcome`], or `None` when
    /// the string is not one of ours (still a close, just not classifiable).
    #[must_use]
    pub fn known_outcome(&self) -> Option<PromotionOutcome> {
        PromotionOutcome::parse(&self.outcome).ok()
    }
}

/// Read the close record from `plans/<feature>/plan.md`'s frontmatter.
/// `Ok(None)` when the feature has no close record — no plan file, or a plan
/// file whose frontmatter carries no `outcome`.
pub(crate) fn read_close(
    layout: &Layout,
    feature: &FeatureName,
) -> Result<Option<CloseRecord>, Failure> {
    let plan_path = layout.plan_dir(feature).join("plan.md");
    let Some(plan_source) = fs::read_text(&plan_path)? else {
        return Ok(None);
    };
    let frontmatter = frontmatter::parse::<PlanFrontmatter>(&plan_source)?;
    let Some(outcome) = frontmatter.outcome else {
        return Ok(None);
    };
    Ok(Some(CloseRecord {
        outcome,
        closed_at: frontmatter.closed_at.unwrap_or_default(),
    }))
}

/// Record a close: write `outcome` and a fresh timestamp onto
/// `plans/<feature>/plan.md`'s frontmatter, keeping the body byte-for-byte.
/// Returns the record that was written.
pub(crate) fn write_close(
    layout: &Layout,
    feature: &FeatureName,
    outcome: PromotionOutcome,
) -> Result<CloseRecord, Failure> {
    let plan_path = layout.plan_dir(feature).join("plan.md");
    let plan_source = fs::read_text(&plan_path)?.unwrap_or_default();

    let closed_at = rfc3339_now();
    let updated = PlanFrontmatter {
        outcome: Some(outcome.to_string()),
        closed_at: Some(closed_at.clone()),
    };
    let rendered = frontmatter::replace(&plan_source, &updated)?;
    fs::ensure_dir(&layout.plan_dir(feature))?;
    fs::write_text(&plan_path, &rendered)?;

    Ok(CloseRecord {
        outcome: outcome.to_string(),
        closed_at,
    })
}

/// Whether the feature's close record carries the `integrated` outcome — the
/// whole-child immutability fact.
///
/// Only the recorded outcome counts: a child whose receipts are all fresh and
/// passing but that has not been closed is not yet "fully integrated" in the
/// lifecycle sense (receipt-freshness policy is the mutation module's job,
/// not a blanket guard here).
pub(crate) fn is_fully_integrated(layout: &Layout, feature: &Feature) -> Result<bool, Failure> {
    Ok(read_close(layout, &feature.name)?
        .and_then(|record| record.known_outcome())
        .is_some_and(|outcome| outcome == PromotionOutcome::Integrated))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/lifecycle.rs"]
mod tests;
