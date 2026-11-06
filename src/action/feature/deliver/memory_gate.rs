//! The memory review that `ivar feature deliver` requires before apply.

use std::collections::BTreeMap;

use serde_json::json;

use super::input::DeliverInput;
use crate::action::memory::capture::CaptureEnv;
use crate::action::memory::drain::drain;
use crate::action::memory::events::{MemoryEventScope, report_memory_event};
use crate::action::memory::review::{ReviewState, feature_heads, review_state};
use crate::domain::feature::{Feature, MemoryReviewGate};
use crate::domain::name::FeatureName;
use crate::domain::observation::SourceEvent;
use crate::error::{Failure, FixAction, Warning};
use crate::store::layout::Layout;

pub(crate) fn memory_gate(
    layout: &Layout,
    name: &FeatureName,
    feature: &Feature,
) -> (MemoryReviewGate, Vec<Warning>) {
    let mut warnings = Vec::new();
    if let Err(failure) = drain(layout) {
        warnings.push(Warning::new(
            "deliver.memory_drain_failed",
            name.to_string(),
            format!(
                "the capture spool could not be drained, so a recent review may be missed: {}",
                failure.what
            ),
        ));
    }
    let state = review_state(layout, name).unwrap_or_else(|failure| {
        warnings.push(Warning::new(
            "deliver.memory_unreadable",
            name.to_string(),
            format!("memory could not be read: {}", failure.what),
        ));
        ReviewState::Missing
    });
    match state {
        ReviewState::Reviewed { heads } => {
            warnings.extend(moved_heads(name, &heads, &feature_heads(layout, feature)));
            (MemoryReviewGate::Reviewed, warnings)
        }
        ReviewState::Skipped | ReviewState::Missing => (MemoryReviewGate::Missing, warnings),
    }
}

fn moved_heads(
    name: &FeatureName,
    reviewed: &BTreeMap<String, String>,
    current: &BTreeMap<String, String>,
) -> Vec<Warning> {
    current
        .iter()
        .filter_map(|(repo, head)| {
            let then = reviewed.get(repo)?;
            (then != head).then(|| {
                Warning::new(
                    "deliver.memory_review_outdated",
                    repo.clone(),
                    format!(
                        "`{repo}` moved from {then} to {head} since the memory review of `{name}`; run memory_review again if the new commits change what memory says"
                    ),
                )
            })
        })
        .collect()
}

pub(crate) fn memory_not_reviewed(feature: &FeatureName) -> Failure {
    Failure::blocked(
        "deliver.memory_not_reviewed",
        format!("delivering `{feature}` needs its memory reviewed"),
    )
    .expected("a `memory.reviewed` observation for the feature")
    .actual(format!("`{feature}` has no memory review"))
    .fix(FixAction::safe(
        "deliver.review_memory",
        format!(
            "In a session of `{feature}`, call the memory_review MCP tool, give each stale page a verdict and confirm; then preview and apply again."
        ),
    ))
}

pub(crate) fn check_skip(input: &DeliverInput) -> Result<bool, Failure> {
    let Some(reason) = &input.skip_memory_review else {
        return Ok(false);
    };
    if input.preview {
        return Err(Failure::blocked(
            "deliver.skip_memory_review_with_preview",
            "`--skip-memory-review` applies a delivery; it cannot be combined with `--preview`",
        )
        .fix(FixAction::safe(
            "deliver.skip_at_apply",
            "Preview without it, then pass `--skip-memory-review \"<reason>\"` with `--fingerprint`.",
        )));
    }
    if reason.trim().is_empty() {
        return Err(Failure::blocked(
            "deliver.skip_memory_review_reason_required",
            "`--skip-memory-review` needs a reason",
        )
        .fix(FixAction::safe(
            "deliver.give_skip_reason",
            "Say why this delivery needs no memory review: `--skip-memory-review \"<reason>\"`.",
        )));
    }
    Ok(true)
}

pub(crate) fn record_skip(layout: &Layout, feature: &FeatureName, reason: &str) {
    let scope = MemoryEventScope {
        session_id: CaptureEnv::from_process().session_id.unwrap_or_default(),
        feature: feature.to_string(),
        parent_feature: String::new(),
    };
    report_memory_event(
        layout,
        &scope,
        SourceEvent::MemoryReviewSkipped,
        json!({ "reason": reason }),
    );
}
