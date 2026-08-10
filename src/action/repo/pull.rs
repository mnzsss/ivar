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
#[path = "../../../tests/unit/action/repo/pull.rs"]
mod tests;
