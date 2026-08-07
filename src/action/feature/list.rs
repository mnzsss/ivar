//! `ivar feature list` — every feature in the hall and how far it got.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{Feature, WorktreeState};
use crate::domain::name::FeatureName;
use crate::error::{Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use crate::action::Ctx;

/// One feature's summary.
#[derive(Debug, Clone, Serialize)]
pub struct FeatureSummary {
    /// The feature's name.
    pub name: FeatureName,
    /// The branch every promoted repo's worktree is on.
    pub branch: String,
    /// How many repos are promoted into it.
    pub promoted_count: usize,
    /// How many of those promotions are fully `Ready`.
    pub ready_count: usize,
}

/// What `ivar feature list` found.
#[derive(Debug, Clone, Serialize)]
pub struct ListOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// One entry per feature, sorted by name.
    pub features: Vec<FeatureSummary>,
}

impl WriteHuman for ListOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.features.is_empty() {
            writeln!(w, "No features in {}.", self.root)?;
            return Ok(());
        }
        writeln!(w, "Features in {}:", self.root)?;
        for feature in &self.features {
            writeln!(
                w,
                "  {}  branch {}  promoted {}/{}",
                feature.name, feature.branch, feature.ready_count, feature.promoted_count,
            )?;
        }
        Ok(())
    }
}

/// List every feature with a `feature.json` under `.ivar/features/`.
///
/// A feature whose record cannot be parsed is skipped with the rest still
/// listed — this is a status command, and one corrupt record should not hide
/// the others. (The skipped name is not reported here; `doctor` is where
/// that lives.)
pub fn list(ctx: &Ctx) -> Outcome<ListOutcome> {
    let layout = discover_hall(ctx)?;

    let features_dir = layout.ivar_dir().join("features");
    let mut features = Vec::new();
    if fs::is_dir(&features_dir)? {
        for entry in fs::read_dir(&features_dir)? {
            let name = match entry.file_name() {
                Some(name) => name.to_owned(),
                None => continue,
            };
            let Some(feature_name) = FeatureName::new(name.as_str()).ok() else {
                continue;
            };
            if let Ok(Some(feature)) = Feature::read(&layout, &feature_name) {
                features.push(summary_of(&feature));
            }
        }
    }
    features.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Report::new(ListOutcome {
        root: layout.root().to_path_buf(),
        features,
    }))
}

fn summary_of(feature: &Feature) -> FeatureSummary {
    FeatureSummary {
        name: feature.name.clone(),
        branch: feature.branch.to_string(),
        promoted_count: feature.promotions.len(),
        ready_count: feature.count_worktrees(WorktreeState::Ready),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::action::feature::create::CreateInput;
    use crate::action::feature::create::create as create_action;
    use crate::action::hall::{self, InitInput};
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
    fn list_reports_an_empty_hall_as_empty() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let report = list(&ctx).unwrap();

        assert!(report.is_clean());
        assert!(report.value.features.is_empty());
    }

    #[test]
    fn list_reports_created_features_sorted_by_name() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        create_action(
            &ctx,
            CreateInput {
                name: "zeta".to_owned(),
            },
        )
        .unwrap();
        create_action(
            &ctx,
            CreateInput {
                name: "alpha".to_owned(),
            },
        )
        .unwrap();

        let report = list(&ctx).unwrap();

        let names: Vec<&str> = report
            .value
            .features
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(names, vec!["alpha", "zeta"]);
        assert_eq!(report.value.features[0].promoted_count, 0);
    }

    #[test]
    fn list_outside_a_hall_is_blocked() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root);

        let failure = list(&ctx).unwrap_err();

        assert_eq!(failure.code, "hall.not_found");
    }

    #[test]
    fn the_human_surface_lists_features_with_their_counts() {
        let outcome = ListOutcome {
            root: Utf8PathBuf::from("/hall"),
            features: vec![FeatureSummary {
                name: FeatureName::new("checkout").unwrap(),
                branch: "checkout".to_owned(),
                promoted_count: 2,
                ready_count: 1,
            }],
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Features in /hall:\n  checkout  branch checkout  promoted 1/2\n"
        );
    }
}
