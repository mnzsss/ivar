//! `ivar repo pull` — fetch one or all repos from their remotes.
//!
//! Fetch only, never merge and never reset: the worktrees under `.ivar/` are
//! where agents work, and "pull" changing what a working tree points at is
//! the kind of surprise a tool that runs in shared hall directories must not
//! cause. `git fetch --prune` updates the bare clone's refs; worktrees catch
//! up through rebase/merge the way their users already do.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git};
use crate::store::manifest::{Manifest, Repo};

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// What `ivar repo pull` needs.
#[derive(Debug, Clone, Default)]
pub struct PullInput {
    /// The repo to fetch. `None` fetches every repo in the manifest.
    pub repo: Option<String>,
}

/// What `ivar repo pull` did.
#[derive(Debug, Clone, Serialize)]
pub struct PullOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// Every repo that fetched successfully, in manifest order.
    pub repos: Vec<RepoName>,
    /// Every repo whose fetch failed, by name — each one also becomes a
    /// [`Warning`](crate::error::Warning), so the process exits 1.
    pub failed: Vec<String>,
}

impl WriteHuman for PullOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Fetched in {}:", self.root)?;
        for name in &self.repos {
            writeln!(w, "  {name}  fetched")?;
        }
        for name in &self.failed {
            writeln!(w, "  {name}  FAILED")?;
        }
        if self.repos.is_empty() && self.failed.is_empty() {
            writeln!(w, "  (no repos declared)")?;
        }
        Ok(())
    }
}

/// Fetch one repo — or all, when `input.repo` is `None`.
///
/// A repo whose fetch fails becomes an entry in `failed` and a
/// [`Warning`], and the others still fetch — the warning discipline from
/// ARCHITECTURE.md, applied the way `sync` applies it.
pub fn pull(ctx: &Ctx, input: PullInput) -> Outcome<PullOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let targets = resolve_targets(&manifest, input.repo.as_deref())?;

    let mut repos = Vec::new();
    let mut failed = Vec::new();
    let mut warnings = Vec::new();

    for repo in targets {
        let bare = layout.repo_bare(repo.name());
        match git.fetch(&bare) {
            // With `--quiet` a no-op fetch is indistinguishable from one
            // that pulled commits — what the report can say truthfully is
            // that the fetch ran.
            Ok(()) => repos.push(repo.name().clone()),
            Err(error) => {
                failed.push(repo.name().to_string());
                warnings.push(Warning::new(
                    "repo.fetch_failed",
                    repo.name().to_string(),
                    error.to_string(),
                ));
            }
        }
    }

    Ok(Report::with_warnings(
        PullOutcome {
            root: layout.root().to_path_buf(),
            repos,
            failed,
        },
        warnings,
    ))
}

/// The repos to fetch: the one named, or every repo in the manifest.
///
/// A named repo that is not in the manifest is blocked with a fix action —
/// a typo should not silently fetch nothing.
fn resolve_targets<'a>(manifest: &'a Manifest, named: Option<&str>) -> Result<Vec<&'a Repo>, Failure> {
    match named {
        None => Ok(manifest.repos().iter().collect()),
        Some(raw) => {
            let name = RepoName::new(raw)?;
            manifest
                .repos()
                .iter()
                .find(|repo| repo.name() == &name)
                .map(|repo| vec![repo])
                .ok_or_else(|| {
                    Failure::blocked(
                        "repo.not_found",
                        format!("`{name}` is not in ivar.json"),
                    )
                    .expected(format!("a repo name declared in `ivar.json`"))
                    .actual(format!("`{name}` does not appear in `repos`"))
                    .fix(FixAction::safe(
                        "repo.check_name",
                        "Check the repo name spelling, or run `ivar repo list`.",
                    ))
                })
        }
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
    use crate::error::Status;
    use crate::store::layout::Layout;
    use crate::store::manifest::Providers;
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

        (guard, root)
    }

    #[test]
    fn pull_fetches_every_declared_repo() {
        let (_guard, root) = hall_with(&[("api", "main"), ("web", "main")]);
        let ctx = Ctx::new(root.clone());
        crate::action::sync::sync(&ctx, Default::default()).unwrap();

        let report = pull(&ctx, PullInput::default()).unwrap();

        assert!(report.is_clean());
        assert_eq!(report.value.repos.len(), 2);
        assert!(report.value.failed.is_empty());
    }

    #[test]
    fn pull_with_no_repos_reports_an_empty_run() {
        let (_guard, root) = hall_with(&[]);
        let ctx = Ctx::new(root);

        let report = pull(&ctx, PullInput::default()).unwrap();

        assert!(report.is_clean());
        assert!(report.value.repos.is_empty());
    }

    #[test]
    fn pull_accepts_a_named_repo() {
        let (_guard, root) = hall_with(&[("api", "main"), ("web", "main")]);
        let ctx = Ctx::new(root.clone());
        crate::action::sync::sync(&ctx, Default::default()).unwrap();

        let report = pull(&ctx, PullInput { repo: Some("api".to_owned()) }).unwrap();

        assert_eq!(report.value.repos.len(), 1);
        assert_eq!(report.value.repos[0].as_str(), "api");
    }

    #[test]
    fn pull_blocks_on_a_repo_that_is_not_in_the_manifest() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        let ctx = Ctx::new(root);

        let failure = pull(&ctx, PullInput { repo: Some("ghost".to_owned()) }).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "repo.not_found");
    }

    #[test]
    fn a_fetch_failure_becomes_a_warning_and_the_run_continues() {
        let (_guard, root) = hall_with(&[("api", "main"), ("ghost", "main")]);
        let ctx = Ctx::new(root.clone());
        // Declare a repo pointing at a non-existent origin; `sync` never ran,
        // so its bare clone does not exist and fetch has nothing to talk to.
        let layout = Layout::at(root.clone());
        let manifest = Manifest::read(&layout).unwrap().unwrap();
        let mut repos = manifest.repos().to_vec();
        repos.push(Repo::new(
            RepoName::new("gone").unwrap(),
            root.join("no-such-origin").as_str(),
            BranchName::new("main").unwrap(),
        ));
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            repos,
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        let report = pull(&ctx, PullInput::default()).unwrap();

        assert!(!report.is_clean());
        assert!(
            report.warnings.iter().any(|w| w.subject == "gone"),
            "the failing repo must surface as a warning"
        );
        assert!(report.value.failed.contains(&"gone".to_owned()));
    }

    #[test]
    fn pull_outside_a_hall_is_blocked() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root);

        let failure = pull(&ctx, PullInput::default()).unwrap_err();

        assert_eq!(failure.code, "hall.not_found");
    }

    #[test]
    fn the_human_surface_reports_each_repo_and_any_failures() {
        let outcome = PullOutcome {
            root: Utf8PathBuf::from("/hall"),
            repos: vec![RepoName::new("api").unwrap()],
            failed: vec!["web".to_owned()],
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Fetched in /hall:\n  api  fetched\n  web  FAILED\n"
        );
    }
}
