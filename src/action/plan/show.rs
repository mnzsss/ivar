//! `ivar plan show <feature>` — print one feature's SPDD artifact.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use crate::action::Ctx;

/// Which artifact to show.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Artifact {
    Requirements,
    Analysis,
    Plan,
}

impl Artifact {
    /// The artifact's filename.
    const fn filename(self) -> &'static str {
        match self {
            Self::Requirements => "requirements.md",
            Self::Analysis => "analysis.md",
            Self::Plan => "plan.md",
        }
    }
}

/// What `ivar plan show` needs.
#[derive(Debug, Clone)]
pub struct ShowInput {
    /// The feature whose artifact to show.
    pub feature: String,
    /// Which artifact.
    pub artifact: Artifact,
}

/// What `ivar plan show` did.
#[derive(Debug, Clone, Serialize)]
pub struct ShowOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature.
    pub feature: FeatureName,
    /// Which artifact was shown.
    pub artifact: Artifact,
    /// The artifact's content.
    pub content: String,
}

impl WriteHuman for ShowOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        write!(w, "{}", self.content)
    }
}

/// Print `input.artifact` for `input.feature`.
///
/// Blocked when the artifact does not exist — a plan not yet scaffolded is
/// not an empty plan, it is a missing one.
pub fn show(ctx: &Ctx, input: ShowInput) -> Outcome<ShowOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;

    let path = layout.plan_dir(&feature).join(input.artifact.filename());
    let content = fs::read_text(&path)?.ok_or_else(|| {
        Failure::blocked(
            "plan.artifact_missing",
            format!("`{}` does not exist", path),
        )
        .expected("the feature's SPDD artifact to have been scaffolded")
        .actual(format!("`{}` is absent", input.artifact.filename()))
        .fix(FixAction::safe(
            "plan.create_first",
            format!("Scaffold the feature's plans with `ivar plan create {feature}`."),
        ))
    })?;

    Ok(Report::new(ShowOutcome {
        root: layout.root().to_path_buf(),
        feature,
        artifact: input.artifact,
        content,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::action::feature::create::{
        self as feature_create, CreateInput as FeatureCreateInput,
    };
    use crate::action::hall::{self, InitInput};
    use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
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
        feature_create::create(
            &ctx,
            FeatureCreateInput {
                name: "checkout".to_owned(),
            },
        )
        .unwrap();
        plan_create::create(
            &ctx,
            PlanCreateInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();
        (guard, root)
    }

    #[test]
    fn show_prints_the_scaffolded_artifact() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let report = show(
            &ctx,
            ShowInput {
                feature: "checkout".to_owned(),
                artifact: Artifact::Requirements,
            },
        )
        .unwrap();

        assert!(report.value.content.contains("# Requirements"));
    }

    #[test]
    fn show_is_rejected_for_a_missing_artifact() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        // Delete the artifact behind ivar's back.
        fs::remove_path(&root.join("plans/checkout/analysis.md")).unwrap();

        let failure = show(
            &ctx,
            ShowInput {
                feature: "checkout".to_owned(),
                artifact: Artifact::Analysis,
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "plan.artifact_missing");
    }

    #[test]
    fn artifact_filenames_match_the_layout_contract() {
        assert_eq!(Artifact::Requirements.filename(), "requirements.md");
        assert_eq!(Artifact::Analysis.filename(), "analysis.md");
        assert_eq!(Artifact::Plan.filename(), "plan.md");
    }
}
