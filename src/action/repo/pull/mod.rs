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
//!
//! A skipped default branch — one that diverged and cannot fast-forward — is
//! `ivar` refusing to guess. `--diagnose` turns that refusal into a report
//! (read-only): the local-only and remote-only commits, so a human can tell a
//! branch that merely fell behind from one that genuinely diverged, and spot
//! local commits already re-landed upstream. The branch is never moved; the
//! reconciliation is left to the human, because only they know whether the
//! local commits are theirs.
//!
//! `--resolve` automates the one reconciliation that is provably safe: when
//! every local-only commit is a duplicate of work already in the remote
//! (same patch-id — re-landed as a squash, a rebase, or a cherry-pick), the
//! branch is reset to the remote tip and reported `Resolved`. Nothing local is
//! lost, because the content is upstream. It never touches a branch with
//! genuine local work or a dirty worktree — those are reported (with the
//! `--diagnose` detail, since resolve implies it) and left for the human.
//! Resolve implies `--diagnose`; `--diagnose` alone never moves a branch.
//!
//! Because every repo costs a network round trip, the run reports which one it
//! is on through [`Ctx::progress`] — one transient stderr line, erased before
//! the summary is written, absent under `--json` and off a terminal. The
//! granularity is per repo, not per phase inside [`refresh_default`]: the
//! fetch is the part that waits on a remote, and the two local steps around it
//! would flash past unread.

mod diagnosis;

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Divergence, Git, TargetState};
use crate::infra::fs;
use crate::infra::progress::Progress;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Repo};

use crate::action::Ctx;
use crate::action::{discover_hall, read_manifest};

use self::diagnosis::{diagnose_divergence, remote_ref, safe_to_reset};

/// What `ivar repo pull` needs.
#[derive(Debug, Clone, Default)]
pub struct PullInput {
    /// The repo to refresh. `None` refreshes every repo in the manifest.
    pub repo: Option<String>,
    /// When a repo cannot fast-forward, report the divergence in detail
    /// (the local and remote commits each side has) instead of only the
    /// "skipped" line. Read-only — it never moves a ref or a branch.
    pub diagnose: bool,
    /// Automatically reconcile a diverged default branch when it is safe to:
    /// reset it to the remote tip when every local commit is a duplicate of
    /// work that already landed upstream (same patch-id). Never touches a
    /// branch with genuine local work — that is reported and left to the
    /// human. Implies [`Self::diagnose`] for the repos it cannot resolve.
    pub resolve: bool,
}

/// What happened to one repo's default branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PullStatus {
    /// Fetched and fast-forwarded.
    Refreshed,
    /// Fetched and found to have diverged, but every local commit was a
    /// duplicate of work already upstream, so the default branch was reset to
    /// the remote tip. No local work was lost.
    Resolved,
    /// The fetch failed — the remote is unreachable, or the repo was never
    /// materialised.
    Failed { reason: String },
    /// The fetch worked but the default branch cannot fast-forward.
    Skipped {
        reason: String,
        /// The local-vs-remote divergence, when the run asked for a
        /// diagnosis (`--diagnose`). Absent otherwise.
        #[serde(skip_serializing_if = "Option::is_none")]
        divergence: Option<Divergence>,
    },
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
    /// `(refreshed, resolved, failed, skipped)` — the counts the summary line
    /// prints.
    #[must_use]
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let mut refreshed = 0;
        let mut resolved = 0;
        let mut failed = 0;
        let mut skipped = 0;
        for repo in &self.repos {
            match &repo.status {
                PullStatus::Refreshed => refreshed += 1,
                PullStatus::Resolved => resolved += 1,
                PullStatus::Failed { .. } => failed += 1,
                PullStatus::Skipped { .. } => skipped += 1,
            }
        }
        (refreshed, resolved, failed, skipped)
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
                PullStatus::Resolved => writeln!(
                    w,
                    "  {}  resolved — local commits were duplicates already upstream; reset to the remote tip",
                    repo.repo
                )?,
                PullStatus::Failed { reason } => writeln!(w, "  {}  FAILED — {reason}", repo.repo)?,
                PullStatus::Skipped { reason, divergence } => {
                    writeln!(w, "  {}  skipped — {reason}", repo.repo)?;
                    if let Some(divergence) = divergence {
                        write_divergence(w, &repo.repo, divergence)?;
                    }
                }
            }
        }
        let (refreshed, resolved, failed, skipped) = self.counts();
        writeln!(
            w,
            "refreshed: {refreshed}  resolved: {resolved}  failed: {failed}  skipped: {skipped}"
        )
    }
}

/// The `--diagnose` detail under a skipped repo: the local-only and
/// remote-only commits, so a human can tell a branch that fell behind from
/// one that genuinely diverged — and spot local commits already re-landed
/// upstream.
fn write_divergence(
    w: &mut impl io::Write,
    repo: &RepoName,
    divergence: &Divergence,
) -> io::Result<()> {
    let Divergence {
        local_only,
        remote_only,
    } = divergence;

    if !local_only.is_empty() {
        writeln!(
            w,
            "      {repo} is {} commit(s) ahead — only here:",
            local_only.len()
        )?;
        for commit in local_only {
            writeln!(
                w,
                "        {sha:.8}  {subject}",
                sha = commit.sha,
                subject = commit.subject
            )?;
        }
    }
    if !remote_only.is_empty() {
        writeln!(
            w,
            "      {repo} is {} commit(s) behind — only upstream:",
            remote_only.len()
        )?;
        for commit in remote_only {
            writeln!(
                w,
                "        {sha:.8}  {subject}",
                sha = commit.sha,
                subject = commit.subject
            )?;
        }
    }
    if local_only.is_empty() && remote_only.is_empty() {
        writeln!(w, "      {repo} is neither ahead nor behind the remote")?;
    }
    Ok(())
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

    let total = targets.len();
    for (index, repo) in targets.iter().enumerate() {
        ctx.progress().step(&fetch_step(index, total, repo));
        let status = refresh_default(&git, &layout, repo, input.diagnose, input.resolve);
        if let PullStatus::Failed { reason } = &status {
            warnings.push(Warning::new(
                "repo.pull_failed",
                repo.name().to_string(),
                reason.clone(),
            ));
        } else if let PullStatus::Skipped { reason, .. } = &status {
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
    ctx.progress().clear();

    Ok(Report::with_warnings(
        PullOutcome {
            root: layout.root().to_path_buf(),
            repos,
        },
        warnings,
    ))
}

/// The transient line announcing repo `index` (0-based) of `total`.
///
/// Shared between [`pull`] and [`refresh_all`] — the latter is the Smart
/// Fetch sweep `session::start` and `execute::tick` both call: three verbs
/// waiting on the same round trip say so the same way, and a change to the
/// wording is one edit.
fn fetch_step(index: usize, total: usize, repo: &Repo) -> String {
    format!(
        "[{}/{total}] {}: fetching {}…",
        index + 1,
        repo.name(),
        repo.default_branch()
    )
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
/// `pub(crate)` because [`refresh_all`] — the Smart Fetch sweep — is a loop
/// around this same operation; the plan's "use the existing pull logic" is
/// literally this function.
pub(crate) fn refresh_default(
    git: &impl Git,
    layout: &Layout,
    repo: &Repo,
    diagnose: bool,
    resolve: bool,
) -> PullStatus {
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

    // Lift the read-only guard (if any) for the git mutation below. Held as a
    // value: the re-guard happens on drop, so no path out of the match below
    // can leave the worktree writable.
    let _guard = match fs::LiftedGuard::lift(&[&worktree]) {
        Ok(guard) => guard,
        Err(failure) => {
            return PullStatus::Failed {
                reason: failure.what,
            };
        }
    };

    match git.fetch_branch(&worktree, repo.default_branch().as_str()) {
        Err(error) => PullStatus::Failed {
            reason: error.to_string(),
        },
        Ok(()) => match git.fast_forward(&worktree) {
            Ok(()) => PullStatus::Refreshed,
            Err(error) => {
                let reason = format!("cannot fast-forward the default branch: {error}");
                let divergence = if diagnose || resolve {
                    diagnose_divergence(git, &worktree, repo)
                } else {
                    None
                };
                // Only `--resolve` gets the named blocker: it is the run that
                // tried to act and declined, so it is the run that owes an
                // explanation. A plain pull never offered to fix anything, and
                // its callers already assert git's own wording.
                match resolve.then(|| safe_to_reset(git, &worktree, repo)) {
                    None => PullStatus::Skipped { reason, divergence },
                    Some(Ok(())) => match git.reset_hard(&worktree, &remote_ref(repo)) {
                        Ok(()) => PullStatus::Resolved,
                        Err(reset_error) => PullStatus::Skipped {
                            reason: format!("{reason}; and could not resolve: {reset_error}"),
                            divergence,
                        },
                    },
                    Some(Err(blocker)) => PullStatus::Skipped {
                        reason: blocker.reason().to_owned(),
                        divergence,
                    },
                }
            }
        },
    }
}

/// Fetch-and-fast-forward every registered repo — the **Smart Fetch** sweep
/// `session start` runs before a session's view dir exists and `execute
/// tick` runs once per tick before its fan-out.
///
/// Returns a status per repo, in manifest order, instead of warnings: the
/// warning codes differ per caller (`session.smart_fetch_*` vs
/// `execute.tick_smart_fetch_*`), so turning a [`PullStatus`] into a
/// [`Warning`] is the caller's job, not this function's — passing warning
/// codes in here would make `pull` know about verbs it does not own.
pub(crate) fn refresh_all(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    progress: &dyn Progress,
) -> Vec<(RepoName, PullStatus)> {
    let mut results = Vec::new();
    let total = manifest.repos().len();
    for (index, repo) in manifest.repos().iter().enumerate() {
        progress.step(&fetch_step(index, total, repo));
        let status = refresh_default(git, layout, repo, false, false);
        results.push((repo.name().clone(), status));
    }
    progress.clear();
    results
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
#[path = "../../../../tests/unit/action/repo/pull/mod.rs"]
mod tests;
