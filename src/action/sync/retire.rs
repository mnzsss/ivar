//! Retiring the checkout a repo's previous default branch left behind.
//!
//! `ivar.json` records only the *current* `defaultBranch`, so after it changes
//! `sync` creates the new branch's worktree and the old one is nobody's: not
//! the default, not a feature's. Its directory still carries the read-only
//! guard ivar put on it, which is the one trace of "this was a default or base
//! checkout" the disk keeps — and so the test for what may be retired.
//!
//! Retirement removes the worktree only; the branch stays. It happens only when
//! nothing would be lost: a clean tree whose commits are all already in the new
//! default branch. Anything else is a [`Warning`] and is left alone.

use camino::Utf8Path;

use crate::action::repo::pull::diagnosis::landed_upstream;
use crate::error::Warning;
use crate::git::{self, WorktreeEntry};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Repo};

use super::{Change, Entry};

/// Retire every guarded checkout of `repo` that is neither its current default
/// branch nor a feature's, when that is safe; warn about the ones it is not
/// safe for.
///
/// Called only once the new default worktree exists, so a failure earlier in
/// the sync never leaves a repo with no default checkout at all.
pub(super) fn retire_stale_default_checkouts(
    git: &impl git::Git,
    layout: &Layout,
    manifest: &Manifest,
    repo: &Repo,
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
) {
    let surface = format!("repo {}", repo.name());
    let bare = layout.repo_bare(repo.name());
    let Ok(worktrees) = git.list_worktrees(&bare) else {
        // `doctor` reports a repo whose worktrees cannot be listed.
        return;
    };

    // Computed lazily: a repo with no guarded stray checkout (nearly all of
    // them) never reads the feature records.
    let mut owned = None;
    for entry in worktrees {
        let Some(branch) = stale_candidate(layout, repo, &entry) else {
            continue;
        };
        let owned = owned.get_or_insert_with(|| {
            crate::action::hall::feature_worktrees(layout, manifest).map_err(|findings| {
                findings
                    .into_iter()
                    .map(|finding| finding.what)
                    .collect::<Vec<_>>()
                    .join("; ")
            })
        });
        let owned = match owned {
            Ok(owned) => owned,
            Err(reason) => {
                warnings.push(not_retired(
                    repo,
                    &entry.path,
                    &format!("feature ownership could not be read: {reason}"),
                ));
                continue;
            }
        };
        if owned.contains(&(repo.name().clone(), branch.to_owned())) {
            continue;
        }

        match unsafe_reason(git, &bare, repo, &entry.path, branch) {
            Some(reason) => warnings.push(not_retired(repo, &entry.path, &reason)),
            None => match git.remove_worktree(&bare, &entry.path) {
                Ok(()) => entries.push(
                    Entry::new(&surface, format!("worktree {branch}"), Change::Removed)
                        .detail(format!("no longer the default branch; `{branch}` is kept")),
                ),
                Err(error) => warnings.push(not_retired(
                    repo,
                    &entry.path,
                    &format!("git could not remove it: {error}"),
                )),
            },
        }
    }
}

/// The branch of `entry` when it looks like a checkout of an earlier default
/// or base branch: live, on a branch other than the current default, under the
/// repo's own directory, and carrying ivar's read-only guard.
///
/// Feature ownership is checked by the caller, after this cheap filter.
fn stale_candidate<'a>(layout: &Layout, repo: &Repo, entry: &'a WorktreeEntry) -> Option<&'a str> {
    let branch = entry.branch.as_deref()?;
    if entry.prunable
        || branch == repo.default_branch().as_str()
        || !entry.path.starts_with(layout.repo_dir(repo.name()))
    {
        return None;
    }
    let guarded = fs::unix_mode(&entry.path)
        .ok()
        .flatten()
        .is_some_and(|mode| mode & 0o222 == 0);
    guarded.then_some(branch)
}

/// Why `branch`'s checkout at `worktree` cannot be removed without losing
/// something, or `None` when it can.
///
/// Conservative in the way `repo pull --resolve` is: a tree whose state cannot
/// be read is dirty, and a containment that cannot be proven is not contained.
fn unsafe_reason(
    git: &impl git::Git,
    bare: &Utf8Path,
    repo: &Repo,
    worktree: &Utf8Path,
    branch: &str,
) -> Option<String> {
    let default = repo.default_branch().as_str();
    if git.worktree_dirty(worktree).unwrap_or(true) {
        return Some("it has uncommitted changes, or its state could not be read".to_owned());
    }
    let contained = match git.is_ancestor(bare, branch, default) {
        Ok(true) => true,
        Ok(false) => git
            .divergence(worktree, branch, default)
            .is_ok_and(|divergence| landed_upstream(git, worktree, branch, default, &divergence)),
        Err(_) => false,
    };
    (!contained).then(|| format!("`{branch}` has commits that `{default}` does not contain"))
}

fn not_retired(repo: &Repo, path: &Utf8Path, reason: &str) -> Warning {
    Warning::new(
        "sync.stale_default_checkout_kept",
        format!("repo {}", repo.name()),
        format!(
            "`{path}` is a checkout of a branch that is no longer the default, and was left in \
             place: {reason}. Inspect it, then remove it with `git worktree remove`."
        ),
    )
}
