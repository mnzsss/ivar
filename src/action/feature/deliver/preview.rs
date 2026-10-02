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

/// Everything that refuses apply, in the order apply checks it. The preview
/// prints this list, and apply refuses exactly when it is non-empty.
pub(crate) fn blockers(preview: &DeliveryPreview) -> Vec<String> {
    let descendants = preview.tree_blockers.iter().map(|blocker| {
        format!(
            "descendant `{}` is {}: {}",
            blocker.feature, blocker.state, blocker.reason
        )
    });
    let plan = (preview.plan_gate != GateState::Approved)
        .then(|| format!("the plan gate is {}, not approved", preview.plan_gate));
    let land = (preview.mode == DeliveryMode::Land && preview.repos.is_empty())
        .then(|| "no repositories are promoted to land".to_owned());
    let repos = preview.repos.iter().flat_map(|repo| {
        repo.blockers
            .iter()
            .map(move |blocker| format!("{}: {blocker}", repo.repo))
    });
    descendants.chain(plan).chain(land).chain(repos).collect()
}

pub(crate) fn blocked(feature: &FeatureName, blockers: &[String]) -> Failure {
    Failure::blocked(
        "deliver.blocked",
        format!(
            "cannot deliver `{feature}`: {} blocker(s) listed by the preview",
            blockers.len()
        ),
    )
    .expected("a preview with no blockers")
    .actual(blockers.join("; "))
    .fix(FixAction::safe(
        "deliver.clear_blockers",
        format!(
            "Resolve each blocker, then run `ivar feature deliver {feature} --preview` and apply again."
        ),
    ))
}

/// Delivering a feature whose plan gate is not approved, refused.
///
/// `ivar` has no persisted lifecycle field; this *is* the lifecycle, read from
/// the artifact a human crossed. See ARCHITECTURE.md, seam 7.
pub(crate) fn plan_not_approved(
    feature: &FeatureName,
    state: GateState,
    plan_written: bool,
) -> Failure {
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
    .fix(if plan_written {
        FixAction::safe(
            "deliver.approve_plan",
            format!(
                "Approve it with `ivar plan approve {feature} plan`, then preview and apply again."
            ),
        )
        .command(format!("ivar plan approve {feature} plan"))
    } else {
        FixAction::safe(
            "deliver.write_plan",
            format!(
                "Write a plan with `ivar plan create {feature} plan`, approve it with `ivar plan approve {feature} plan`, then preview and apply again."
            ),
        )
        .command(format!("ivar plan create {feature} plan"))
    })
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
