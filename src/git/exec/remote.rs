use camino::Utf8Path;

use crate::infra::proc;

use super::super::Error;
use super::clone::remote_branch_ref;
use super::{git, run};

/// `git --git-dir <git_dir> fetch --prune --quiet`.
///
/// Touches the network, so it shells out. `--quiet` because the output is
/// evidence, not status — the caller wants the exit code, and git's fetch
/// summary would be noise in every report. A no-op fetch (already up to date)
/// exits zero just like a fetch that pulled commits; with `--quiet` there is
/// no way to tell them apart, and the caller does not need to.
///
/// What it moves is `refs/remotes/origin/*`, via the configured
/// `remote.origin.fetch` refspec.
/// No branch a worktree has checked out is touched, and `--prune` drops
/// tracking refs for branches the remote deleted — never a local branch.
pub(crate) fn fetch(git_dir: &Utf8Path) -> Result<(), Error> {
    run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("fetch")
        .arg("--prune")
        .arg("--quiet"))?;
    Ok(())
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
/// helper lives in that repository's config (see [`super::clone_bare`]), and a bare
/// `git ls-remote <url>` outside it would have no token to offer.
pub(crate) fn remote_branch_tip(
    git_dir: &Utf8Path,
    remote: &str,
    branch: &str,
) -> Result<Option<String>, Error> {
    let stdout = run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("ls-remote")
        .arg(remote)
        .arg(remote_branch_ref(git_dir, branch)))?;
    Ok(stdout.split_whitespace().next().map(str::to_owned))
}

/// `git --git-dir <git_dir> push <remote> <from>:<mapped>`.
///
/// Pushes from the bare clone, which holds every worktree's refs — the feature
/// branch's tip lives there whether or not a worktree is checked out. `remote`
/// is the URL from the manifest, so preview and apply agree on what "the
/// remote" means; `to` is the full ref or branch name the branch lands at.
/// For prefixed repositories, `to` is mapped under [`super::REF_PREFIX_KEY`] to its
/// resolved remote ref before pushing.
///
/// Naming a URL rather than a remote is what makes that agreement possible and
/// is also why [`record_push`] exists: git moves a remote-tracking ref only
/// for a push that named a remote, and writes nothing at all for this one.
pub(crate) fn push(git_dir: &Utf8Path, remote: &str, from: &str, to: &str) -> Result<(), Error> {
    let mapped = to
        .strip_prefix("refs/heads/")
        .map_or_else(|| to.to_owned(), |b| remote_branch_ref(git_dir, b));
    run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("push")
        .arg(remote)
        .arg(format!("{from}:{mapped}")))?;
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
    let _ = run(&git()
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

/// Publish `branch` at exactly `at` on `remote`, refused if `remote` already
/// has `branch` — `git push --force-with-lease="refs/heads/<branch>:"
/// <remote> <at>:refs/heads/<branch>`.
///
/// The empty expected-tip is git's own compare-and-create: the push is
/// refused rather than overwriting an unexpected branch someone else
/// published under this name. On success, [`record_push`] updates the local
/// tracking ref exactly as [`push`] does.
pub(crate) fn publish_remote_branch(
    git_dir: &Utf8Path,
    remote: &str,
    branch: &str,
    at: &str,
) -> Result<(), Error> {
    let to = format!("refs/heads/{branch}");
    let mapped = remote_branch_ref(git_dir, branch);
    run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("push")
        .arg(format!("--force-with-lease={mapped}:"))
        .arg(remote)
        .arg(format!("{at}:{mapped}")))?;
    record_push(git_dir, remote, at, &to);
    Ok(())
}

/// Delete `branch` on `remote`, refused if it moved past `expected_tip` —
/// `git push --force-with-lease="<mapped>:<expected_tip>"
/// <remote> :<mapped>`.
///
/// For prefixed repositories, `branch` is resolved to `<mapped>` under
/// [`super::REF_PREFIX_KEY`] (`refs/heads/<prefix><branch>`).
///
/// The non-empty expected-tip is git's own compare-and-delete: the push is
/// refused ("stale info") if the remote branch is not exactly at
/// `expected_tip`, which is the race guard a rename needs before deleting
/// the branch it just republished under its new name. On success the local
/// `refs/remotes/origin/<branch>` tracking ref is removed, under the same
/// "only when `remote` is origin's own configured URL" condition
/// [`record_push`] applies.
pub(crate) fn delete_remote_branch(
    git_dir: &Utf8Path,
    remote: &str,
    branch: &str,
    expected_tip: &str,
) -> Result<(), Error> {
    let mapped = remote_branch_ref(git_dir, branch);
    run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("push")
        .arg(format!("--force-with-lease={mapped}:{expected_tip}"))
        .arg(remote)
        .arg(format!(":{mapped}")))?;
    record_delete(git_dir, remote, branch);
    Ok(())
}

/// The delete-time counterpart to [`record_push`]: removes the local
/// `refs/remotes/origin/<branch>` tracking ref, and only when `remote` is
/// origin's own configured URL — a ref named `origin` must not be made to
/// forget a branch some other remote deleted. Never reports failure, for the
/// same reason `record_push` does not: the remote deletion has already
/// landed.
fn record_delete(git_dir: &Utf8Path, remote: &str, branch: &str) {
    if origin_url(git_dir).as_deref() != Some(remote) {
        return;
    }
    let _ = run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("update-ref")
        .arg("-d")
        .arg(format!("refs/remotes/origin/{branch}")));
}
