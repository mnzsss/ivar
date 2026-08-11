//! Mutations and anything touching a remote, through the `git` binary.
//!
//! ADR-0001 §3: reads go through `git2`, writes and network go through the
//! binary. Two reasons, both from measured failure rather than taste — the
//! worst bug in the closest prior art was libgit2's SSH transport against a key
//! held in hardware, and `git worktree add` through the CLI produces a stock
//! layout, which is the cheapest way to keep a third-party git TUI working
//! against a hall.
//!
//! # Failing fast instead of hanging
//!
//! Every invocation here gets `GIT_TERMINAL_PROMPT=0` plus emptied askpass
//! hooks, and [`proc::capture`] gives it `/dev/null` for stdin. Together those
//! make git *refuse* rather than block when it wants a credential it does not
//! have. A blocked prompt is invisible behind a progress line, so it does not
//! read as "waiting for input" — it reads as a hang, and the user kills the
//! process without ever seeing the question.
//!
//! # What is deliberately not set
//!
//! `GIT_SSH_COMMAND` is left alone, even though forcing
//! `ssh -o BatchMode=yes -o ConnectTimeout=10` would make an unreachable host
//! give up sooner. Setting it would clobber a user's own `GIT_SSH_COMMAND` or
//! `core.sshCommand`, which is exactly the setting a corporate bastion or a
//! hardware key needs. Stdin being `/dev/null` already covers the case that
//! matters (no silent prompt); the rest is the user's network, and taking their
//! ssh configuration away to shorten a timeout is a bad trade.

use camino::{Utf8Path, Utf8PathBuf};

use crate::infra::proc;

use super::Error;

/// A `git` invocation with this module's fail-fast environment already applied.
///
/// Every command in this module starts here, so the discipline described in the
/// module doc comment cannot be forgotten at one call site.
fn git() -> proc::Command {
    proc::Command::new("git")
        // Refuse rather than prompt: a prompt nobody can see reads as a hang.
        .env("GIT_TERMINAL_PROMPT", "0")
        // Set-and-empty, not unset — that is how git is told to stop looking
        // for an askpass helper. See `infra::proc::Command::env`.
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
}

/// Run `command`, turning a non-zero exit into [`Error::Refused`] carrying
/// git's own stderr.
///
/// This is the one place a git exit code becomes an error. `infra::proc`
/// deliberately returns it as data; the translation belongs here, where "git
/// said no" is unambiguous, rather than in the general subprocess boundary
/// where it is not.
fn run(command: proc::Command) -> Result<String, Error> {
    let output = proc::capture(&command)?;
    if output.success() {
        return Ok(output.stdout);
    }
    Err(Error::Refused {
        command: command.display(),
        detail: output.diagnostic(),
    })
}

/// `git clone --bare <url> <dest>`.
///
/// For GitHub HTTPS URLs the ivar credential helper is attached to *this
/// invocation only* (`-c credential.helper`), so a private repo clones
/// through `gh`/`$GITHUB_TOKEN` without the token ever being written to
/// `.git/config` — the helper answers on demand and disappears.
pub(crate) fn clone_bare(url: &str, dest: &Utf8Path) -> Result<(), Error> {
    let mut command = git().arg("clone").arg("--bare");
    if crate::infra::github::is_github_https(url) {
        // The helper is invoked by git only when it needs a credential; for a
        // public repo it is never called, so the `-c` has no observable cost.
        command = command
            .arg("-c")
            .arg("credential.helper=!ivar git-credential");
    }
    command = command.arg(url).arg(dest.as_str());
    run(command)?;
    Ok(())
}

/// `git --git-dir <git_dir> worktree add <dest> <branch>`.
///
/// `branch` must already exist. Creating a branch as part of adding a worktree
/// (`worktree add -b`) is a feature-slice concern; `sync` only ever materialises
/// a branch the remote already has.
pub(crate) fn add_worktree(git_dir: &Utf8Path, dest: &Utf8Path, branch: &str) -> Result<(), Error> {
    run(git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("worktree")
        .arg("add")
        .arg(dest.as_str())
        .arg(branch))?;
    Ok(())
}

/// `git --git-dir <git_dir> fetch --prune --quiet`.
///
/// Touches the network, so it shells out. `--quiet` because the output is
/// evidence, not status — the caller wants the exit code, and git's fetch
/// summary would be noise in every report. A no-op fetch (already up to date)
/// exits zero just like a fetch that pulled commits; with `--quiet` there is
/// no way to tell them apart, and the caller does not need to.
pub(crate) fn fetch(git_dir: &Utf8Path) -> Result<(), Error> {
    run(git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("fetch")
        .arg("--prune")
        .arg("--quiet"))?;
    Ok(())
}

/// `git --git-dir <git_dir> worktree add -b <branch> <dest> <from_branch>`.
///
/// The one operation that creates a branch *and* a worktree in a single git
/// call — what `feature promote` needs, and the reason `sync`'s
/// [`add_worktree`] stays strictly branch-exists-only: sync materialises what
/// the remote already has, promote is where new branches are born.
pub(crate) fn create_branch_and_worktree(
    git_dir: &Utf8Path,
    branch: &str,
    from_branch: &str,
    dest: &Utf8Path,
) -> Result<(), Error> {
    run(git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("worktree")
        .arg("add")
        .arg("-b")
        .arg(branch)
        .arg(dest.as_str())
        .arg(from_branch))?;
    Ok(())
}

/// `git -C <worktree> fetch --prune --quiet origin <branch>`.
///
/// Runs *inside* the worktree, not against the bare: a bare clone here has no
/// `remote.origin.fetch` refspec, and fetching straight into the shared
/// `refs/heads/*` is refused by git while the branch is checked out in a
/// worktree. A worktree-local fetch lands in `FETCH_HEAD` and moves nothing —
/// the fast-forward is the separate, deliberate next step.
pub(crate) fn fetch_branch(worktree: &Utf8Path, branch: &str) -> Result<(), Error> {
    run(git()
        .cwd(worktree)
        .arg("fetch")
        .arg("--prune")
        .arg("--quiet")
        .arg("origin")
        .arg(branch))?;
    Ok(())
}

/// `git -C <worktree> merge --ff-only FETCH_HEAD`.
///
/// Advances the worktree's checked-out branch (and its files) to the tip the
/// preceding [`fetch_branch`] landed in `FETCH_HEAD`. Non-zero when the
/// branches diverged — "cannot fast-forward" — which the caller reports as
/// skipped, never as a batch abort.
pub(crate) fn fast_forward(worktree: &Utf8Path) -> Result<(), Error> {
    run(git()
        .cwd(worktree)
        .arg("merge")
        .arg("--ff-only")
        .arg("FETCH_HEAD"))?;
    Ok(())
}

/// `git --git-dir <git_dir> worktree remove --force <dest>`.
///
/// `--force` because a worktree with uncommitted changes is refused by git
/// otherwise, and this is only called from a cascade that has decided the
/// work is being torn down.
pub(crate) fn remove_worktree(git_dir: &Utf8Path, dest: &Utf8Path) -> Result<(), Error> {
    run(git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(dest.as_str()))?;
    Ok(())
}

/// `git -C <worktree> status --porcelain` — whether the worktree holds
/// uncommitted changes.
///
/// Porcelain output is empty exactly when the worktree is clean, so the
/// boolean is the non-emptiness of the captured stdout. Untracked files count
/// as dirty — a push does not carry them, and the preview saying "clean" while
/// `git status` disagrees would be a lie the human acts on.
pub(crate) fn worktree_dirty(path: &Utf8Path) -> Result<bool, Error> {
    let command = git().cwd(path).arg("status").arg("--porcelain");
    let output = proc::capture(&command)?;
    if !output.success() {
        return Err(Error::Refused {
            command: command.display(),
            detail: output.diagnostic(),
        });
    }
    Ok(!output.stdout.is_empty())
}

/// `git -C <worktree> status --porcelain -z --untracked-files=all` — every
/// path in the worktree that diverges from its last commit, as
/// worktree-relative paths. Tracked edits and untracked files alike, which is
/// the difference that matters to the caller: a file an executor *created*
/// outside its write contract is untracked, and invisible to `git diff`.
///
/// `-z` rather than the default line format because the default *quotes* any
/// path holding a space or a non-ASCII byte, and a quoted path is one the
/// caller would have to unquote correctly before deciding whether a write
/// contract covers it. Whether a write is refused must not hinge on getting
/// an escaping dialect right; NUL-separated records need no quoting at all.
///
/// `--untracked-files=all` rather than the default `normal`, which collapses
/// a new directory into the directory name alone — one entry standing for any
/// number of files, none of them named.
///
/// A rename emits two records, the new path then the original. Both are
/// returned: both are writes, since the file at the old path is gone.
pub(crate) fn changed_paths(path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, Error> {
    let command = git()
        .cwd(path)
        .arg("status")
        .arg("--porcelain")
        .arg("-z")
        .arg("--untracked-files=all");
    let output = proc::capture(&command)?;
    if !output.success() {
        return Err(Error::Refused {
            command: command.display(),
            detail: output.diagnostic(),
        });
    }
    Ok(parse_status_z(&output.stdout))
}

/// The paths named by `git status --porcelain -z` output.
///
/// Each record is `XY<space><path>`: two status columns, then the path. A
/// record whose first column is `R` or `C` (rename, copy) is followed by a
/// second record carrying the origin path, which is taken as well.
fn parse_status_z(stdout: &str) -> Vec<Utf8PathBuf> {
    let mut paths = Vec::new();
    let mut records = stdout.split('\0').filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        let mut columns = record.chars();
        let (Some(index_status), Some(_worktree_status)) = (columns.next(), columns.next()) else {
            continue;
        };
        let entry = columns.as_str().trim_start();
        if entry.is_empty() {
            continue;
        }
        paths.push(Utf8PathBuf::from(entry));
        if matches!(index_status, 'R' | 'C')
            && let Some(origin) = records.next()
        {
            paths.push(Utf8PathBuf::from(origin));
        }
    }
    paths
}

/// `git -C <worktree> diff HEAD` — the worktree's uncommitted divergence from
/// its last commit, staged and unstaged.
///
/// Empty when the worktree is clean. Untracked files are invisible to
/// `git diff` by design, so "clean" means "no tracked content diverged" — the
/// caller (reconcile) wants the code divergence an executor left uncommitted,
/// which is always a tracked edit.
pub(crate) fn diff_worktree(path: &Utf8Path) -> Result<String, Error> {
    let command = git().cwd(path).arg("diff").arg("HEAD");
    let output = proc::capture(&command)?;
    if !output.success() {
        return Err(Error::Refused {
            command: command.display(),
            detail: output.diagnostic(),
        });
    }
    Ok(output.stdout)
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
    let stdout = run(command)?;
    let count = stdout.trim().parse::<u64>().map_err(|_| Error::Refused {
        command: format!("git rev-list --count {base}..{branch}"),
        detail: format!("expected a commit count, got `{stdout}`"),
    })?;
    Ok(count)
}

/// `git --git-dir <git_dir> rev-parse --abbrev-ref --symbolic-full-name
/// <branch>@{upstream}` — whether `branch` has an upstream configured.
///
/// A non-zero exit *is* the answer here — "no upstream configured for branch
/// 'x'" is git's refusal, and the most useful sentence about why. That is why
/// this does not go through [`run`], which turns every refusal into an error:
/// the caller wants a `bool`. Callers must ensure `git_dir` is a repository
/// first — a non-repository also exits non-zero, and would be read as "no
/// upstream".
pub(crate) fn has_upstream(git_dir: &Utf8Path, branch: &str) -> Result<bool, Error> {
    let command = git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("--symbolic-full-name")
        .arg(format!("{branch}@{{upstream}}"));
    let output = proc::capture(&command)?;
    Ok(output.success())
}

/// `git --git-dir <git_dir> push <remote> <from>:<to>`.
///
/// Pushes from the bare clone, which holds every worktree's refs — the feature
/// branch's tip lives there whether or not a worktree is checked out. `remote`
/// is the URL from the manifest, so preview and apply agree on what "the
/// remote" means; `to` is the full ref the branch lands at.
pub(crate) fn push(git_dir: &Utf8Path, remote: &str, from: &str, to: &str) -> Result<(), Error> {
    run(git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("push")
        .arg(remote)
        .arg(format!("{from}:{to}")))?;
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
    run(git().cwd(worktree).arg("rebase").arg(branch))?;
    Ok(())
}

/// `git -C <worktree> rebase --abort` — abandon an in-progress rebase and
/// restore the branch to where it was before it started.
///
/// Refuses (non-zero) when no rebase is in progress — "no rebase in progress"
/// is git's own answer, surfaced as [`Error::Refused`].
pub(crate) fn abort_rebase(worktree: &Utf8Path) -> Result<(), Error> {
    run(git().cwd(worktree).arg("rebase").arg("--abort"))?;
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/unit/git/exec.rs"]
mod tests;
