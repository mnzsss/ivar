//! `ivar plan create <feature>` — scaffold a feature's SPDD artifacts.
//!
//! Creates `plans/<feature>/` with `requirements.md`, `analysis.md`, and
//! `plan.md`, each carrying a short structural template. It never overwrites
//! existing artifacts — a plan a teammate already wrote to is a plan in
//! progress, not a file to regenerate.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use crate::action::Ctx;

/// The SPDD artifacts this slice scaffolds, in the canonical order.
const ARTIFACTS: [(&str, &str); 3] = [
    (
        "requirements.md",
        "# Requirements\n\nWhat this feature must do. One sentence per requirement.\n\n- [ ] \n",
    ),
    (
        "analysis.md",
        "# Analysis\n\nHow the requirements will be met, and what was considered and rejected.\n\n",
    ),
    ("plan.md", PLAN_TEMPLATE),
];

/// The plan scaffold, longer than its two siblings because `## Operations` and
/// `## Operation details` are parsed, not read: they are what `tick` turns into
/// executor prompts (see [`crate::action::execute::plan_ops`]). Handing over
/// the headings the parser needs is cheaper than refusing the plan later.
const PLAN_TEMPLATE: &str = "\
# Plan

The REASONS canvas. Design sections are prose; `Operations` and `Operation
details` are parsed — see `/ivar-plan` for the rules.

## Entities

Domain model, delta only.

## Approach

The chosen design, and what was rejected.

## Structure

File and module organization.

## Operations

Each `###` heading is a workstream id from the execution graph, byte for byte —
never a phase or a cluster. Its bullets are operation ids and nothing else.

### <workstream-id>
- OP-<SLUG>
write_contract:
- path/it/may/write.rs

## Operation details

One entry per operation id, running until the next `**OP-*` marker or the next
heading. The text reaches the executor verbatim, bulleted metadata included.

**OP-<SLUG>** — What it changes.

- `dependsOn`: other operation ids, or nothing
- `touches`: the files it changes
- `tests`: what proves it
- `doneWhen`: the condition that closes it

## Norms

Conventions this feature follows.

## Safeguards

What to watch out for.
";

/// What `ivar plan create` needs.
#[derive(Debug, Clone)]
pub struct CreateInput {
    /// The feature to scaffold plans for.
    pub feature: String,
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
}

impl WriteHuman for CreateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Created SPDD artifacts for `{}` in {}",
            self.feature, self.plan_dir
        )
    }
}

/// Scaffold the SPDD artifacts for `input.feature`.
///
/// Blocked when the feature does not exist (plans belong to features), and
/// when the plan directory already exists with any artifact in it.
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
    if has_any_artifact(&plan_dir)? {
        return Err(Failure::blocked(
            "plan.already_exists",
            format!("`{}` already has SPDD artifacts", plan_dir),
        )
        .expected("a feature with no plan artifacts yet")
        .actual("one or more of requirements.md / analysis.md / plan.md already exist")
        .fix(FixAction::safe(
            "plan.use_existing",
            "Work with the existing artifacts, or remove them deliberately first.",
        )));
    }

    fs::ensure_dir(&plan_dir)?;
    for (name, template) in ARTIFACTS {
        fs::write_text(&plan_dir.join(name), template)?;
    }

    Ok(Report::new(CreateOutcome {
        root: layout.root().to_path_buf(),
        feature,
        plan_dir,
    }))
}

/// Whether any of the three artifacts already exist in `plan_dir`.
fn has_any_artifact(plan_dir: &camino::Utf8Path) -> Result<bool, Failure> {
    for (name, _) in ARTIFACTS {
        if fs::is_file(&plan_dir.join(name))? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/plan/create.rs"]
mod tests;
