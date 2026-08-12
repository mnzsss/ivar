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

/// The fetch refspec every hall bare clone gets.
///
/// `git clone --bare` configures *no* `remote.origin.fetch` at all, so a bare
/// clone left as git makes it has an empty `refs/remotes/`. That is invisible
/// until something asks for a remote-tracking ref, and then it fails in a hall
/// and nowhere else:
///
/// * `git push --force-with-lease` with no explicit expectation leases against
///   `refs/remotes/origin/<branch>`; a ref that does not exist reads as
///   "stale info", and the only way out is passing the SHA by hand.
/// * `<branch>@{upstream}` does not resolve and `git status` reports no
///   ahead/behind — in a worktree a human works in every day.
///
/// What the refspec does *not* fix is a push made to a URL rather than to a
/// named remote: git records nothing local about it. That is why "already
/// pushed" is asked of the remote by [`remote_branch_tip`] and never of the
/// local config.
///
/// Fetching into `refs/remotes/*` rather than `refs/heads/*` is also what makes
/// the refspec safe here: git refuses to fetch into a branch that is checked
/// out in a worktree, and in a hall every branch is.
const REMOTE_TRACKING_REFSPEC: &str = "+refs/heads/*:refs/remotes/origin/*";

/// `git clone --bare <url> <dest>`.
///
/// Both `-c` settings sit *after* `clone`, which is the persisting form: git
/// writes them into the new repository's config rather than applying them to
/// this invocation. That is deliberate for each.
///
/// The refspec has to persist — see [`REMOTE_TRACKING_REFSPEC`] for what breaks
/// without it, all of it long after the clone returned.
///
/// For GitHub HTTPS URLs the credential helper persists too, because the clone
/// is not the last time this repo talks to the remote: every later fetch and
/// every push a human makes from a worktree needs the same token. What is
/// written to `.git/config` is the *command*, never its answer — the token is
/// re-derived from `gh`/`$GITHUB_TOKEN` on each call and never lands on disk,
/// which is the property that matters and the reason a helper is registered
/// instead of a credential stored.
pub(crate) fn clone_bare(url: &str, dest: &Utf8Path) -> Result<(), Error> {
    let mut command = git()
        .arg("clone")
        .arg("--bare")
        .arg("-c")
        .arg(format!("remote.origin.fetch={REMOTE_TRACKING_REFSPEC}"));
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

/// Point `git_dir`'s origin at [`REMOTE_TRACKING_REFSPEC`], whatever it was
/// set to before.
///
/// The repair path for halls cloned by a build that did not configure it.
/// Re-cloning is not an option once feature branches live in the bare, so
/// `sync` sets it in place on every run; the refs themselves appear at the
/// next fetch.
///
/// `--replace-all` rather than plain set: a key with several values makes
/// `git config` refuse outright, and this is the one refspec the bare is
/// supposed to have — collapsing to it is the point, and the bare under
/// `.ivar/repos/` is ivar's to normalise.
pub(crate) fn ensure_remote_tracking(git_dir: &Utf8Path) -> Result<(), Error> {
    run(git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("config")
        .arg("--replace-all")
        .arg("remote.origin.fetch")
        .arg(REMOTE_TRACKING_REFSPEC))?;
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
///
/// What it moves is `refs/remotes/origin/*`, via [`REMOTE_TRACKING_REFSPEC`].
/// No branch a worktree has checked out is touched, and `--prune` drops
/// tracking refs for branches the remote deleted — never a local branch.
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
/// Runs *inside* the worktree, not against the bare, so what it fetches lands
/// in `FETCH_HEAD` and no branch ref moves — the fast-forward is the separate,
/// deliberate next step, and a feature worktree sharing this bare's refs is
/// untouched by a default-branch refresh.
///
/// [`REMOTE_TRACKING_REFSPEC`] still applies: git updates
/// `refs/remotes/origin/<branch>` opportunistically alongside `FETCH_HEAD`, so
/// a `--force-with-lease` from this worktree has something to lease against
/// after a `repo pull`.
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

/// `git --git-dir <git_dir> ls-remote <remote> refs/heads/<branch>` — the
/// commit `remote` holds `branch` at, or `None` when it does not have it.
///
/// This is what "already pushed" is made of, and it is asked of the remote
/// because every local stand-in for it lies about a push ivar made: [`push`]
/// goes to a URL, not to a named remote, and git writes neither an upstream
/// nor a remote-tracking ref for such a push. A branch ivar pushed itself
/// would read as unpushed forever if this asked the config instead.
///
/// `--git-dir` is what makes it work against a private repo: the credential
/// helper lives in that repository's config (see [`clone_bare`]), and a bare
/// `git ls-remote <url>` outside it would have no token to offer.
pub(crate) fn remote_branch_tip(
    git_dir: &Utf8Path,
    remote: &str,
    branch: &str,
) -> Result<Option<String>, Error> {
    let stdout = run(git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("ls-remote")
        .arg(remote)
        .arg(format!("refs/heads/{branch}")))?;
    Ok(stdout.split_whitespace().next().map(str::to_owned))
}

/// `git --git-dir <git_dir> push <remote> <from>:<to>`.
///
/// Pushes from the bare clone, which holds every worktree's refs — the feature
/// branch's tip lives there whether or not a worktree is checked out. `remote`
/// is the URL from the manifest, so preview and apply agree on what "the
/// remote" means; `to` is the full ref the branch lands at.
///
/// Naming a URL rather than a remote is what makes that agreement possible and
/// is also why [`record_push`] exists: git moves a remote-tracking ref only
/// for a push that named a remote, and writes nothing at all for this one.
pub(crate) fn push(git_dir: &Utf8Path, remote: &str, from: &str, to: &str) -> Result<(), Error> {
    run(git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("push")
        .arg(remote)
        .arg(format!("{from}:{to}")))?;
    record_push(git_dir, remote, from, to);
    Ok(())
}

/// Move the remote-tracking ref git would have moved itself, had [`push`]
/// named a remote instead of a URL.
///
/// Without this the bare's `refs/remotes/origin/<branch>` never learns about a
/// push ivar made. That ref is what `git push --force-with-lease` leases
/// against, so a human who rewrites a commit `deliver` already pushed is
/// refused for "stale info" — and nothing repairs it, because nothing fetches
/// between a delivery and the next thing a human does in their worktree.
///
/// Two things this deliberately does not do. It does not record a push aimed
/// anywhere but origin's own URL — a ref named `origin` must not be made to
/// claim a commit origin has never seen. And it does not report failure: the
/// push has already landed, and a bookkeeping write that did not stick cannot
/// be allowed to turn a delivered branch into a failed one.
fn record_push(git_dir: &Utf8Path, remote: &str, from: &str, to: &str) {
    let Some(branch) = to.strip_prefix("refs/heads/") else {
        return;
    };
    if origin_url(git_dir).as_deref() != Some(remote) {
        return;
    }
    let _ = run(git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("update-ref")
        .arg(format!("refs/remotes/origin/{branch}"))
        .arg(from));
}

/// `remote.origin.url`, or `None` when origin has none — which includes
/// `git_dir` not being a repository at all.
fn origin_url(git_dir: &Utf8Path) -> Option<String> {
    let output = proc::capture(
        &git()
            .arg("--git-dir")
            .arg(git_dir.as_str())
            .arg("config")
            .arg("--get")
            .arg("remote.origin.url"),
    )
    .ok()?;
    output.success().then(|| output.stdout.trim().to_owned())
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
