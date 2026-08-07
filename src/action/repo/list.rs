//! `ivar repo list` — show every repo the hall knows about, and its state.
//!
//! Read-only. It looks at what `ivar.json` declares and what exists under
//! `.ivar/`, and never mutates either.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::error::{Outcome, Report, WriteHuman};
use crate::git::{self, TargetState};
use crate::store::layout::Layout;
use crate::store::manifest::Repo;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// One repo's observed state.
#[derive(Debug, Clone, Serialize)]
pub struct RepoStatus {
    /// The repo's name, as declared in `ivar.json`.
    pub name: RepoName,
    /// The git remote URL.
    pub url: String,
    /// The branch a fresh worktree defaults to.
    pub default_branch: String,
    /// Whether the bare clone exists under `.ivar/`.
    pub bare_cloned: bool,
    /// Whether the default-branch worktree exists.
    pub default_worktree: bool,
    /// Every branch the bare clone knows about, sorted.
    pub branches: Vec<String>,
}

/// What `ivar repo list` found.
#[derive(Debug, Clone, Serialize)]
pub struct ListOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// One entry per repo in `ivar.json`, in manifest order.
    pub repos: Vec<RepoStatus>,
}

impl WriteHuman for ListOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.repos.is_empty() {
            writeln!(w, "No repos in {}.", self.root)?;
            return Ok(());
        }
        writeln!(w, "Repos in {}:", self.root)?;
        for repo in &self.repos {
            let bare = if repo.bare_cloned { "cloned" } else { "missing" };
            let worktree = if repo.default_worktree {
                String::new()
            } else {
                " (no worktree)".to_owned()
            };
            let branches = if repo.branches.is_empty() {
                String::new()
            } else {
                format!("  [{}]", repo.branches.join(", "))
            };
            writeln!(
                w,
                "  {}  {bare}  {}{worktree}  ← {}{branches}",
                repo.name, repo.default_branch, repo.url,
            )?;
        }
        Ok(())
    }
}

/// List every repo declared in `ivar.json`, with its on-disk state.
///
/// A repo whose bare clone cannot be read (corrupt, or gone mid-listing)
/// reports `bare_cloned: false` and an empty branch list rather than failing
/// the whole listing — this is a status command, and one broken repo should
/// not hide the other seven.
pub fn list(ctx: &Ctx) -> Outcome<ListOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let repos = manifest
        .repos()
        .iter()
        .map(|repo| status_of(&git, &layout, repo))
        .collect();

    Ok(Report::new(ListOutcome {
        root: layout.root().to_path_buf(),
        repos,
    }))
}

/// Observe one repo's on-disk state without letting any single probe fail
/// the listing.
fn status_of(git: &impl git::Git, layout: &Layout, repo: &Repo) -> RepoStatus {
    let bare = layout.repo_bare(repo.name());
    let worktree = layout.repo_worktree(repo.name(), repo.default_branch());

    let bare_state = git.target_state(&bare).unwrap_or(TargetState::Absent);
    let worktree_state = git
        .target_state(&worktree)
        .unwrap_or(TargetState::Absent);

    let branches = if matches!(bare_state, TargetState::Repository) {
        git.list_branches(&bare).unwrap_or_default()
    } else {
        Vec::new()
    };

    RepoStatus {
        name: repo.name().clone(),
        url: repo.url().to_owned(),
        default_branch: repo.default_branch().to_string(),
        bare_cloned: matches!(bare_state, TargetState::Repository),
        default_worktree: matches!(worktree_state, TargetState::Repository),
        branches,
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
    use crate::action::hall::{self, InitInput};
    use crate::domain::name::{BranchName, HallName};
    use crate::domain::provider::Provider;
    use crate::store::manifest::{Manifest, Providers};
    use crate::test_support::{hall_root, seeded_repo};

    fn hall_with(repos: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
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

        if !repos.is_empty() {
            let origins = root.parent().unwrap().join("origins");
            let declared: Vec<Repo> = repos
                .iter()
                .map(|(name, branch)| {
                    let origin = seeded_repo(&origins.join(name), branch);
                    Repo::new(
                        RepoName::new(*name).unwrap(),
                        origin.as_str(),
                        BranchName::new(*branch).unwrap(),
                    )
                })
                .collect();

            let layout = Layout::at(root.clone());
            let manifest = Manifest::new(
                HallName::new("acme").unwrap(),
                Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
                declared,
                None,
            )
            .unwrap();
            Manifest::write(&layout, &manifest).unwrap();
        }

        (guard, root)
    }

    #[test]
    fn list_reports_an_empty_hall_as_empty() {
        let (_guard, root) = hall_with(&[]);
        let ctx = Ctx::new(root);

        let report = list(&ctx).unwrap();

        assert!(report.is_clean());
        assert!(report.value.repos.is_empty());
    }

    #[test]
    fn list_reports_a_declared_repo_before_any_sync() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        let ctx = Ctx::new(root);

        let report = list(&ctx).unwrap();

        let repo = &report.value.repos[0];
        assert_eq!(repo.name.as_str(), "api");
        assert_eq!(repo.default_branch, "main");
        assert!(!repo.bare_cloned, "not synced yet");
        assert!(repo.branches.is_empty());
    }

    #[test]
    fn list_reports_a_synced_repo_with_its_branches() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        let ctx = Ctx::new(root.clone());
        crate::action::sync::sync(&ctx, Default::default()).unwrap();

        let report = list(&ctx).unwrap();

        let repo = &report.value.repos[0];
        assert!(repo.bare_cloned);
        assert!(repo.default_worktree);
        assert!(repo.branches.contains(&"main".to_owned()));
    }

    #[test]
    fn list_outside_a_hall_is_blocked() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root);

        let failure = list(&ctx).unwrap_err();

        assert_eq!(failure.code, "hall.not_found");
    }

    #[test]
    fn the_human_surface_lists_repos_with_their_state() {
        let outcome = ListOutcome {
            root: Utf8PathBuf::from("/hall"),
            repos: vec![RepoStatus {
                name: RepoName::new("api").unwrap(),
                url: "git@example.com:acme/api.git".to_owned(),
                default_branch: "main".to_owned(),
                bare_cloned: true,
                default_worktree: true,
                branches: vec!["dev".to_owned(), "main".to_owned()],
            }],
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Repos in /hall:\n  api  cloned  main  ← git@example.com:acme/api.git  [dev, main]\n"
        );
    }
}
