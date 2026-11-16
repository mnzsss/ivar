//! The preview half of delivery: the fingerprint that gates apply, the plan
//! gate check, and the refusals when a preview (or an approved plan) is
//! missing.

use super::input::{DeliverInput, PullRequestMetadata};
use crate::domain::feature::{
    DeliveryMode, DeliveryPreview, DeliveryRepo, DeliveryTreeBlocker, GateState, MemoryReviewGate,
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
    memory_review: MemoryReviewGate,
    tree_blockers: &[DeliveryTreeBlocker],
    repos: &[DeliveryRepo],
) -> Result<String, Failure> {
    let preview = DeliveryPreview {
        feature: feature.clone(),
        mode,
        plan_gate,
        repos: repos.to_vec(),
        tree_blockers: tree_blockers.to_vec(),
        memory_review,
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

/// What lifts a blocker at apply, and what the feature has on disk. One value
/// feeds both the preview and the apply, so the two never diverge.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Waivers {
    /// `--skip-memory-review` was given.
    pub skip_memory_review: bool,
    /// `--no-plan` was given.
    pub no_plan: bool,
    /// Any of `requirements.md`, `analysis.md` or `plan.md` exists.
    pub plan_artifacts: bool,
}

impl Waivers {
    /// `--no-plan` lifts the plan blocker only for a feature that never wrote
    /// an SPDD artifact: the escape is "never written", never "written and
    /// ignored".
    pub(crate) fn waives_plan(self, plan_gate: GateState) -> bool {
        self.no_plan && !self.plan_artifacts && plan_gate == GateState::Pending
    }
}

/// Everything that refuses apply, in the order apply checks it. The preview
/// prints this list, and apply refuses exactly when it is non-empty.
pub(crate) fn blockers(preview: &DeliveryPreview, waivers: Waivers) -> Vec<String> {
    let descendants = preview.tree_blockers.iter().map(|blocker| {
        format!(
            "descendant `{}` is {}: {}",
            blocker.feature, blocker.state, blocker.reason
        )
    });
    let plan = (preview.plan_gate != GateState::Approved && !waivers.waives_plan(preview.plan_gate))
        .then(|| {
            if preview.plan_gate == GateState::Pending && !waivers.plan_artifacts {
                "the plan gate is pending and no plan was written: apply with `--no-plan \"<reason>\"` (human decision) or write and approve a plan".to_owned()
            } else {
                format!("the plan gate is {}, not approved", preview.plan_gate)
            }
        });
    let memory = (preview.memory_review == MemoryReviewGate::Missing
        && !waivers.skip_memory_review)
        .then(|| {
            format!(
                "memory review is missing: run `ivar memory review {0} --confirm`, or the memory_review MCP tool with `feature: \"{0}\"` and `confirm`",
                preview.feature
            )
        });
    let land = (preview.mode == DeliveryMode::Land && preview.repos.is_empty())
        .then(|| "no repositories are promoted to land".to_owned());
    let repos = preview.repos.iter().flat_map(|repo| {
        repo.blockers
            .iter()
            .map(move |blocker| format!("{}: {blocker}", repo.repo))
    });
    descendants
        .chain(plan)
        .chain(memory)
        .chain(land)
        .chain(repos)
        .collect()
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
/// the artifact a human crossed. See docs/architecture/seams.md, seam 7.
pub(crate) fn plan_not_approved(
    feature: &FeatureName,
    state: GateState,
    plan_written: bool,
    any_artifact: bool,
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
        let waiver = if any_artifact {
            ""
        } else {
            "; or, if the human decided this change needs no plan, apply with `--no-plan \"<reason>\"`"
        };
        FixAction::safe(
            "deliver.write_plan",
            format!(
                "Write a plan with `ivar plan create {feature} plan`, approve it with `ivar plan approve {feature} plan`, then preview and apply again{waiver}."
            ),
        )
        .command(format!("ivar plan create {feature} plan"))
    })
}

/// `--no-plan` given to a feature that wrote an SPDD artifact. The flag
/// waives a plan that was never written, not one that was written and ignored.
pub(crate) fn no_plan_with_artifacts(feature: &FeatureName, state: GateState) -> Failure {
    let failure = Failure::blocked(
        "deliver.no_plan_with_artifacts",
        format!("`--no-plan` cannot waive the plan of `{feature}`: it has plan artifacts"),
    )
    .expected("a feature with no `requirements.md`, `analysis.md` or `plan.md`")
    .actual(format!(
        "`{feature}` has plan artifacts and its plan gate is {state}"
    ));
    if state == GateState::Approved {
        failure.fix(FixAction::safe(
            "deliver.drop_no_plan",
            "The plan is already approved: apply again without `--no-plan`.",
        ))
    } else {
        failure.fix(
            FixAction::safe(
                "deliver.approve_plan",
                format!(
                    "Approve the plan with `ivar plan approve {feature} plan` (revise it first if it needs revision), then preview and apply again without `--no-plan`."
                ),
            )
            .command(format!("ivar plan approve {feature} plan")),
        )
    }
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
