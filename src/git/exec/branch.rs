use camino::Utf8Path;

use super::super::Error;
use super::{git, run};

/// `git -C <worktree> merge --ff-only FETCH_HEAD`.
///
/// Advances the worktree's checked-out branch (and its files) to the tip the
/// preceding [`super::fetch_branch`] landed in `FETCH_HEAD`. Non-zero when the
/// branches diverged — "cannot fast-forward" — which the caller reports as
/// skipped, never as a batch abort.
pub(crate) fn fast_forward(worktree: &Utf8Path) -> Result<(), Error> {
    run(&git()
        .cwd(worktree)
        .arg("merge")
        .arg("--ff-only")
        .arg("FETCH_HEAD"))?;
    Ok(())
}

/// `git --git-dir <git_dir> branch <branch> <revision>` — create a branch at
/// an explicit revision in the bare repository. The temporary
/// `ivar-integrate/<feature>/<repo>` branches integration uses are created
/// here and deleted with [`delete_branch`] once their worktrees are gone.
pub(crate) fn create_branch(git_dir: &Utf8Path, branch: &str, revision: &str) -> Result<(), Error> {
    run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("branch")
        .arg(branch)
        .arg(revision))?;
    Ok(())
}

/// `git --git-dir <git_dir> branch -D <branch>` — delete a branch. Used only
/// for the temporary `ivar-integrate/...` branches, after their worktrees
/// have been removed (git refuses to delete a checked-out branch, which is
/// the order's guardrail).
pub(crate) fn delete_branch(git_dir: &Utf8Path, branch: &str) -> Result<(), Error> {
    run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("branch")
        .arg("-D")
        .arg(branch))?;
    Ok(())
}

/// `git -C <worktree> merge --no-ff --no-edit <source>` — a merge commit
/// with the default message, never a fast-forward. `--no-edit` is what keeps
/// git from opening an editor for the auto-generated message.
pub(crate) fn merge_no_ff(worktree: &Utf8Path, source: &str) -> Result<(), Error> {
    run(&git()
        .cwd(worktree)
        .arg("merge")
        .arg("--no-ff")
        .arg("--no-edit")
        .arg(source))?;
    Ok(())
}

/// `git -C <worktree> merge --squash <source>` then `git -C <worktree>
/// commit -m <message>` — the squash strategy's two steps, since `--squash`
/// stages without committing.
/// # Why `--no-verify`
///
/// This commit lands on the default branch, which is exactly what the
/// protection hook refuses — so without this, protecting a repo would break
/// `ivar deliver` in that repo. `--no-verify` is written here, at the one call
/// site that commits onto a protected branch, rather than as an environment
/// marker the hook honours: an env var is readable and settable by anything
/// with a shell, which is the audience the hook exists to stop. An argument on
/// this one line is not.
pub(crate) fn squash_merge(worktree: &Utf8Path, source: &str, message: &str) -> Result<(), Error> {
    run(&git().cwd(worktree).arg("merge").arg("--squash").arg(source))?;
    run(&git()
        .cwd(worktree)
        .arg("commit")
        .arg("--no-verify")
        .arg("-m")
        .arg(message))?;
    Ok(())
}

/// `git -C <worktree> merge --ff-only <revision>` — advance the checked-out
/// branch (and its files) to `revision`. Refuses when the branch diverged
/// and cannot fast-forward.
pub(crate) fn fast_forward_to(worktree: &Utf8Path, revision: &str) -> Result<(), Error> {
    run(&git()
        .cwd(worktree)
        .arg("merge")
        .arg("--ff-only")
        .arg(revision))?;
    Ok(())
}

/// `git --git-dir <git_dir> rev-list --count <base>..<branch>` — how many
/// commits `branch` carries beyond `base`.
///
/// Both must exist; a missing revision is git's refusal, surfaced as
/// [`Error::Refused`] with git's own sentence.
pub(crate) fn commits_ahead(git_dir: &Utf8Path, base: &str, branch: &str) -> Result<u64, Error> {
    let command = git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("rev-list")
        .arg("--count")
        .arg(format!("{base}..{branch}"));
    let stdout = run(&command)?;
    let count = stdout.trim().parse::<u64>().map_err(|_| Error::Refused {
        command: format!("git rev-list --count {base}..{branch}"),
        detail: format!("expected a commit count, got `{stdout}`"),
    })?;
    Ok(count)
}

/// `git log --format=%h%x00%B%x1e <base>..<branch>` — each commit's short
/// SHA and full message, newest first.
pub(crate) fn commit_messages(
    git_dir: &Utf8Path,
    base: &str,
    branch: &str,
) -> Result<Vec<(String, String)>, Error> {
    let command = git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("log")
        .arg("--format=%h%x00%B%x1e")
        .arg(format!("{base}..{branch}"));
    let stdout = run(&command)?;
    Ok(stdout
        .split('\x1e')
        .filter_map(|record| record.trim_start().split_once('\0'))
        .map(|(sha, message)| (sha.to_owned(), message.to_owned()))
        .collect())
}

/// `git -C <worktree> reset --hard <revision>` — move the checked-out branch
/// (and its files) to `revision`, discarding local commits beyond it.
///
/// Destructive by definition; the caller has verified the dropped commits are
/// duplicates of work landed elsewhere and that the worktree is clean. Runs
/// inside the worktree, like `fast_forward`.
pub(crate) fn reset_hard(worktree: &Utf8Path, revision: &str) -> Result<(), Error> {
    run(&git().cwd(worktree).arg("reset").arg("--hard").arg(revision))?;
    Ok(())
}

/// `git -C <worktree> rebase <branch>` — replay the worktree's checked-out
/// branch on top of `<branch>`.
///
/// A conflict stops the rebase and exits non-zero — [`run`] turns that into
/// [`Error::Refused`] with git's own stderr — and leaves the worktree in the
/// middle of the rebase. The caller decides what that means (abort and move
/// on, in `feature rebase`'s case); this function's job ends at reporting.
pub(crate) fn rebase_branch(worktree: &Utf8Path, branch: &str) -> Result<(), Error> {
    run(&git().cwd(worktree).arg("rebase").arg(branch))?;
    Ok(())
}

/// `git -C <worktree> rebase --abort` — abandon an in-progress rebase and
/// restore the branch to where it was before it started.
///
/// Refuses (non-zero) when no rebase is in progress — "no rebase in progress"
/// is git's own answer, surfaced as [`Error::Refused`].
pub(crate) fn abort_rebase(worktree: &Utf8Path) -> Result<(), Error> {
    run(&git().cwd(worktree).arg("rebase").arg("--abort"))?;
    Ok(())
}

/// `git --git-dir <git_dir> branch -m <from> <to>` — relabel a local branch.
///
/// A metadata-only ref rename: it relabels `refs/heads/<from>` to
/// `refs/heads/<to>` and updates any worktree's symbolic `HEAD` that pointed
/// at `from`, touching no worktree's index or working tree. Refuses
/// (`Error::Refused`) when `from` does not exist or `to` already does.
pub(crate) fn rename_branch(git_dir: &Utf8Path, from: &str, to: &str) -> Result<(), Error> {
    run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("branch")
        .arg("-m")
        .arg(from)
        .arg(to))?;
    Ok(())
}
