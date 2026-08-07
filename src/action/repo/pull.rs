//! `ivar repo pull` — refresh every registered repo's default branch.
//!
//! The valhalla **Pull** semantics: fetch-and-fast-forward of the read-only
//! default-branch worktree, for every registered repo — including ones
//! promoted in a feature. The fast-forward only ever advances the default
//! worktree; a feature worktree's branch is never touched, and advancing
//! default leaves every feature's merge-base unchanged.
//!
//! Best-effort: a repo whose remote is unreachable (or whose default branch
//! cannot fast-forward) is reported and skipped, never aborting the batch.
//! The run ends with a refreshed/failed/skipped summary, and any failure
//! exits `1` (through the [`Warning`] channel) rather than `2`.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git, TargetState};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Repo};

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// What `ivar repo pull` needs.
#[derive(Debug, Clone, Default)]
pub struct PullInput {
    /// The repo to refresh. `None` refreshes every repo in the manifest.
    pub repo: Option<String>,
}

/// What happened to one repo's default branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PullStatus {
    /// Fetched and fast-forwarded.
    Refreshed,
    /// The fetch failed — the remote is unreachable, or the repo was never
    /// materialised.
    Failed { reason: String },
    /// The fetch worked but the default branch cannot fast-forward.
    Skipped { reason: String },
}

/// One repo's place in a pull run.
#[derive(Debug, Clone, Serialize)]
pub struct RepoPull {
    /// The repo's name, as declared in `ivar.json`.
    pub repo: RepoName,
    /// What happened to its default branch.
    #[serde(flatten)]
    pub status: PullStatus,
}

/// What `ivar repo pull` did.
#[derive(Debug, Clone, Serialize)]
pub struct PullOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// Every repo's status, in manifest order.
    pub repos: Vec<RepoPull>,
}

impl PullOutcome {
    /// How many repos ended in each [`PullStatus`] variant, as
    /// `(refreshed, failed, skipped)` — the counts the summary line prints.
    #[must_use]
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut refreshed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        for repo in &self.repos {
            match &repo.status {
                PullStatus::Refreshed => refreshed += 1,
                PullStatus::Failed { .. } => failed += 1,
                PullStatus::Skipped { .. } => skipped += 1,
            }
        }
        (refreshed, failed, skipped)
    }
}

impl WriteHuman for PullOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Pulled in {}:", self.root)?;
        if self.repos.is_empty() {
            writeln!(w, "  (no repos declared)")?;
        }
        for repo in &self.repos {
            match &repo.status {
                PullStatus::Refreshed => writeln!(w, "  {}  refreshed", repo.repo)?,
                PullStatus::Failed { reason } => writeln!(w, "  {}  FAILED — {reason}", repo.repo)?,
                PullStatus::Skipped { reason } => {
                    writeln!(w, "  {}  skipped — {reason}", repo.repo)?
                }
            }
        }
        let (refreshed, failed, skipped) = self.counts();
        writeln!(
            w,
            "refreshed: {refreshed}  failed: {failed}  skipped: {skipped}"
        )
    }
}

/// Refresh one repo — or all, when `input.repo` is `None`.
///
/// A repo whose fetch fails becomes `Failed` and a [`Warning`], one whose
/// default branch cannot fast-forward becomes `Skipped` and a [`Warning`],
/// and the rest still refresh — the best-effort discipline from
/// ARCHITECTURE.md, applied the way `sync` applies it.
pub fn pull(ctx: &Ctx, input: PullInput) -> Outcome<PullOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let targets = resolve_targets(&manifest, input.repo.as_deref())?;

    let mut repos = Vec::new();
    let mut warnings = Vec::new();

    for repo in targets {
        let status = refresh_default(&git, &layout, repo);
        if let PullStatus::Failed { reason } = &status {
            warnings.push(Warning::new(
                "repo.pull_failed",
                repo.name().to_string(),
                reason.clone(),
            ));
        } else if let PullStatus::Skipped { reason } = &status {
            warnings.push(Warning::new(
                "repo.pull_skipped",
                repo.name().to_string(),
                reason.clone(),
            ));
        }
        repos.push(RepoPull {
            repo: repo.name().clone(),
            status,
        });
    }

    Ok(Report::with_warnings(
        PullOutcome {
            root: layout.root().to_path_buf(),
            repos,
        },
        warnings,
    ))
}

/// Fetch-and-fast-forward one repo's default-branch worktree.
///
/// The fetch runs *inside* the worktree (see [`Git::fetch_branch`] for why),
/// then the fast-forward advances it. A repo whose default worktree was never
/// materialised is `Failed` — the hall is out of line with `ivar.json`, and
/// `ivar sync` is the way back in line.
///
/// A default worktree that a session has read-only-guarded is temporarily made
/// writable for the duration of the git mutation and re-guarded after: git
/// cannot create files in a write-bit-cleared directory, and a checkout that
/// fails mid-merge would leave the branch advanced but the files missing.
///
/// `pub(crate)` because Smart Fetch on session start is this same operation —
/// the plan's "use the existing pull logic" is literally this function.
pub(crate) fn refresh_default(git: &impl Git, layout: &Layout, repo: &Repo) -> PullStatus {
    let worktree = layout.repo_worktree(repo.name(), repo.default_branch());

    match git.target_state(&worktree) {
        Ok(TargetState::Repository) => {}
        Ok(_) => {
            return PullStatus::Failed {
                reason: "no default-branch worktree; run `ivar sync`".to_owned(),
            };
        }
        Err(error) => {
            return PullStatus::Failed {
                reason: error.to_string(),
            };
        }
    }

    // Lift the read-only guard (if any) for the git mutation below. The
    // restore is best-effort: the refresh result stands even if the chmod
    // fails, and the next session start/connect re-applies the guard.
    let lifted = match fs::unix_mode(&worktree) {
        Ok(Some(mode)) if mode & 0o222 == 0 => match fs::restore_write_bits(&worktree) {
            Ok(()) => true,
            Err(error) => {
                return PullStatus::Failed {
                    reason: format!("could not lift the read-only guard: {error}"),
                };
            }
        },
        Ok(_) => false,
        Err(error) => {
            return PullStatus::Failed {
                reason: error.to_string(),
            };
        }
    };

    let status = match git.fetch_branch(&worktree, repo.default_branch().as_str()) {
        Err(error) => PullStatus::Failed {
            reason: error.to_string(),
        },
        Ok(()) => match git.fast_forward(&worktree) {
            Ok(()) => PullStatus::Refreshed,
            Err(error) => PullStatus::Skipped {
                reason: format!("cannot fast-forward the default branch: {error}"),
            },
        },
    };

    if lifted {
        let _ = fs::clear_write_bits(&worktree);
    }
    status
}

/// The repos to refresh: the one named, or every repo in the manifest.
///
/// A named repo that is not in the manifest is blocked with a fix action —
/// a typo should not silently refresh nothing.
fn resolve_targets<'a>(
    manifest: &'a Manifest,
    named: Option<&str>,
) -> Result<Vec<&'a Repo>, Failure> {
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
                    Failure::blocked("repo.not_found", format!("`{name}` is not in ivar.json"))
                        .expected("a repo name declared in `ivar.json`")
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
    use crate::store::manifest::Providers;
    use crate::test_support::{git, hall_root, seeded_repo};
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

    /// The seeded origin path of `name`, from the manifest's declared url.
    fn origin_path(root: &camino::Utf8Path, name: &str) -> Utf8PathBuf {
        let manifest = Manifest::read(&Layout::at(root.to_path_buf()))
            .unwrap()
            .unwrap();
        let url = manifest
            .repos()
            .iter()
            .find(|repo| repo.name().as_str() == name)
            .unwrap()
            .url();
        Utf8PathBuf::from(url)
    }

    fn status_of<'a>(report: &'a Report<PullOutcome>, name: &str) -> &'a PullStatus {
        &report
            .value
            .repos
            .iter()
            .find(|repo| repo.repo.as_str() == name)
            .unwrap()
            .status
    }

    #[test]
    fn pull_refreshes_every_declared_repo_and_reports_refreshed() {
        let (_guard, root) = hall_with(&[("api", "main"), ("web", "main")]);
        let ctx = Ctx::new(root.clone());
        crate::action::sync::sync(&ctx, Default::default()).unwrap();

        let report = pull(&ctx, PullInput::default()).unwrap();

        assert!(report.is_clean());
        assert_eq!(report.value.repos.len(), 2);
        for repo in &report.value.repos {
            assert_eq!(repo.status, PullStatus::Refreshed);
        }
    }

    /// The fetch-and-fast-forward is real, not just a report: the default
    /// worktree's files catch up to the commit the origin gained after sync.
    #[test]
    fn pull_advances_the_default_worktree_to_the_origins_new_tip() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        let ctx = Ctx::new(root.clone());
        crate::action::sync::sync(&ctx, Default::default()).unwrap();

        let origin = origin_path(&root, "api");
        std::fs::write(origin.join("CHANGELOG.md"), "v1\n").unwrap();
        git(&origin, &["add", "CHANGELOG.md"]);
        git(&origin, &["commit", "-m", "v1"]);

        let report = pull(&ctx, PullInput::default()).unwrap();

        assert!(report.is_clean());
        assert_eq!(status_of(&report, "api"), &PullStatus::Refreshed);
        assert_eq!(
            std::fs::read_to_string(root.join(".ivar/repos/api/main/CHANGELOG.md")).unwrap(),
            "v1\n"
        );
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

        let report = pull(
            &ctx,
            PullInput {
                repo: Some("api".to_owned()),
            },
        )
        .unwrap();

        assert_eq!(report.value.repos.len(), 1);
        assert_eq!(report.value.repos[0].repo.as_str(), "api");
        assert_eq!(report.value.repos[0].status, PullStatus::Refreshed);
    }

    #[test]
    fn pull_blocks_on_a_repo_that_is_not_in_the_manifest() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        let ctx = Ctx::new(root);

        let failure = pull(
            &ctx,
            PullInput {
                repo: Some("ghost".to_owned()),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "repo.not_found");
    }

    /// A read-only-guarded default worktree (the state a live session leaves
    /// behind) must still refresh: the guard is lifted for the fetch-and-
    /// fast-forward and re-applied afterwards. Without the lift, git cannot
    /// create the new file and would leave the branch advanced but the file
    /// missing — with a zero exit code.
    #[test]
    fn pull_refreshes_a_read_only_guarded_worktree_and_reapplies_the_guard() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        let ctx = Ctx::new(root.clone());
        crate::action::sync::sync(&ctx, Default::default()).unwrap();

        let worktree = root.join(".ivar/repos/api/main");
        let origin = origin_path(&root, "api");
        std::fs::write(origin.join("CHANGELOG.md"), "v1\n").unwrap();
        git(&origin, &["add", "CHANGELOG.md"]);
        git(&origin, &["commit", "-m", "v1"]);

        // The guard a session's view-dir materialisation would have applied.
        fs::clear_write_bits(&worktree).unwrap();

        let report = pull(&ctx, PullInput::default()).unwrap();

        assert_eq!(status_of(&report, "api"), &PullStatus::Refreshed);
        assert_eq!(
            std::fs::read_to_string(worktree.join("CHANGELOG.md")).unwrap(),
            "v1\n",
            "the guarded worktree must catch up to the new commit"
        );
        assert_eq!(
            fs::unix_mode(&worktree).unwrap().unwrap() & 0o222,
            0,
            "the read-only guard must be re-applied after the refresh"
        );
        // Restore so TempDir can clean up.
        fs::restore_write_bits(&worktree).unwrap();
    }

    /// A repo whose default worktree was never materialised cannot be
    /// refreshed — it fails (with the way back in line named), and the
    /// healthy repos still refresh.
    #[test]
    fn a_repo_with_no_worktree_fails_and_the_others_still_refresh() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        let ctx = Ctx::new(root.clone());
        crate::action::sync::sync(&ctx, Default::default()).unwrap();
        // A second declared repo that was never synced.
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
            report
                .warnings
                .iter()
                .any(|warning| warning.subject == "gone"),
            "the failing repo must surface as a warning"
        );
        assert!(matches!(
            status_of(&report, "gone"),
            PullStatus::Failed { reason } if !reason.is_empty()
        ));
        assert_eq!(status_of(&report, "api"), &PullStatus::Refreshed);
    }

    /// The "skipped" case: a default branch that diverged locally cannot be
    /// fast-forwarded — it is reported and skipped, never failed and never
    /// clobbered.
    #[test]
    fn a_non_fast_forward_branch_is_skipped_not_failed() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        let ctx = Ctx::new(root.clone());
        crate::action::sync::sync(&ctx, Default::default()).unwrap();

        // The default worktree gains a local commit while the origin moves
        // elsewhere — the fetch succeeds, the fast-forward cannot.
        let worktree = root.join(".ivar/repos/api/main");
        git(&worktree, &["commit", "--allow-empty", "-m", "local drift"]);
        let origin = origin_path(&root, "api");
        std::fs::write(origin.join("CHANGELOG.md"), "v1\n").unwrap();
        git(&origin, &["add", "CHANGELOG.md"]);
        git(&origin, &["commit", "-m", "v1"]);

        let report = pull(&ctx, PullInput::default()).unwrap();

        assert!(!report.is_clean());
        assert!(matches!(
            status_of(&report, "api"),
            PullStatus::Skipped { reason } if !reason.is_empty()
        ));
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.subject == "api" && warning.code == "repo.pull_skipped"),
            "a skipped repo must surface as a warning"
        );
    }

    #[test]
    fn pull_outside_a_hall_is_blocked() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root);

        let failure = pull(&ctx, PullInput::default()).unwrap_err();

        assert_eq!(failure.code, "hall.not_found");
    }

    #[test]
    fn the_human_surface_reports_per_repo_status_and_the_counts() {
        let outcome = PullOutcome {
            root: Utf8PathBuf::from("/hall"),
            repos: vec![
                RepoPull {
                    repo: RepoName::new("api").unwrap(),
                    status: PullStatus::Refreshed,
                },
                RepoPull {
                    repo: RepoName::new("web").unwrap(),
                    status: PullStatus::Failed {
                        reason: "no worktree".to_owned(),
                    },
                },
                RepoPull {
                    repo: RepoName::new("app").unwrap(),
                    status: PullStatus::Skipped {
                        reason: "diverged".to_owned(),
                    },
                },
            ],
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Pulled in /hall:\n  api  refreshed\n  web  FAILED — no worktree\n  app  skipped — diverged\n\
             refreshed: 1  failed: 1  skipped: 1\n"
        );
    }

    #[test]
    fn the_json_surface_carries_the_status_per_repo() {
        let outcome = PullOutcome {
            root: Utf8PathBuf::from("/hall"),
            repos: vec![RepoPull {
                repo: RepoName::new("api").unwrap(),
                status: PullStatus::Refreshed,
            }],
        };

        let json = serde_json::to_string(&Report::new(outcome)).unwrap();

        assert_eq!(
            json,
            r#"{"root":"/hall","repos":[{"repo":"api","status":"refreshed"}]}"#
        );
    }
}
