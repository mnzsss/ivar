//! One promoted repo's rebase: the skip checks, the base tip (remote or
//! local), and the rebase itself, aborted on a stop.

use camino::Utf8Path;

use crate::domain::feature::{Feature, Promotion};
use crate::domain::name::{BranchName, RepoName};
use crate::error::{Failure, Warning};
use crate::git::{self, Git};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::base;
use super::types::{BaseSource, RebaseStatus, RepoRebase};

/// A repo left untouched before any rebase ran.
fn skipped(
    repo_name: &RepoName,
    onto: Option<BranchName>,
    code: &'static str,
    what: &str,
) -> (RepoRebase, Vec<Warning>, bool) {
    (
        RepoRebase {
            repo: repo_name.clone(),
            status: RebaseStatus::Skipped,
            onto,
            base_source: None,
        },
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
) -> Result<(RepoRebase, Vec<Warning>, bool), Failure> {
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
            RepoRebase {
                repo: repo_name.clone(),
                status: RebaseStatus::Rebased,
                onto: Some(target),
                base_source: Some(source),
            },
            warnings,
            onto.is_some(),
        )),
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
                RepoRebase {
                    repo: repo_name.clone(),
                    status: RebaseStatus::Conflicted,
                    onto: Some(target),
                    base_source: Some(source),
                },
                warnings,
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
