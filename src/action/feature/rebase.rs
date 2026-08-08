//! `ivar feature rebase <name>` — rebase every promoted repo's feature-branch
//! worktree onto its default branch.
//!
//! The point of a rebase here is to bring a feature's work up to date with the
//! work that landed on the default branches since the feature branched. Each
//! promoted repo's worktree (on the feature branch) is replayed on top of that
//! repo's `default_branch` from `ivar.json`.
//!
//! # Per-repo, never a batch abort
//!
//! A dirty worktree is skipped with a warning — rebasing over uncommitted work
//! is how it gets lost. A rebase that stops (a conflict, or any other git
//! refusal) is aborted with `git rebase --abort` and reported as conflicted,
//! and the next repo is tried. The report carries one status per repo:
//! `rebased`, `skipped`, or `conflicted`.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::Feature;
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git};
use crate::infra::fs;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// What `ivar feature rebase` needs.
#[derive(Debug, Clone)]
pub struct RebaseInput {
    /// The feature's name.
    pub name: String,
}

/// What happened to one promoted repo's worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RebaseStatus {
    /// The rebase completed — the worktree's branch now sits on the default
    /// branch's tip.
    Rebased,
    /// The repo was not rebased (dirty worktree, or no worktree to rebase).
    Skipped,
    /// The rebase stopped and was aborted; the worktree is untouched.
    Conflicted,
}

/// One promoted repo's rebase result.
#[derive(Debug, Clone, Serialize)]
pub struct RepoRebase {
    /// The repo.
    pub repo: RepoName,
    /// What happened to it.
    pub status: RebaseStatus,
}

/// What `ivar feature rebase` did.
#[derive(Debug, Clone, Serialize)]
pub struct RebaseOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature whose repos were rebased.
    pub feature: FeatureName,
    /// The feature branch every promoted worktree is on.
    pub branch: String,
    /// One entry per promoted repo, in name order.
    pub repos: Vec<RepoRebase>,
}

impl WriteHuman for RebaseOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Rebased feature `{}` (branch: {}) in {}:",
            self.feature, self.branch, self.root
        )?;
        if self.repos.is_empty() {
            writeln!(w, "  no repos promoted")?;
        }
        for repo in &self.repos {
            writeln!(
                w,
                "  {}  {}",
                repo.repo,
                match repo.status {
                    RebaseStatus::Rebased => "rebased",
                    RebaseStatus::Skipped => "skipped",
                    RebaseStatus::Conflicted => "conflicted",
                }
            )?;
        }
        Ok(())
    }
}

/// Rebase every promoted repo of `input.name` onto its default branch.
///
/// Blocked when the feature does not exist. Per-repo problems are warnings on
/// a clean report — skipped (dirty, or no worktree) and conflicted repos
/// continue the batch, exactly like `deliver`'s best-effort pushes.
pub fn rebase(ctx: &Ctx, input: RebaseInput) -> Outcome<RebaseOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let name = FeatureName::new(input.name)?;
    let feature = Feature::read(&layout, &name)?.ok_or_else(|| {
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
    })?;

    let mut repos = Vec::new();
    let mut warnings = Vec::new();
    for repo_name in feature.promotions.keys() {
        let worktree = layout.repo_worktree(repo_name, &feature.branch);
        let default_branch = manifest
            .repos()
            .iter()
            .find(|repo| repo.name() == repo_name)
            .map(|repo| repo.default_branch().to_string());

        let Some(default_branch) = default_branch else {
            repos.push(RepoRebase {
                repo: repo_name.clone(),
                status: RebaseStatus::Skipped,
            });
            warnings.push(Warning::new(
                "rebase.repo_not_in_manifest",
                repo_name.as_str(),
                "not in ivar.json; nothing to rebase onto",
            ));
            continue;
        };

        if !fs::is_dir(&worktree)? {
            repos.push(RepoRebase {
                repo: repo_name.clone(),
                status: RebaseStatus::Skipped,
            });
            warnings.push(Warning::new(
                "rebase.no_worktree",
                repo_name.as_str(),
                "no worktree materialised for this repo",
            ));
            continue;
        }

        // Rebase over uncommitted work is how it gets lost — a dirty worktree
        // is skipped, never rebased around.
        if git.worktree_dirty(&worktree)? {
            repos.push(RepoRebase {
                repo: repo_name.clone(),
                status: RebaseStatus::Skipped,
            });
            warnings.push(Warning::new(
                "rebase.dirty",
                repo_name.as_str(),
                "worktree has uncommitted changes; commit or stash them first",
            ));
            continue;
        }

        match git.rebase_branch(&worktree, &default_branch) {
            Ok(()) => repos.push(RepoRebase {
                repo: repo_name.clone(),
                status: RebaseStatus::Rebased,
            }),
            Err(git::Error::Refused { .. }) => {
                // The rebase stopped — a conflict, most likely. Abort it so
                // the worktree is exactly where it was, then move on.
                if let Err(abort) = git.abort_rebase(&worktree) {
                    warnings.push(Warning::new(
                        "rebase.abort_failed",
                        repo_name.as_str(),
                        format!("could not abort the stopped rebase: {abort}"),
                    ));
                }
                repos.push(RepoRebase {
                    repo: repo_name.clone(),
                    status: RebaseStatus::Conflicted,
                });
                warnings.push(Warning::new(
                    "rebase.conflicted",
                    repo_name.as_str(),
                    "rebase stopped (likely a conflict) and was aborted",
                ));
            }
            Err(other) => return Err(other.into()),
        }
    }
    repos.sort_by(|a, b| a.repo.cmp(&b.repo));

    Ok(Report::with_warnings(
        RebaseOutcome {
            root: layout.root().to_path_buf(),
            feature: name,
            branch: feature.branch.to_string(),
            repos,
        },
        warnings,
    ))
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
    use crate::domain::name::{BranchName, HallName};
    use crate::domain::provider::Provider;
    use crate::error::Status;
    use crate::store::layout::Layout;
    use crate::store::manifest::{Manifest, Providers, Repo};
    use crate::test_support::{git, hall_root, seeded_repo};

    /// A hall with one seeded repo declared, a feature created, and the repo
    /// promoted. Committer identity is set on the bare clone (shared by its
    /// worktrees) so `git rebase` — which runs through `git::System`, not the
    /// `-c`-flagged test helper — can create its commits on any machine.
    fn hall_with_promoted_feature() -> (tempfile::TempDir, Utf8PathBuf) {
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
            },
        )
        .unwrap();
        crate::action::sync::sync(&ctx, Default::default()).unwrap();
        promote::promote(
            &ctx,
            PromoteInput {
                feature: "checkout".to_owned(),
                repo: "api".to_owned(),
            },
        )
        .unwrap();

        git(
            &root.join(".ivar/repos/api/.bare"),
            &["config", "user.name", "ivar tests"],
        );
        git(
            &root.join(".ivar/repos/api/.bare"),
            &["config", "user.email", "tests@ivar.invalid"],
        );

        (guard, root)
    }

    fn rebase_input(name: &str) -> RebaseInput {
        RebaseInput {
            name: name.to_owned(),
        }
    }

    /// Commit directly in the default-branch worktree — which advances the
    /// shared `main` ref — so the feature branch has something to rebase onto.
    fn advance_main(root: &Utf8PathBuf) {
        let worktree = root.join(".ivar/repos/api/main");
        git(
            &worktree,
            &[
                "-c",
                "user.name=ivar tests",
                "-c",
                "user.email=tests@ivar.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "main work",
            ],
        );
    }

    #[test]
    fn rebase_replays_the_feature_work_onto_the_advanced_default_branch() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());

        // Feature work, committed on the feature branch.
        let feature_wt = root.join(".ivar/repos/api/checkout");
        std::fs::write(feature_wt.join("feat.txt"), "feature\n").unwrap();
        git(&feature_wt, &["add", "feat.txt"]);
        git(&feature_wt, &["commit", "-m", "feature work"]);
        // The default branch advances past the branch point.
        advance_main(&root);

        let report = rebase(&ctx, rebase_input("checkout")).unwrap();

        assert!(report.is_clean());
        assert_eq!(report.value.repos.len(), 1);
        assert_eq!(report.value.repos[0].status, RebaseStatus::Rebased);
        // The worktree now carries both the feature work and the main work.
        assert!(fs::is_file(&feature_wt.join("feat.txt")).unwrap());
        assert!(
            fs::is_file(&feature_wt.join("README.md")).unwrap(),
            "rebase must leave the base branch's files in place"
        );
    }

    #[test]
    fn rebase_skips_a_dirty_worktree_with_a_warning() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());
        let feature_wt = root.join(".ivar/repos/api/checkout");
        advance_main(&root);

        // Uncommitted work — untracked files count as dirty.
        std::fs::write(feature_wt.join("notes.md"), "mine\n").unwrap();

        let report = rebase(&ctx, rebase_input("checkout")).unwrap();

        assert_eq!(report.value.repos[0].status, RebaseStatus::Skipped);
        assert!(!report.is_clean());
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.code == "rebase.dirty")
        );
    }

    #[test]
    fn rebase_aborts_on_a_conflict_and_leaves_the_worktree_untouched() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());
        let feature_wt = root.join(".ivar/repos/api/checkout");
        let main_wt = root.join(".ivar/repos/api/main");

        // Both branches edit the same file, so the replay cannot apply cleanly.
        std::fs::write(feature_wt.join("README.md"), "feature\n").unwrap();
        git(&feature_wt, &["add", "README.md"]);
        git(&feature_wt, &["commit", "-m", "feature edit"]);
        std::fs::write(main_wt.join("README.md"), "main\n").unwrap();
        git(&main_wt, &["add", "README.md"]);
        git(&main_wt, &["commit", "-m", "main edit"]);

        let report = rebase(&ctx, rebase_input("checkout")).unwrap();

        assert_eq!(report.value.repos[0].status, RebaseStatus::Conflicted);
        assert!(!report.is_clean());
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.code == "rebase.conflicted")
        );
        // The abort restored the worktree: no rebase in progress, no unmerged
        // paths, and the branch's own committed content is back.
        let status = std::process::Command::new("git")
            .args(["-C", feature_wt.as_str(), "status", "--porcelain"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&status.stdout), "");
        assert_eq!(
            std::fs::read_to_string(feature_wt.join("README.md")).unwrap(),
            "feature\n"
        );
    }

    #[test]
    fn rebase_is_rejected_for_a_missing_feature() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root);

        let failure = rebase(&ctx, rebase_input("ghost")).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "feature.not_found");
    }

    #[test]
    fn the_human_surface_lists_per_repo_status() {
        let outcome = RebaseOutcome {
            root: Utf8PathBuf::from("/hall"),
            feature: FeatureName::new("checkout").unwrap(),
            branch: "checkout".to_owned(),
            repos: vec![RepoRebase {
                repo: RepoName::new("api").unwrap(),
                status: RebaseStatus::Rebased,
            }],
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Rebased feature `checkout` (branch: checkout) in /hall:\n  api  rebased\n"
        );
    }
}
