//! `ivar plan create <feature> [artifacts...]` — scaffold a feature's SPDD
//! artifacts.
//!
//! Creates `plans/<feature>/` with `requirements.md`, `analysis.md`, and
//! `plan.md`, each carrying a short structural template. Named with no
//! artifacts, it scaffolds all three (today's behaviour, unchanged). Named
//! with a subset, it writes only what is missing from that subset and
//! silently leaves anything already present alone — the light-to-full
//! upgrade path, so a feature that started with `plan.md` only can grow
//! `requirements.md` and `analysis.md` later without hand-editing files. It
//! never overwrites an existing artifact — a plan a teammate already wrote to
//! is a plan in progress, not a file to regenerate.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use super::Artifact;
use crate::action::Ctx;

/// The plan scaffold records implementation intent and verification for a
/// provider-native coordinator; it does not encode a local execution graph.
const PLAN_TEMPLATE: &str = "\
# Plan

The REASONS canvas: explain the implementation, its constraints, and how it
will be verified.

## Entities

Domain model, delta only.

## Approach

The chosen design, and what was rejected.

## Structure

File and module organization.

## Changes

Describe the implementation in reviewable steps, including the files or
interfaces each step affects.

## Verification

List the checks that demonstrate the change is complete.

## Norms

Conventions this feature follows.

## Safeguards

What to watch out for.
";

/// The template text scaffolded for a fresh artifact. One mapping from
/// `Artifact` to its content — `Artifact::filename` is the other half, so
/// there is exactly one place that knows which file backs which artifact.
const fn template_for(artifact: Artifact) -> &'static str {
    match artifact {
        Artifact::Requirements => {
            "# Requirements\n\nWhat this feature must do. One sentence per requirement.\n\n- [ ] \n"
        }
        Artifact::Analysis => {
            "# Analysis\n\nHow the requirements will be met, and what was considered and rejected.\n\n"
        }
        Artifact::Plan => PLAN_TEMPLATE,
    }
}

/// What `ivar plan create` needs.
#[derive(Debug, Clone)]
pub struct CreateInput {
    /// The feature to scaffold plans for.
    pub feature: String,
    /// Which artifacts to scaffold. Empty means "all three" — the default,
    /// unchanged path. Non-empty is a chosen subset: write what's missing
    /// from it, skip what's already there.
    pub artifacts: Vec<Artifact>,
}

/// What `ivar plan create` did.
#[derive(Debug, Clone, Serialize)]
pub struct CreateOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the plans belong to.
    pub feature: FeatureName,
    /// The plan directory that now holds the artifacts.
    pub plan_dir: Utf8PathBuf,
    /// Artifacts written this run, in canonical order.
    pub created: Vec<Artifact>,
    /// Requested artifacts that were already present, left untouched, in
    /// canonical order. Always empty on the no-subset path — that path
    /// refuses instead of skipping (N-BACKWARD-COMPATIBLE).
    pub skipped: Vec<Artifact>,
}

impl WriteHuman for CreateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Created {} for `{}` in {}",
            artifact_list(&self.created),
            self.feature,
            self.plan_dir
        )?;
        if !self.skipped.is_empty() {
            writeln!(
                w,
                "Already present, left untouched: {}",
                artifact_list(&self.skipped)
            )?;
        }
        Ok(())
    }
}

/// Render artifacts as their filenames, comma-separated, for the human
/// surface.
fn artifact_list(artifacts: &[Artifact]) -> String {
    artifacts
        .iter()
        .map(|a| a.filename())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Scaffold the SPDD artifacts for `input.feature`.
///
/// Blocked when the feature does not exist (plans belong to features).
///
/// With no artifacts named, blocked when the plan directory already has any
/// artifact in it — today's behaviour, exactly (N-BACKWARD-COMPATIBLE). With
/// artifacts named, blocked only when every one of them is already present;
/// otherwise the missing ones are written and the present ones are reported
/// as skipped.
pub fn create(ctx: &Ctx, input: CreateInput) -> Outcome<CreateOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;

    // Plans belong to features; a plan for a feature nobody created is a
    // promise that will never be kept.
    let feature_dir = layout.feature_dir(&feature);
    if !fs::is_dir(&feature_dir)? {
        return Err(Failure::blocked(
            "plan.feature_not_found",
            format!("feature `{feature}` does not exist"),
        )
        .expected("an existing feature to plan")
        .actual(format!("`{feature}` has no feature directory"))
        .fix(FixAction::safe(
            "feature.create_first",
            format!("Create the feature first with `ivar feature create {feature}`."),
        )));
    }

    // Planning may continue during a partial integration, but not after the
    // whole child closes as `integrated`.
    let feature_record =
        crate::domain::feature::Feature::read(&layout, &feature)?.ok_or_else(|| {
            Failure::blocked(
                "plan.feature_vanished",
                format!("feature `{feature}` has a directory but no feature.json"),
            )
        })?;
    crate::action::feature::ensure_not_fully_integrated(&layout, &feature_record)?;

    let plan_dir = layout.plan_dir(&feature);

    // No subset named: today's set, in canonical order. Named: `Artifact::ALL`
    // filtered down to what was asked for — this both orders and deduplicates
    // the request for free.
    let no_subset = input.artifacts.is_empty();
    let wanted: Vec<Artifact> = if no_subset {
        Artifact::ALL.to_vec()
    } else {
        Artifact::ALL
            .into_iter()
            .filter(|a| input.artifacts.contains(a))
            .collect()
    };

    let (present, missing) = partition_by_existence(&plan_dir, &wanted)?;

    // No-subset refuses on ANY artifact already existing — byte-for-byte the
    // old rule. A named subset refuses only when EVERY requested artifact is
    // already there; anything less than that is the incremental-upgrade path
    // (Q-INCREMENTAL-CREATE), and the artifacts already present are skipped,
    // not regenerated.
    let refuse = if no_subset {
        !present.is_empty()
    } else {
        missing.is_empty()
    };
    if refuse {
        return Err(Failure::blocked(
            "plan.already_exists",
            format!("`{plan_dir}` already has SPDD artifacts"),
        )
        .expected("a feature with no plan artifacts yet")
        .actual("one or more of requirements.md / analysis.md / plan.md already exist")
        .fix(FixAction::safe(
            "plan.use_existing",
            "Work with the existing artifacts, or remove them deliberately first.",
        )));
    }

    fs::ensure_dir(&plan_dir)?;
    for artifact in &missing {
        fs::write_text(&plan_dir.join(artifact.filename()), template_for(*artifact))?;
    }

    Ok(Report::new(CreateOutcome {
        root: layout.root().to_path_buf(),
        feature,
        plan_dir,
        created: missing,
        skipped: present,
    }))
}

/// Split `wanted` into what already exists on disk and what doesn't, both in
/// the order `wanted` was given (which callers pass in canonical order).
fn partition_by_existence(
    plan_dir: &Utf8Path,
    wanted: &[Artifact],
) -> Result<(Vec<Artifact>, Vec<Artifact>), Failure> {
    let mut present = Vec::new();
    let mut missing = Vec::new();
    for artifact in wanted {
        if fs::is_file(&plan_dir.join(artifact.filename()))? {
            present.push(*artifact);
        } else {
            missing.push(*artifact);
        }
    }
    Ok((present, missing))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/plan/create.rs"]
mod tests;
