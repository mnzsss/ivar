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
    (
        "plan.md",
        "# Plan\n\nStep-by-step implementation plan. Bite-sized tasks, in order.\n\n",
    ),
];

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
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
    use crate::action::hall::{self, InitInput};
    use crate::error::Status;
    use crate::test_support::hall_root;

    fn seeded_hall() -> (tempfile::TempDir, Utf8PathBuf) {
        let (guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());
        hall::init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("."),
                name: Some("acme".to_owned()),
                provider: None,
            },
        )
        .unwrap();
        feature_create::create(&ctx, FeatureCreateInput { name: "checkout".to_owned() }).unwrap();
        (guard, root)
    }

    #[test]
    fn create_scaffolds_the_three_artifacts() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());

        let report = create(&ctx, CreateInput { feature: "checkout".to_owned() }).unwrap();

        assert!(report.is_clean());
        let plan_dir = root.join("plans/checkout");
        assert!(fs::is_file(&plan_dir.join("requirements.md")).unwrap());
        assert!(fs::is_file(&plan_dir.join("analysis.md")).unwrap());
        assert!(fs::is_file(&plan_dir.join("plan.md")).unwrap());
    }

    #[test]
    fn create_is_rejected_for_a_missing_feature() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let failure = create(&ctx, CreateInput { feature: "ghost".to_owned() }).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "plan.feature_not_found");
    }

    #[test]
    fn create_is_rejected_when_artifacts_already_exist() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        create(&ctx, CreateInput { feature: "checkout".to_owned() }).unwrap();

        let failure = create(&ctx, CreateInput { feature: "checkout".to_owned() }).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "plan.already_exists");
    }

    #[test]
    fn the_human_surface_names_the_plan_dir() {
        let outcome = CreateOutcome {
            root: Utf8PathBuf::from("/hall"),
            feature: FeatureName::new("checkout").unwrap(),
            plan_dir: Utf8PathBuf::from("/hall/plans/checkout"),
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Created SPDD artifacts for `checkout` in /hall/plans/checkout\n"
        );
    }
}
