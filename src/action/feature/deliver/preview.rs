//! The preview half of delivery: the fingerprint that gates apply, the plan
//! gate check, and the refusals when a preview (or an approved plan) is
//! missing.

use super::input::{DeliverInput, PullRequestMetadata};
use crate::domain::feature::{
    DeliveryMode, DeliveryPreview, DeliveryRepo, DeliveryTreeBlocker, GateState,
};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction};
use crate::infra::{hash, json};
use crate::store::layout::Layout;

pub(crate) fn apply_command(input: &DeliverInput, fingerprint: &str) -> String {
    let mut words = vec![
        "ivar".to_owned(),
        "feature".to_owned(),
        "deliver".to_owned(),
        shell_word(&input.feature),
    ];
    if input.land {
        words.push("--land".to_owned());
    }
    for repo in &input.only {
        words.push("--only".to_owned());
        words.push(shell_word(repo));
    }
    words.push("--fingerprint".to_owned());
    words.push(shell_word(fingerprint));
    push_metadata(&mut words, &input.global_metadata);
    for scoped in &input.repo_overrides {
        words.push("--repo".to_owned());
        words.push(shell_word(&scoped.repo));
        push_metadata(&mut words, &scoped.metadata);
    }
    words.join(" ")
}

fn push_metadata(words: &mut Vec<String>, metadata: &PullRequestMetadata) {
    if let Some(title) = &metadata.title {
        words.push("--name".to_owned());
        words.push(shell_word(title));
    }
    if let Some(body) = &metadata.body {
        words.push("--body".to_owned());
        words.push(shell_word(body));
    }
    if metadata.draft == Some(true) {
        words.push("--draft".to_owned());
    }
}

fn shell_word(value: &str) -> String {
    let plain = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@,+".contains(c));
    if plain {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

pub(crate) fn fingerprint_for(
    feature: &FeatureName,
    mode: DeliveryMode,
    plan_gate: GateState,
    tree_blockers: &[DeliveryTreeBlocker],
    repos: &[DeliveryRepo],
) -> Result<String, Failure> {
    let preview = DeliveryPreview {
        feature: feature.clone(),
        mode,
        plan_gate,
        repos: repos.to_vec(),
        tree_blockers: tree_blockers.to_vec(),
        fingerprint: String::new(),
    };
    let rendered = json::to_canonical_string(&preview)?;
    Ok(hash::text(&rendered))
}

pub(crate) fn plan_gate_state(
    layout: &Layout,
    feature: &FeatureName,
) -> Result<GateState, Failure> {
    crate::action::plan::effective_plan_gate(layout, feature)
}

/// Delivering a feature whose plan gate is not approved, refused.
///
/// `ivar` has no persisted lifecycle field; this *is* the lifecycle, read from
/// the artifact a human crossed. See ARCHITECTURE.md, seam 7.
pub(crate) fn plan_not_approved(feature: &FeatureName, state: GateState) -> Failure {
    let actual = match state {
        GateState::Pending => format!("`{feature}`'s plan gate has never been approved"),
        GateState::NeedsRevision => {
            format!("`{feature}`'s plan gate was approved, then invalidated by a revision")
        }
        GateState::Approved => format!("`{feature}`'s plan gate is approved"),
    };

    Failure::blocked(
        "deliver.plan_not_approved",
        format!("delivering `{feature}` needs its plan gate approved"),
    )
    .expected("the `plan` gate in state approved")
    .actual(actual)
    .fix(FixAction::safe(
        "deliver.approve_plan",
        format!(
            "Approve it with `ivar plan approve {feature} plan`, then preview and apply again."
        ),
    ))
}

pub(crate) fn preview_required(feature: &FeatureName) -> Failure {
    Failure::blocked(
        "deliver.preview_required",
        format!("delivering `{feature}` needs a preview fingerprint"),
    )
    .expected("the fingerprint printed by `ivar feature deliver --preview`")
    .actual("no `--fingerprint` was given")
    .fix(FixAction::safe(
        "deliver.preview_first",
        format!(
            "Run `ivar feature deliver {feature} --preview` and pass its fingerprint with `--fingerprint`."
        ),
    ))
}
