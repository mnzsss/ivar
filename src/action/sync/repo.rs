//! The repo half of `ivar sync`: bare clone, default-branch worktree, and
//! the diagnostics for when git refuses.

use camino::Utf8Path;

use crate::domain::name::BranchName;
use crate::error::{Failure, FixAction, Warning};
use crate::git::{self, TargetState};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Repo;

use super::setup::run_setup_script;
use super::{Change, Entry, record_failure};

pub(crate) fn sync_repo(
    git: &impl git::Git,
    layout: &Layout,
    repo: &Repo,
    force_setup: bool,
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
) {
    let name = repo.name();
    let surface = format!("repo {name}");
    let bare = layout.repo_bare(name);
    let branch = repo.default_branch();
    let worktree = layout.repo_worktree(name, branch);

    match ensure_bare(git, repo, &bare) {
        Ok(change) => entries.push(Entry::new(&surface, "bare clone", change)),
        Err(failure) => return record_failure(entries, warnings, &surface, "bare clone", failure),
    }

    match ensure_worktree(git, &bare, &worktree, branch) {
        Ok(change) => entries.push(Entry::new(&surface, format!("worktree {branch}"), change)),
        Err(failure) => {
            return record_failure(entries, warnings, &surface, "worktree", failure);
        }
    }

    match run_setup_script(git, layout, repo, &worktree, &surface, force_setup) {
        // No script for this repo. Silence rather than a "nothing to do" line —
        // most repos will never have one, and a report is only readable if
        // every line in it is about something that exists.
        Ok(None) => {}
        Ok(Some(entry)) => entries.push(entry),
        Err(failure) => record_failure(entries, warnings, &surface, "setup script", failure),
    }

    // After the setup script, deliberately. A project's `pnpm install` may
    // install husky, which writes `core.hooksPath` into the shared config and
    // would disable a hook installed before it ran. Going last means ivar's
    // worktree-local override is written over whatever the script left behind.
    match git.protect_default_branch(&bare, &worktree, branch.as_str()) {
        Ok(git::Protection::Installed) => {
            entries.push(Entry::new(&surface, "branch protection", Change::Created));
        }
        Ok(git::Protection::AlreadyInstalled) => {}
        Err(error) => record_failure(
            entries,
            warnings,
            &surface,
            "branch protection",
            error.into(),
        ),
    }
}

pub(crate) fn ensure_bare(
    git: &impl git::Git,
    repo: &Repo,
    bare: &Utf8Path,
) -> Result<Change, Failure> {
    match git.target_state(bare)? {
        TargetState::Repository => {
            // Halls cloned before the remote-tracking refspec existed have an
            // empty `refs/remotes/`, and a `--force-with-lease` in them refuses
            // with "stale info". Setting it is idempotent and touches no ref,
            // so it is not a change worth reporting — but it has to happen on
            // an existing bare, because re-cloning is not an option once the
            // hall's feature branches live there.
            git.ensure_remote_tracking(bare)?;
            Ok(Change::Unchanged)
        }
        TargetState::Occupied => Err(occupied(
            bare,
            "sync.bare_not_a_repository",
            "a bare clone",
            "sync.remove_partial_clone",
        )),
        TargetState::Absent => {
            if let Some(parent) = bare.parent() {
                fs::ensure_dir(parent)?;
            }
            git.clone_bare(repo.url(), bare)?;
            Ok(Change::Created)
        }
    }
}

pub(crate) fn ensure_worktree(
    git: &impl git::Git,
    bare: &Utf8Path,
    worktree: &Utf8Path,
    branch: &BranchName,
) -> Result<Change, Failure> {
    match git.target_state(worktree)? {
        TargetState::Repository => Ok(Change::Unchanged),
        TargetState::Occupied => Err(occupied(
            worktree,
            "sync.worktree_path_occupied",
            "a worktree",
            "sync.clear_worktree_path",
        )),
        TargetState::Absent => {
            // A branch name may contain `/`, which nests. git creates the leaf
            // itself but the intermediate directories are ours.
            if let Some(parent) = worktree.parent() {
                fs::ensure_dir(parent)?;
            }
            git.add_worktree(bare, worktree, branch.as_str())
                .map_err(|error| explain_missing_branch(git, bare, branch, error))?;
            Ok(Change::Created)
        }
    }
}

pub(crate) fn occupied(
    path: &Utf8Path,
    code: &'static str,
    expected: &str,
    fix_code: &'static str,
) -> Failure {
    Failure::blocked(code, format!("`{path}` exists but is not {expected}"))
        .expected(format!("{expected}, or nothing at all"))
        .actual("a directory that git does not recognise")
        .fix(FixAction::unsafe_(
            fix_code,
            format!(
                "Remove `{path}` and run `ivar sync` again — check first that nothing of yours is in it."
            ),
        ))
}

pub(crate) fn explain_missing_branch(
    git: &impl git::Git,
    bare: &Utf8Path,
    branch: &BranchName,
    error: git::Error,
) -> Failure {
    let Ok(default) = git.head_branch(bare) else {
        return error.into();
    };
    if default == branch.as_str() {
        return error.into();
    }

    // The default branch goes in `what`, not only in `actual`: a per-item
    // failure reaches the user through `record_failure`, which keeps the
    // sentence and drops everything around it. A message whose useful half
    // lives in a field nobody renders is a message that did not get delivered.
    Failure::blocked(
        "sync.branch_not_in_repo",
        format!("`{branch}` is not a branch in this repository; its default branch is `{default}`"),
    )
    .expected(format!("a branch named `{branch}`, as `ivar.json` declares"))
    .actual(format!("this repository's default branch is `{default}`"))
    .fix(FixAction::safe(
        "sync.correct_default_branch",
        format!("Set this repo's `default_branch` to `{default}` in `ivar.json`, then run `ivar sync` again."),
    ))
}
