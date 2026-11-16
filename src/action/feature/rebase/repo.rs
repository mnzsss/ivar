//! One promoted repo's rebase: the skip checks, the base tip (remote or
//! local), and the rebase itself, aborted on a stop — or, with
//! `--keep-conflicts`, left in progress for `--continue` / `--abort`.

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::feature::{Feature, Promotion};
use crate::domain::name::{BranchName, RepoName};
use crate::error::{Failure, Warning};
use crate::git::{self, Git};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::base;
use super::types::{BaseSource, RebaseStatus, RepoRebase};

/// One repo's rebase result, the warnings it raised, and whether it actually
/// rebased onto `--onto`'s target — the only repos whose declared base is
/// safe to rewrite once the batch is done.
pub(super) type RepoStep = (RepoRebase, Vec<Warning>, bool);

/// A repo left untouched before any rebase ran.
fn skipped(
    repo_name: &RepoName,
    onto: Option<BranchName>,
    code: &'static str,
    what: &str,
) -> RepoStep {
    (
        RepoRebase::bare(repo_name, RebaseStatus::Skipped, onto, None),
        vec![Warning::new(code, repo_name.as_str(), what)],
        false,
    )
}

/// Rebase one promoted repo's feature-branch worktree onto its target base.
/// The trailing `bool` is whether this repo actually rebased onto `onto`'s
/// target — the only ones whose declared base is safe to rewrite once the
/// batch is done.
#[allow(clippy::too_many_arguments)]
pub(super) fn rebase_one_repo(
    git: &impl Git,
    layout: &Layout,
    feature: &Feature,
    repo_name: &RepoName,
    promotion: &Promotion,
    manifest: &Manifest,
    onto: Option<&BranchName>,
    offline: bool,
    keep_conflicts: bool,
) -> Result<RepoStep, Failure> {
    let worktree = layout.repo_worktree(repo_name, &feature.branch);
    let Some(manifest_repo) = manifest
        .repos()
        .iter()
        .find(|repo| repo.name() == repo_name)
    else {
        return Ok(skipped(
            repo_name,
            None,
            "rebase.repo_not_in_manifest",
            "not in ivar.json; nothing to rebase onto",
        ));
    };
    let target = onto
        .cloned()
        .unwrap_or_else(|| base::resolve(feature, promotion, manifest_repo.default_branch()));
    if !fs::is_dir(&worktree)? {
        return Ok(skipped(
            repo_name,
            Some(target),
            "rebase.no_worktree",
            "no worktree materialised for this repo",
        ));
    }
    // A rebase left in progress also lists its unmerged paths as "dirty", and
    // "commit or stash" would be the wrong advice for it.
    if git.is_rebase_in_progress(&worktree)? {
        return Ok(skipped(
            repo_name,
            Some(target),
            "rebase.in_progress",
            &format!(
                "rebase in progress; use `ivar feature rebase {} --continue` or `--abort`",
                feature.name
            ),
        ));
    }
    // Rebase over uncommitted work is how it gets lost — a dirty worktree is
    // skipped, never rebased around, and never costs a network call.
    if git.worktree_dirty(&worktree)? {
        return Ok(skipped(
            repo_name,
            Some(target),
            "rebase.dirty",
            "worktree has uncommitted changes; commit or stash them first",
        ));
    }
    let (from, source, fallback) = rebase_from(
        git,
        layout,
        repo_name,
        manifest_repo.url(),
        &worktree,
        &target,
        offline,
    );
    let mut warnings: Vec<Warning> = fallback.into_iter().collect();
    match git.rebase_branch(&worktree, &from) {
        Ok(()) => Ok((
            RepoRebase::bare(repo_name, RebaseStatus::Rebased, Some(target), Some(source)),
            warnings,
            onto.is_some(),
        )),
        Err(git::Error::Refused { detail, .. }) if keep_conflicts => {
            let stopped = Stopped {
                repo_name,
                target,
                source,
                detail: &detail,
            };
            stopped.keep(git, &worktree, warnings)
        }
        Err(git::Error::Refused { .. }) => {
            warnings.push(if let Err(abort) = git.abort_rebase(&worktree) {
                Warning::new(
                    "rebase.abort_failed",
                    repo_name.as_str(),
                    format!("could not abort the stopped rebase: {abort}"),
                )
            } else {
                Warning::new(
                    "rebase.conflicted",
                    repo_name.as_str(),
                    "rebase stopped (likely a conflict) and was aborted",
                )
            });
            Ok((
                RepoRebase::bare(
                    repo_name,
                    RebaseStatus::Conflicted,
                    Some(target),
                    Some(source),
                ),
                warnings,
                false,
            ))
        }
        Err(other) => Err(other.into()),
    }
}

/// A `--keep-conflicts` rebase git refused: what it was onto and why it stopped.
struct Stopped<'a> {
    repo_name: &'a RepoName,
    target: BranchName,
    source: BaseSource,
    detail: &'a str,
}

impl Stopped<'_> {
    /// Leave the stopped rebase in progress. Only a rebase that is really
    /// stopped can be resumed: a refusal with none in progress (an unknown
    /// revision, say) is no conflict, and is reported as a failure instead.
    fn keep(
        self,
        git: &impl Git,
        worktree: &Utf8Path,
        mut warnings: Vec<Warning>,
    ) -> Result<RepoStep, Failure> {
        let Self {
            repo_name,
            target,
            source,
            detail,
        } = self;
        if !git.is_rebase_in_progress(worktree)? {
            warnings.push(Warning::new(
                "rebase.failed",
                repo_name.as_str(),
                format!("git refused the rebase and none is in progress: {detail}"),
            ));
            let result =
                RepoRebase::bare(repo_name, RebaseStatus::Skipped, Some(target), Some(source));
            return Ok((result, warnings, false));
        }
        let files = unmerged_paths(git, worktree)?;
        warnings.push(stopped_warning(repo_name, detail, &files));
        let mut result = RepoRebase::bare(
            repo_name,
            RebaseStatus::Conflicted,
            Some(target),
            Some(source),
        );
        result.conflicted_files = files;
        result.in_progress = true;
        Ok((result, warnings, false))
    }
}

/// The paths still unmerged in the worktree, in git's order.
fn unmerged_paths(git: &impl Git, worktree: &Utf8Path) -> Result<Vec<Utf8PathBuf>, Failure> {
    Ok(git
        .conflicts(worktree)?
        .into_iter()
        .map(|conflict| conflict.path)
        .collect())
}

fn join_paths(paths: &[Utf8PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The warning for a rebase left stopped in the worktree: why, which paths,
/// and how to go on.
fn stopped_warning(repo_name: &RepoName, detail: &str, files: &[Utf8PathBuf]) -> Warning {
    let files = if files.is_empty() {
        String::new()
    } else {
        format!("; conflicted: {}", join_paths(files))
    };
    Warning::new(
        "rebase.conflicted",
        repo_name.as_str(),
        format!(
            "rebase stopped and was left in progress ({}){files}; resolve it in the worktree, then `--continue` or `--abort`",
            detail.trim()
        ),
    )
}

/// The worktree of a repo whose rebase `--continue` / `--abort` should act
/// on, or the skipped result when there is nothing to act on.
fn resumable_worktree(
    git: &impl Git,
    layout: &Layout,
    feature: &Feature,
    repo_name: &RepoName,
    verb: &str,
) -> Result<Result<Utf8PathBuf, RepoStep>, Failure> {
    let worktree = layout.repo_worktree(repo_name, &feature.branch);
    if !fs::is_dir(&worktree)? {
        return Ok(Err(skipped(
            repo_name,
            None,
            "rebase.no_worktree",
            "no worktree materialised for this repo",
        )));
    }
    if !git.is_rebase_in_progress(&worktree)? {
        return Ok(Err(skipped(
            repo_name,
            None,
            "rebase.not_in_progress",
            &format!("no rebase in progress; nothing to {verb}"),
        )));
    }
    Ok(Ok(worktree))
}

/// `--continue` for one repo: finish the rebase a `--keep-conflicts` run left
/// in progress, once its conflicts are resolved and staged.
///
/// Still-unmerged paths are refused up front, without running git, so the
/// worktree keeps its state. A rebase that stops again on a later commit is
/// conflicted once more, with that commit's paths.
pub(super) fn continue_one_repo(
    git: &impl Git,
    layout: &Layout,
    feature: &Feature,
    repo_name: &RepoName,
) -> Result<RepoStep, Failure> {
    let worktree = match resumable_worktree(git, layout, feature, repo_name, "continue")? {
        Ok(worktree) => worktree,
        Err(skipped) => return Ok(skipped),
    };
    let stopped = |files: Vec<Utf8PathBuf>, warning: Warning| {
        let mut result = RepoRebase::bare(repo_name, RebaseStatus::Conflicted, None, None);
        result.conflicted_files = files;
        result.in_progress = true;
        (result, vec![warning], false)
    };

    let unresolved = unmerged_paths(git, &worktree)?;
    if !unresolved.is_empty() {
        let list = join_paths(&unresolved);
        let warning = Warning::new(
            "rebase.unresolved",
            repo_name.as_str(),
            format!(
                "unresolved paths remain: {list}; resolve and `git add` them, then `--continue` again"
            ),
        );
        return Ok(stopped(unresolved, warning));
    }
    match git.continue_rebase(&worktree) {
        Ok(()) => Ok((
            RepoRebase::bare(repo_name, RebaseStatus::Rebased, None, None),
            Vec::new(),
            false,
        )),
        Err(error) => {
            let git::Error::Refused { detail, .. } = &error else {
                return Err(error.into());
            };
            if git.is_rebase_in_progress(&worktree)? {
                let files = unmerged_paths(git, &worktree)?;
                let warning = stopped_warning(repo_name, detail, &files);
                Ok(stopped(files, warning))
            } else {
                Err(error.into())
            }
        }
    }
}

/// `--abort` for one repo: give up the rebase a `--keep-conflicts` run left in
/// progress and restore the branch to where it was before.
pub(super) fn abort_one_repo(
    git: &impl Git,
    layout: &Layout,
    feature: &Feature,
    repo_name: &RepoName,
) -> Result<RepoStep, Failure> {
    let worktree = match resumable_worktree(git, layout, feature, repo_name, "abort")? {
        Ok(worktree) => worktree,
        Err(skipped) => return Ok(skipped),
    };
    match git.abort_rebase(&worktree) {
        Ok(()) => Ok((
            RepoRebase::bare(repo_name, RebaseStatus::Skipped, None, None),
            vec![
                Warning::new(
                    "rebase.aborted",
                    repo_name.as_str(),
                    "rebase aborted; the branch is back where it was before the rebase",
                )
                .advisory(),
            ],
            false,
        )),
        Err(git::Error::Refused { detail, .. }) => {
            let mut result = RepoRebase::bare(repo_name, RebaseStatus::Conflicted, None, None);
            result.conflicted_files = unmerged_paths(git, &worktree)?;
            result.in_progress = git.is_rebase_in_progress(&worktree)?;
            Ok((
                result,
                vec![Warning::new(
                    "rebase.abort_failed",
                    repo_name.as_str(),
                    format!("could not abort the rebase: {detail}"),
                )],
                false,
            ))
        }
        Err(other) => Err(other.into()),
    }
}

/// The revision to rebase onto, where it came from, and the warning a
/// fallback carries. Runs only for a clean worktree.
fn rebase_from(
    git: &impl Git,
    layout: &Layout,
    repo_name: &RepoName,
    url: &str,
    worktree: &Utf8Path,
    target: &BranchName,
    offline: bool,
) -> (String, BaseSource, Option<Warning>) {
    if offline {
        return (target.to_string(), BaseSource::Local, None);
    }
    let unreachable = |detail: String| {
        Warning::new(
            "rebase.remote_unreachable",
            repo_name.as_str(),
            format!(
                "could not read `{target}` from the remote ({detail}); rebased onto the local `{target}`, which may be stale. Re-run when the remote is reachable"
            ),
        )
    };
    match git.remote_branch_tip(&layout.repo_bare(repo_name), url, target.as_str()) {
        Ok(Some(_)) => match git.fetch_branch(worktree, target.as_str()) {
            Ok(()) => ("FETCH_HEAD".to_owned(), BaseSource::Remote, None),
            Err(error) => (
                target.to_string(),
                BaseSource::Local,
                Some(unreachable(error.to_string())),
            ),
        },
        Ok(None) => (target.to_string(), BaseSource::Local, None),
        Err(error) => (
            target.to_string(),
            BaseSource::Local,
            Some(unreachable(error.to_string())),
        ),
    }
}
