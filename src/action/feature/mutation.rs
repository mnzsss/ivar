//! The mutation boundaries for a feature that has begun — or finished —
//! integrating. Scoped guards, never one blanket "any receipt means no
//! mutations".
//!
//! The plan's model has three scopes, and collapsing them would freeze work
//! that must stay movable:
//!
//! - **The whole child** — a feature closed as `integrated` is immutable,
//!   full stop. [`ensure_not_fully_integrated`] is the narrow gate for plan
//!   mutations (planning may keep moving during a *partial* integration, just
//!   not after the `integrated` close record exists).
//! - **Structure** — relationship, base, policy, and promotion membership are
//!   feature-wide facts frozen by the *first* receipt of any kind.
//!   [`ensure_structure_mutable`] is the gate for reparenting, promote/demote
//!   membership changes, and feature-wide rebase.
//! - **One promotion** — a promotion carrying a *successful* receipt is
//!   individually immutable even when its receipt later goes stale;
//!   [`ensure_promotion_mutable`] is the gate, and a failed-evidence receipt
//!   does not lock its promotion, so repo B stays repairable while repo A
//!   stays byte-for-byte locked.
//!
//! [`ensure_unrestricted_session_allowed`] ensures an unrestricted session
//! cannot coexist with a successful partial state.

use crate::domain::feature::Feature;
use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction};
use crate::store::layout::Layout;

use super::lifecycle::is_fully_integrated;

/// Refuse when the feature is fully integrated (closed as `integrated`) —
/// the whole-child immutability fact. Planning mutations stay legal during a
/// *partial* integration.
pub(crate) fn ensure_not_fully_integrated(
    layout: &Layout,
    feature: &Feature,
) -> Result<(), Failure> {
    if is_fully_integrated(layout, feature)? {
        return Err(integration_immutable(feature));
    }
    Ok(())
}

/// Refuse when the feature is fully integrated *or* carries any receipt —
/// relationship/base/policy and promotion membership are feature-wide facts.
pub(crate) fn ensure_structure_mutable(layout: &Layout, feature: &Feature) -> Result<(), Failure> {
    if is_fully_integrated(layout, feature)? {
        return Err(integration_immutable(feature));
    }
    if feature.has_any_receipt() {
        return Err(structure_frozen(feature));
    }
    Ok(())
}

/// Refuse when the feature is fully integrated, or when `repo`'s promotion
/// carries a receipt with *successful* recorded evidence. Failed evidence
/// does not lock its promotion — that one stays repairable and resumable.
pub(crate) fn ensure_promotion_mutable(
    layout: &Layout,
    feature: &Feature,
    repo: &RepoName,
) -> Result<(), Failure> {
    if is_fully_integrated(layout, feature)? {
        return Err(integration_immutable(feature));
    }
    if feature.promotion_has_successful_receipt(repo) {
        return Err(promotion_immutable(feature, repo));
    }
    Ok(())
}

/// Refuse an unrestricted session on a feature that carries a successful
/// receipt — a session with no contract would be able to write a locked
/// promotion. Fresh children and failed-evidence-only children pass; any
/// successful receipt refuses, and so does a fully integrated child.
pub(crate) fn ensure_unrestricted_session_allowed(
    layout: &Layout,
    feature: &Feature,
) -> Result<(), Failure> {
    if is_fully_integrated(layout, feature)? {
        return Err(integration_immutable(feature));
    }
    for (repo, promotion) in &feature.promotions {
        if promotion
            .integration_receipt
            .as_ref()
            .is_some_and(|receipt| receipt.verification.passed())
        {
            return Err(session_unrestricted_blocked(feature, repo));
        }
    }
    Ok(())
}

/// The failure for a fully integrated child: the whole child is frozen, and
/// the pinned source/result SHAs are named so an accidental movement can be
/// recognised and repaired.
fn integration_immutable(feature: &Feature) -> Failure {
    let pinned = pinned_receipts(feature);
    Failure::blocked(
        "feature.integration_immutable",
        format!(
            "feature `{}` is fully integrated and cannot be mutated",
            feature.name
        ),
    )
    .expected("a feature that has not closed as `integrated`")
    .actual("the close record says `integrated`, which is final — there is no reopen command")
    .details(serde_json::json!({ "pinned": pinned }))
    .fix(
        FixAction::safe("feature.status", "Inspect the feature's tree health.")
            .command(format!("ivar feature status {} --recursive", feature.name)),
    )
}

/// The failure for a feature-wide structure mutation after any receipt:
/// relationship/base/policy and promotion membership froze with the first
/// receipt, whatever its outcome.
fn structure_frozen(feature: &Feature) -> Failure {
    let pinned = pinned_receipts(feature);
    Failure::blocked(
        "feature.integration_structure_frozen",
        format!(
            "feature `{}` has begun integrating; its parent, base, policy, and promotion membership are frozen",
            feature.name
        ),
    )
    .expected("a feature with no integration receipt yet")
    .actual("at least one promotion carries a receipt")
    .details(serde_json::json!({ "pinned": pinned }))
    .fix(FixAction::safe(
        "feature.reparent_before_work",
        "Reparent or change membership only while the child is pristine — before any receipt.",
    ))
}

/// The failure for one locked promotion: its successful receipt pins its
/// source and result, even if freshness later drifts.
fn promotion_immutable(feature: &Feature, repo: &RepoName) -> Failure {
    let receipt = feature
        .promotions
        .get(repo)
        .and_then(|promotion| promotion.integration_receipt.as_ref());
    let source = receipt.map(|r| r.source_sha.clone()).unwrap_or_default();
    let result = receipt.map(|r| r.result_sha.clone()).unwrap_or_default();
    Failure::blocked(
        "feature.promotion_integration_immutable",
        format!(
            "`{repo}`'s promotion in feature `{}` carries a successful integration receipt and cannot be mutated",
            feature.name
        ),
    )
    .expected("a promotion without successful integration evidence")
    .actual(format!(
        "`{repo}`'s receipt pins source {source} and result {result}"
    ))
    .details(serde_json::json!({
        "repo": repo,
        "source_sha": source,
        "result_sha": result,
    }))
    .fix(FixAction::safe(
        "feature.leave_locked_promotion",
        "Leave this promotion alone; repair and resume only unreceipted or failed promotions.",
    ))
}

/// The failure for an unrestricted session on a successful partial state.
fn session_unrestricted_blocked(feature: &Feature, repo: &RepoName) -> Failure {
    Failure::blocked(
        "feature.session_unrestricted_blocked",
        format!(
            "feature `{}` carries a successful integration receipt (on `{repo}`); an unrestricted session could write a locked promotion",
            feature.name
        ),
    )
    .expected("no successful receipt")
    .actual(format!("`{repo}` is locked by its receipt"))
    .fix(FixAction::safe(
        "feature.finish_integration",
        "Finish integration before starting another unrestricted session.",
    ))
}

/// The pinned source/result facts of every receipt, for orientation details.
fn pinned_receipts(feature: &Feature) -> Vec<serde_json::Value> {
    feature
        .promotions
        .iter()
        .filter_map(|(repo, promotion)| {
            promotion.integration_receipt.as_ref().map(|receipt| {
                serde_json::json!({
                    "repo": repo,
                    "source_sha": receipt.source_sha,
                    "result_sha": receipt.result_sha,
                })
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/mutation.rs"]
mod tests;
