//! `ivar feature deliver` — preview, then push + PR, a feature's promoted repos.

pub mod input;
pub mod land;
pub mod outcome;

mod preview;
mod push;
mod repos;

pub use input::{DeliverInput, PullRequestMetadata, RepoMetadataOverride};
pub use outcome::{DeliverOutcome, LandResult, PushResult, RepoCheckResult};

use crate::action::Ctx;
use crate::action::discover_hall;
use crate::action::feature::relations;
use crate::action::feature::verification;
use crate::action::read_manifest;
use crate::domain::feature::{
    DeliveryMode, DeliveryPreview, DeliveryTreeBlocker, Feature, FeatureIntegrationState, GateState,
};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report};
use crate::git;
use crate::store::layout::Layout;

use preview::{fingerprint_for, plan_gate_state, plan_not_approved, preview_required};
use repos::{build_repos, order_by_dependencies};

pub fn deliver(ctx: &Ctx, input: DeliverInput) -> Outcome<DeliverOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let feature_name = FeatureName::new(input.feature)?;
    let feature = read_feature(&layout, &feature_name)?;

    // Only a root delivers. A child's work belongs to its parent — the exact
    // fix names the verb that moves it there.
    if let Some(parent_name) = &feature.parent {
        return Err(Failure::blocked(
            "deliver.child_requires_integration",
            format!("feature `{feature_name}` is a child; it delivers into its parent, not to the remote"),
        )
        .expected("a root feature (one with no parent) to deliver")
        .actual(format!("`{feature_name}` is a subfeature of `{parent_name}`"))
        .fix(
            FixAction::safe(
                "deliver.integrate_child",
                format!("Integrate the child into its parent: `ivar feature integrate {feature_name}`."),
            )
            .command(format!("ivar feature integrate {feature_name}")),
        ));
    }

    // The tree is read as a whole: a corrupt lineage refuses loudly, and the
    // root's blocking descendants are derived from it.
    relations::read_all(&layout)?;
    let blockers = relations::blocking_descendants(&git, &layout, &manifest, &feature)?;
    let tree_blockers: Vec<DeliveryTreeBlocker> = blockers
        .iter()
        .map(|entry| DeliveryTreeBlocker {
            feature: entry.feature.clone(),
            depth: entry.depth,
            state: entry.state,
            reason: blocker_reason(entry.state),
        })
        .collect();

    let plan_gate = plan_gate_state(&layout, &feature_name)?;

    let mode = if input.land {
        DeliveryMode::Land
    } else {
        DeliveryMode::Push
    };

    let mut repos = build_repos(&git, &layout, &manifest, &feature, mode)?;
    repos.sort_by(|a, b| a.repo.cmp(&b.repo));
    order_by_dependencies(&mut repos);
    let fingerprint = fingerprint_for(&feature_name, mode, plan_gate, &tree_blockers, &repos)?;

    let preview = DeliveryPreview {
        feature: feature_name.clone(),
        mode,
        plan_gate,
        repos,
        tree_blockers,
        fingerprint,
    };

    if input.preview {
        return Ok(Report::new(DeliverOutcome {
            root: layout.root().to_path_buf(),
            preview,
            pushes: Vec::new(),
            land: Vec::new(),
            checks: Vec::new(),
        }));
    }

    // A root with blocking descendants cannot deliver: the tree must be
    // healthy below it first, leaves first. Refused before any push or PR.
    if !preview.tree_blockers.is_empty() {
        let names = preview
            .tree_blockers
            .iter()
            .map(|blocker| format!("{} ({})", blocker.feature, blocker.state))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(Failure::blocked(
            "deliver.descendants_block",
            format!(
                "cannot deliver `{feature_name}`: {} descendant(s) still block",
                preview.tree_blockers.len()
            ),
        )
        .expected("every descendant to be integrated, verified, or abandoned")
        .actual(names)
        .fix(FixAction::safe(
            "deliver.integrate_leaves_first",
            "Integrate the blocking descendants first, leaves first.",
        )));
    }

    // The approval gate comes before the drift gate. Both refuse, but only one
    // of them tells a human who never planned the feature what to do next, and
    // a `deliver` with no fingerprint at all is far more often that human than
    // one whose preview went stale.
    if plan_gate != GateState::Approved {
        return Err(plan_not_approved(&feature_name, plan_gate));
    }

    let expected = input
        .fingerprint
        .ok_or_else(|| preview_required(&feature_name))?;
    if expected != preview.fingerprint {
        return Err(Failure::blocked(
            "deliver.fingerprint_mismatch",
            format!(
                "the state of feature `{feature_name}` has drifted since the preview was approved"
            ),
        )
        .expected(format!("the preview fingerprint `{expected}`"))
        .actual(format!(
            "the current state fingerprints as `{}`",
            preview.fingerprint
        ))
        .fix(FixAction::safe(
            "deliver.re_preview",
            format!(
                "Run `ivar feature deliver {feature_name} --preview` again, then apply with the new fingerprint."
            ),
        )));
    }

    if input.land {
        let plans = land::preflight(&git, &layout, &feature, &preview)?;
        let mut warnings = Vec::new();

        // Run ordered checks for each root repo in land mode before executing merges.
        let mut checks = Vec::new();
        for repo in &preview.repos {
            let worktree = layout.repo_worktree(&repo.repo, &feature.branch);
            let repo_checks = verification::checks_for(&manifest, &repo.repo);
            let run = verification::run(&repo_checks, &worktree)?;
            let passed = run.results.iter().all(|result| result.success);
            checks.push(RepoCheckResult {
                repo: repo.repo.clone(),
                passed,
                results: run.results,
            });
            if !passed {
                return Err(Failure::blocked(
                    "deliver.checks_failed",
                    format!("verification checks failed for repo `{}`", repo.repo),
                )
                .expected("all verification checks to pass before landing")
                .actual(format!("verification checks failed in `{}`", repo.repo))
                .fix(FixAction::safe(
                    "deliver.fix_checks",
                    format!(
                        "Fix the failing verification checks in `{}` before landing.",
                        repo.repo
                    ),
                )));
            }
        }

        let land_results = land::execute(&git, &layout, &plans, &mut warnings)?;
        return Ok(Report::with_warnings(
            DeliverOutcome {
                root: layout.root().to_path_buf(),
                preview,
                pushes: Vec::new(),
                land: land_results,
                checks,
            },
            warnings,
        ));
    }

    push::execute(&git, &layout, &manifest, &feature_name, &feature, preview)
}

/// One sentence for why a descendant's state blocks delivery.
fn blocker_reason(state: FeatureIntegrationState) -> String {
    match state {
        FeatureIntegrationState::Active => "work is still in progress".to_owned(),
        FeatureIntegrationState::Failed => "carries failed integration evidence".to_owned(),
        FeatureIntegrationState::Stale => "its receipt no longer matches live state".to_owned(),
        // Unreachable: blocking descendants are only ever these three.
        other => format!("its state is `{other}`"),
    }
}

/// Read the feature, or a `Blocked` failure naming the way out.
fn read_feature(layout: &Layout, name: &FeatureName) -> Result<Feature, Failure> {
    Feature::read(layout, name)?.ok_or_else(|| {
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
    })
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/feature/deliver/mod.rs"]
mod tests;
