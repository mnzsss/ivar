//! Shared feature selector resolution across interactive and non-interactive invocations.

use crate::action::Ctx;
use crate::action::confirm::SelectOption;
use crate::action::feature::list::{self as feature_list, FeatureSummary};
use crate::error::Failure;

/// Resolve a single feature name.
///
/// If `explicit` is provided (`Some`), returns it immediately.
/// If `explicit` is `None`:
/// - When non-interactive, returns `feature.missing_argument` failure.
/// - Otherwise lists features from the hall. If empty, returns `feature.no_features_available` failure.
/// - Prompts the user via `ctx.confirm.select_one(prompt, &options)`.
/// - If cancelled/None, returns `feature.selection_cancelled` failure.
pub fn resolve_single_feature(
    ctx: &Ctx,
    explicit: Option<String>,
    prompt: &str,
) -> Result<String, Failure> {
    if let Some(name) = explicit {
        return Ok(name);
    }

    if !ctx.confirm.is_interactive() {
        return Err(Failure::blocked(
            "feature.missing_argument",
            "feature name is required when non-interactive",
        ));
    }

    let report = feature_list::list(ctx)?;
    let features = report.value.features;
    if features.is_empty() {
        return Err(Failure::blocked(
            "feature.no_features_available",
            "no features exist in this hall",
        ));
    }

    let options: Vec<SelectOption> = features.iter().map(summary_to_select_option).collect();

    let chosen_idx = ctx.confirm.select_one(prompt, &options)?
        .ok_or_else(|| Failure::blocked("feature.selection_cancelled", "feature selection cancelled"))?;

    let selected = options
        .get(chosen_idx)
        .ok_or_else(|| Failure::blocked("feature.selection_cancelled", "invalid selection index"))?;

    Ok(selected.id.clone())
}

/// Resolve one or more feature names.
///
/// If `explicit` is provided (`Some`), returns `vec![explicit]` immediately.
/// If `explicit` is `None`:
/// - When non-interactive, returns `feature.missing_argument` failure.
/// - Otherwise lists features from the hall. If empty, returns `feature.no_features_available` failure.
/// - Prompts the user via `ctx.confirm.select_many(prompt, &options)`.
/// - If empty/cancelled, returns `feature.selection_cancelled` failure.
pub fn resolve_multi_features(
    ctx: &Ctx,
    explicit: Option<String>,
    prompt: &str,
) -> Result<Vec<String>, Failure> {
    if let Some(name) = explicit {
        return Ok(vec![name]);
    }

    if !ctx.confirm.is_interactive() {
        return Err(Failure::blocked(
            "feature.missing_argument",
            "feature name is required when non-interactive",
        ));
    }

    let report = feature_list::list(ctx)?;
    let features = report.value.features;
    if features.is_empty() {
        return Err(Failure::blocked(
            "feature.no_features_available",
            "no features exist in this hall",
        ));
    }

    let options: Vec<SelectOption> = features.iter().map(summary_to_select_option).collect();

    let chosen_indices = ctx.confirm.select_many(prompt, &options)?;
    if chosen_indices.is_empty() {
        return Err(Failure::blocked(
            "feature.selection_cancelled",
            "feature selection cancelled",
        ));
    }

    let mut selected = Vec::new();
    for idx in chosen_indices {
        if let Some(opt) = options.get(idx) {
            selected.push(opt.id.clone());
        }
    }

    Ok(selected)
}

fn summary_to_select_option(summary: &FeatureSummary) -> SelectOption {
    let description = format!(
        "{} ({} promoted, {} ready)",
        summary.state, summary.promoted_count, summary.ready_count
    );
    SelectOption {
        id: summary.name.to_string(),
        description: Some(description),
        path_if_any: String::new(),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/select.rs"]
mod tests;
