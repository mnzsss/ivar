//! `ivar feature demote <feature> <repo>` — remove a repo from a feature.
//!
//! Demoting removes the promotion record. The worktree stays on disk — like
//! `repo remove`, removing work can destroy uncommitted work, and that is a
//! decision `ivar cleanup` (slice 8) gets to make interactively, not a
//! config command on its own.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar feature demote` needs.
#[derive(Debug, Clone)]
pub struct DemoteInput {
    /// The feature's name.
    pub feature: String,
    /// The repo to demote.
    pub repo: String,
}

/// What `ivar feature demote` did.
#[derive(Debug, Clone, Serialize)]
pub struct DemoteOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the repo was demoted from.
    pub feature: FeatureName,
    /// The repo that was demoted.
    pub repo: RepoName,
}

impl WriteHuman for DemoteOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Demoted `{}` from feature `{}`. Its worktree stays on disk — `ivar cleanup` can remove it.",
            self.repo, self.feature,
        )
    }
}

/// Demote `input.repo` from `input.feature`.
///
/// Blocked when the feature does not exist or the repo was never promoted —
/// both name the way out, and neither leaves a half-edited record.
pub fn demote(ctx: &Ctx, input: DemoteInput) -> Outcome<DemoteOutcome> {
    let layout = discover_hall(ctx)?;
    let feature_name = FeatureName::new(input.feature)?;
    let repo_name = RepoName::new(input.repo)?;

    let mut feature =
        crate::domain::feature::Feature::read(&layout, &feature_name)?.ok_or_else(|| {
            Failure::blocked(
                "feature.not_found",
                format!("feature `{feature_name}` does not exist"),
            )
            .expected("an existing feature")
            .actual(format!("`{feature_name}` has no feature.json"))
            .fix(FixAction::safe(
                "feature.create_first",
                format!("Create it first with `ivar feature create {feature_name}`."),
            ))
        })?;

    if !feature.demote(&repo_name) {
        return Err(Failure::blocked(
            "feature.not_promoted",
            format!("`{repo_name}` is not promoted into `{feature_name}`"),
        )
        .expected("a repo currently promoted into this feature")
        .actual("this repo has no promotion record here")
        .fix(FixAction::safe(
            "feature.promote_first",
            format!("Run `ivar feature promote {feature_name} {repo_name}` first."),
        )));
    }

    feature.write(&layout)?;

    Ok(Report::new(DemoteOutcome {
        root: layout.root().to_path_buf(),
        feature: feature_name,
        repo: repo_name,
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
    use crate::action::feature::create::CreateInput;
    use crate::action::feature::create::create as create_action;
    use crate::action::feature::promote::{self, PromoteInput};
    use crate::action::hall::{self, InitInput};
    use crate::domain::feature::Feature;
    use crate::domain::name::{BranchName, HallName};
    use crate::domain::provider::Provider;
    use crate::error::Status;
    use crate::store::layout::Layout;
    use crate::store::manifest::{Manifest, Providers, Repo};
    use crate::test_support::{hall_root, seeded_repo};

    /// A hall with one seeded repo declared and a feature created.
    fn hall_with_feature() -> (tempfile::TempDir, Utf8PathBuf) {
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

        let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            vec![Repo::new(
                RepoName::new("api").unwrap(),
                origin.as_str(),
                BranchName::new("main").unwrap(),
            )],
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        create_action(
            &ctx,
            CreateInput {
                name: "checkout".to_owned(),
                branch: None,
            },
        )
        .unwrap();

        // Materialise the bare clone — promote operates on the cloned repo.
        crate::action::sync::sync(&ctx, Default::default()).unwrap();

        (guard, root)
    }

    #[test]
    fn demote_removes_the_promotion_record_and_keeps_the_worktree() {
        let (_guard, root) = hall_with_feature();
        let ctx = Ctx::new(root.clone());
        promote::promote(
            &ctx,
            PromoteInput {
                feature: "checkout".to_owned(),
                repo: "api".to_owned(),
            },
        )
        .unwrap();

        let report = demote(
            &ctx,
            DemoteInput {
                feature: "checkout".to_owned(),
                repo: "api".to_owned(),
            },
        )
        .unwrap();

        assert!(report.is_clean());
        let feature = Feature::read(
            &Layout::at(root.clone()),
            &FeatureName::new("checkout").unwrap(),
        )
        .unwrap()
        .unwrap();
        assert!(!feature.is_promoted(&RepoName::new("api").unwrap()));
        // The worktree stays.
        assert!(root.join(".ivar/repos/api/checkout/README.md").is_file());
    }

    #[test]
    fn demote_is_rejected_when_the_feature_does_not_exist() {
        let (_guard, root) = hall_with_feature();
        let ctx = Ctx::new(root);

        let failure = demote(
            &ctx,
            DemoteInput {
                feature: "ghost".to_owned(),
                repo: "api".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "feature.not_found");
    }

    #[test]
    fn demote_is_rejected_when_the_repo_was_never_promoted() {
        let (_guard, root) = hall_with_feature();
        let ctx = Ctx::new(root);

        let failure = demote(
            &ctx,
            DemoteInput {
                feature: "checkout".to_owned(),
                repo: "api".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "feature.not_promoted");
    }

    #[test]
    fn the_human_surface_says_the_worktree_stays() {
        let outcome = DemoteOutcome {
            root: Utf8PathBuf::from("/hall"),
            feature: FeatureName::new("checkout").unwrap(),
            repo: RepoName::new("api").unwrap(),
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Demoted `api` from feature `checkout`. Its worktree stays on disk — `ivar cleanup` can remove it.\n"
        );
    }
}
