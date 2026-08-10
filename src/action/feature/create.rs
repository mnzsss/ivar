//! `ivar feature create <name>` — start a feature.
//!
//! A feature is a branch name plus the (initially empty) set of repos
//! promoted onto it. Creating it records that in `features/<name>/` and
//! nothing else: no repo is touched until `promote` says so, and no worktree
//! appears until a repo is promoted onto the branch.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar feature create` needs.
#[derive(Debug, Clone)]
pub struct CreateInput {
    /// The feature's name, unvalidated — [`FeatureName`] is this module's job.
    pub name: String,
    /// The branch to use, unvalidated. `None` derives it from the name.
    ///
    /// The two differ when a branch that already exists cannot be spelled as a
    /// feature name: a [`FeatureName`] is one path segment, while `feat/login`
    /// is an ordinary branch. Without this, such a branch is unreachable —
    /// `promote` can adopt it, but no feature could ever name it.
    pub branch: Option<String>,
}

/// What `ivar feature create` did.
#[derive(Debug, Clone, Serialize)]
pub struct CreateOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature, as created.
    pub name: FeatureName,
    /// The branch every promoted repo's worktree will be checked out on.
    pub branch: BranchName,
}

impl WriteHuman for CreateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Created feature `{}` (branch: {}) in {}",
            self.name, self.branch, self.root
        )
    }
}

/// Create a feature named `input.name`, on `input.branch` or on a branch of
/// the same name.
///
/// A [`FeatureName`] is one path segment; a [`BranchName`] is not. So
/// `<name>` → branch `<name>` covers the ordinary case, and `--branch` covers
/// the one it cannot spell: adopting `feat/login`, which is a perfectly good
/// branch and an impossible feature name.
///
/// Refuses when a feature with that name already exists — a second `create`
/// would overwrite promotions that a teammate is already working against.
pub fn create(ctx: &Ctx, input: CreateInput) -> Outcome<CreateOutcome> {
    let layout = discover_hall(ctx)?;
    let name = FeatureName::new(input.name.clone())?;
    let branch = BranchName::new(input.branch.unwrap_or(input.name))?;

    let dir = layout.feature_dir(&name);
    if fs::is_dir(&dir)? {
        return Err(Failure::blocked(
            "feature.already_exists",
            format!("feature `{name}` already exists"),
        )
        .expected("a feature name that has not been used before")
        .actual(format!("`{}` already has a feature directory", dir))
        .fix(FixAction::safe(
            "feature.use_existing",
            "Use the existing feature, or pick a different name.",
        )));
    }

    let feature = Feature::new(name.clone(), branch.clone());
    feature.write(&layout)?;

    Ok(Report::new(CreateOutcome {
        root: layout.root().to_path_buf(),
        name,
        branch,
    }))
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
        (guard, root)
    }

    #[test]
    fn create_makes_the_feature_directory_and_records_the_feature() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());

        let report = create(
            &ctx,
            CreateInput {
                name: "checkout".to_owned(),
                branch: None,
            },
        )
        .unwrap();

        assert!(report.is_clean());
        assert_eq!(report.value.name.as_str(), "checkout");
        assert_eq!(report.value.branch.as_str(), "checkout");
        assert!(fs::is_file(&root.join(".ivar/features/checkout/feature.json")).unwrap());
    }

    #[test]
    fn create_rejects_a_feature_that_already_exists() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        create(
            &ctx,
            CreateInput {
                name: "checkout".to_owned(),
                branch: None,
            },
        )
        .unwrap();

        let error = create(
            &ctx,
            CreateInput {
                name: "checkout".to_owned(),
                branch: None,
            },
        )
        .unwrap_err();

        assert_eq!(error.status, Status::Blocked);
        assert_eq!(error.code, "feature.already_exists");
    }

    #[test]
    fn create_outside_a_hall_is_blocked() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root);

        let failure = create(
            &ctx,
            CreateInput {
                name: "checkout".to_owned(),
                branch: None,
            },
        )
        .unwrap_err();

        assert_eq!(failure.code, "hall.not_found");
    }

    #[test]
    fn create_rejects_an_invalid_name() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let failure = create(
            &ctx,
            CreateInput {
                name: "../etc".to_owned(),
                branch: None,
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "name.not_a_segment");
    }

    /// The whole point of the option: `feat/login` is a fine branch and an
    /// impossible feature name, so without this the branch is unreachable.
    #[test]
    fn an_explicit_branch_may_be_one_a_feature_name_could_not_spell() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root.clone());

        let report = create(
            &ctx,
            CreateInput {
                name: "login".to_owned(),
                branch: Some("feat/login".to_owned()),
            },
        )
        .unwrap();

        assert_eq!(report.value.name.as_str(), "login");
        assert_eq!(report.value.branch.as_str(), "feat/login");
        assert!(fs::is_file(&root.join(".ivar/features/login/feature.json")).unwrap());
    }

    /// The branch is still validated — `--branch` is not a hole in the rules.
    #[test]
    fn an_explicit_branch_is_still_validated() {
        let (_guard, root) = seeded_hall();
        let ctx = Ctx::new(root);

        let failure = create(
            &ctx,
            CreateInput {
                name: "login".to_owned(),
                branch: Some("../etc".to_owned()),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
    }

    #[test]
    fn the_human_surface_names_the_feature_branch_and_root() {
        let outcome = CreateOutcome {
            root: Utf8PathBuf::from("/hall"),
            name: FeatureName::new("checkout").unwrap(),
            branch: BranchName::new("checkout").unwrap(),
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Created feature `checkout` (branch: checkout) in /hall\n"
        );
    }
}
