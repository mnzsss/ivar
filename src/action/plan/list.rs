//! `ivar plan list` — which features have SPDD artifacts, and how complete.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::FeatureName;
use crate::error::{Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use crate::action::Ctx;

/// The three SPDD artifacts, in canonical order. Shared with `create`; kept
/// here too so `list` can report completeness against the same names.
const ARTIFACTS: [&str; 3] = ["requirements.md", "analysis.md", "plan.md"];

/// One feature's plan state.
#[derive(Debug, Clone, Serialize)]
pub struct PlanSummary {
    /// The feature.
    pub feature: FeatureName,
    /// Which of the three artifacts exist.
    pub artifacts: Vec<String>,
}

/// What `ivar plan list` found.
#[derive(Debug, Clone, Serialize)]
pub struct ListOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// One entry per feature with at least one artifact, by feature name.
    pub plans: Vec<PlanSummary>,
}

impl WriteHuman for ListOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.plans.is_empty() {
            writeln!(w, "No plans in {}.", self.root)?;
            return Ok(());
        }
        writeln!(w, "Plans in {}:", self.root)?;
        for plan in &self.plans {
            writeln!(
                w,
                "  {}  [{}]",
                plan.feature,
                plan.artifacts.join(", ")
            )?;
        }
        Ok(())
    }
}

/// List every feature whose `plans/<feature>/` holds at least one SPDD
/// artifact.
pub fn list(ctx: &Ctx) -> Outcome<ListOutcome> {
    let layout = discover_hall(ctx)?;

    let plans_dir = layout.root().join("plans");
    let mut plans = Vec::new();
    if fs::is_dir(&plans_dir)? {
        for entry in fs::read_dir(&plans_dir)? {
            let Some(name) = entry.file_name() else {
                continue;
            };
            let Ok(feature) = FeatureName::new(name) else {
                continue;
            };
            let plan_dir = plans_dir.join(name);
            let artifacts: Vec<String> = ARTIFACTS
                .iter()
                .filter(|artifact| fs::is_file(&plan_dir.join(artifact)).unwrap_or(false))
                .map(|artifact| (*artifact).to_owned())
                .collect();
            if artifacts.is_empty() {
                continue;
            }
            plans.push(PlanSummary { feature, artifacts });
        }
    }
    plans.sort_by(|a, b| a.feature.cmp(&b.feature));

    Ok(Report::new(ListOutcome {
        root: layout.root().to_path_buf(),
        plans,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
    use crate::action::hall::{self, InitInput};
    use crate::action::plan::create::{self as plan_create, CreateInput};
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
        (guard, root)
    }

    #[test]
    fn list_reports_no_plans_in_a_fresh_hall() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let report = list(&ctx).unwrap();

        assert!(report.is_clean());
        assert!(report.value.plans.is_empty());
    }

    #[test]
    fn list_reports_created_plans_with_their_artifacts() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        feature_create::create(&ctx, FeatureCreateInput { name: "checkout".to_owned() }).unwrap();
        plan_create::create(&ctx, CreateInput { feature: "checkout".to_owned() }).unwrap();

        let report = list(&ctx).unwrap();

        assert_eq!(report.value.plans.len(), 1);
        assert_eq!(report.value.plans[0].feature.as_str(), "checkout");
        assert_eq!(report.value.plans[0].artifacts.len(), 3);
    }

    #[test]
    fn the_human_surface_lists_artifacts_per_feature() {
        let outcome = ListOutcome {
            root: Utf8PathBuf::from("/hall"),
            plans: vec![PlanSummary {
                feature: FeatureName::new("checkout").unwrap(),
                artifacts: vec!["requirements.md".to_owned(), "plan.md".to_owned()],
            }],
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Plans in /hall:\n  checkout  [requirements.md, plan.md]\n"
        );
    }
}
