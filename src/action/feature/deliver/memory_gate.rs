//! The memory review that `ivar feature deliver` requires before apply.

use std::collections::BTreeMap;

use crate::action::memory::drain::drain;
use crate::action::memory::review::{ReviewState, feature_heads, review_state};
use crate::domain::feature::{Feature, MemoryReviewGate};
use crate::domain::name::FeatureName;
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
