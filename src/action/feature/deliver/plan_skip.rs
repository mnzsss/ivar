//! `--no-plan`: the human's explicit waiver of the plan gate for a feature
//! that never wrote a plan.

use super::input::DeliverInput;
use crate::domain::feature::Gate;
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction};
use crate::store::layout::Layout;

/// Validates `--no-plan` and says whether it was given. Refuses with
/// `--preview` and with a blank reason, like `--skip-memory-review`.
pub(crate) fn check(input: &DeliverInput) -> Result<bool, Failure> {
    let Some(reason) = &input.no_plan else {
        return Ok(false);
    };
    if input.preview {
        return Err(Failure::blocked(
            "deliver.no_plan_with_preview",
            "`--no-plan` applies a delivery; it cannot be combined with `--preview`",
        )
        .fix(FixAction::safe(
            "deliver.no_plan_at_apply",
            "Preview without it, then pass `--no-plan \"<reason>\"` with `--fingerprint`.",
        )));
    }
    if reason.trim().is_empty() {
        return Err(Failure::blocked(
            "deliver.no_plan_reason_required",
            "`--no-plan` needs a reason",
        )
        .fix(FixAction::safe(
            "deliver.give_no_plan_reason",
            "Say why this delivery needs no plan: `--no-plan \"<reason>\"`.",
        )));
    }
    Ok(true)
}

/// Whether the feature has any SPDD artifact on disk, read live.
pub(crate) fn any_artifact(layout: &Layout, feature: &FeatureName) -> bool {
    [Gate::Requirements, Gate::Analysis, Gate::Plan]
        .into_iter()
        .any(|gate| crate::action::plan::artifact_path(layout, feature, gate).exists())
}
