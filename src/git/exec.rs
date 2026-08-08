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

use camino::Utf8Path;

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
/// One attempt at the URL as given. See the `git` module doc comment for why
/// the predecessor's SSH-then-HTTPS-via-`gh` fallback is not here yet.
pub(crate) fn clone_bare(url: &str, dest: &Utf8Path) -> Result<(), Error> {
    run(git().arg("clone").arg("--bare").arg(url).arg(dest.as_str()))?;
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
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::test_support::{seeded_repo, utf8_temp_dir};

    #[test]
    fn clone_bare_produces_a_bare_repository() {
        let (_guard, dir) = utf8_temp_dir();
        let origin = seeded_repo(&dir.join("origin"), "main");
        let bare = dir.join("api.bare");

        clone_bare(origin.as_str(), &bare).unwrap();

        // A bare clone has its object database at the top level, not under
        // `.git/` — that is what makes it a valid `--git-dir`.
        assert!(bare.join("HEAD").is_file());
        assert!(bare.join("objects").is_dir());
        assert!(!bare.join(".git").exists());
    }

    #[test]
    fn a_failing_clone_is_refused_with_the_invocation_and_gits_diagnostic() {
        let (_guard, dir) = utf8_temp_dir();

        let error = clone_bare(dir.join("nowhere").as_str(), &dir.join("dest"))
            .expect_err("nothing to clone");

        match error {
            Error::Refused { command, detail } => {
                assert!(command.starts_with("git clone --bare"), "was: {command}");
                assert!(!detail.is_empty(), "git said nothing about why");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn add_worktree_checks_out_the_branchs_content() {
        let (_guard, dir) = utf8_temp_dir();
        let origin = seeded_repo(&dir.join("origin"), "main");
        let bare = dir.join("api.bare");
        clone_bare(origin.as_str(), &bare).unwrap();
        let worktree = dir.join("api/main");

        add_worktree(&bare, &worktree, "main").unwrap();

        assert_eq!(
            std::fs::read_to_string(worktree.join("README.md")).unwrap(),
            "seed\n"
        );
    }

    #[test]
    fn adding_a_worktree_on_a_branch_that_does_not_exist_is_refused() {
        let (_guard, dir) = utf8_temp_dir();
        let origin = seeded_repo(&dir.join("origin"), "main");
        let bare = dir.join("api.bare");
        clone_bare(origin.as_str(), &bare).unwrap();

        let error = add_worktree(&bare, &dir.join("wt"), "no-such-branch")
            .expect_err("branch does not exist");

        assert!(matches!(error, Error::Refused { .. }));
    }

    /// A worktree path that already holds something is git's call to refuse,
    /// not this module's to pre-empt. Duplicating the check here would mean two
    /// answers to one question, and git's is the one that is always right.
    #[test]
    fn adding_a_worktree_over_an_occupied_path_is_refused_by_git() {
        let (_guard, dir) = utf8_temp_dir();
        let origin = seeded_repo(&dir.join("origin"), "main");
        let bare = dir.join("api.bare");
        clone_bare(origin.as_str(), &bare).unwrap();
        let occupied = dir.join("occupied");
        std::fs::create_dir_all(&occupied).unwrap();
        std::fs::write(occupied.join("something"), "here").unwrap();

        let error = add_worktree(&bare, &occupied, "main").expect_err("path is occupied");

        assert!(matches!(error, Error::Refused { .. }));
    }
}
