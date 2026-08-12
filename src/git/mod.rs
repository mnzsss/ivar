//! The only module that knows git exists.
//!
//! # One trait, two backends
//!
//! Reads go through `git2` ([`read`]); mutations and anything touching a remote
//! shell out to the `git` binary ([`exec`]). Callers see [`Git`] and cannot tell
//! which half answered. ADR-0001 §3 has the full reasoning; the short version is
//! that the worst bug in the closest prior art was libgit2's SSH transport
//! against a key held in hardware, and that `git worktree add` through the CLI
//! produces a stock layout — which is the cheapest way to pass the third-party
//! git-TUI gate (`lazygit` broke on output that was perfectly legal).
//!
//! The split also makes `depends=('git')` in the PKGBUILD coherent rather than
//! contradictory: the binary genuinely is required.
//!
//! # Why this is a trait when nothing mocks it
//!
//! Tests never fake git. `tempfile::TempDir` plus a real `git init` is fast,
//! hermetic, and exercises the thing that actually ships — that rule is in
//! ARCHITECTURE.md and this module keeps it.
//!
//! The trait is not there for substitution. It is there so `action` is generic
//! over [`Git`] and therefore *cannot* reach around it into [`exec`] or
//! [`read`] for the one call where shelling out looked easier. Which backend
//! serves an operation is this module's decision to change, and a caller that
//! named `exec::clone_bare` directly would freeze it.
//!
//! # Layering
//!
//! `git` may import `infra` and `error`. Not `domain` — so branch and repo
//! names arrive here as `&str`, already validated by the newtypes in
//! [`crate::domain::name`] before they get this far. This module re-validates
//! nothing and assumes nothing; it passes what it is given to git and reports
//! what git says.
//!
// `pub(crate)`, so the boundary above is a fact and not a promise: nothing
// outside this module — in this crate or out of it — can name
// `git::exec::clone_bare` and freeze which backend serves an operation.
pub mod credential;
pub(crate) mod error;
pub(crate) mod exec;
pub(crate) mod read;

pub use self::error::Error;

use camino::{Utf8Path, Utf8PathBuf};

/// What is at a path, as far as git is concerned.
///
/// Three states rather than a `bool`, because the two non-repository cases need
/// different answers: nothing there means "go ahead and create it", and
/// something-git-does-not-recognise means "stop and ask a human". Collapsing
/// them would hand a partial clone straight back to `git clone`, whose refusal
/// names the symptom (target not empty) and not the cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetState {
    /// Nothing exists at the path.
    Absent,
    /// A git repository — bare, or with a worktree.
    Repository,
    /// Something exists and git does not recognise it: a partial clone, or a
    /// directory made by hand.
    Occupied,
}

/// Everything `ivar` asks git to do.
///
/// See the module doc comment for why this is a trait. [`System`] is the one
/// production implementation; it routes each operation to whichever backend
/// ADR-0001 §3 assigns it.
pub trait Git {
    /// What is at `path`: a repository, something else, or nothing.
    ///
    /// Answers about `path` exactly, never about a repository above it. A
    /// walk-up answer would make "is this worktree materialised?" return true
    /// for an empty directory inside a hall that happens to be a git repo
    /// itself, which is precisely the question `sync` asks.
    ///
    /// This is one operation rather than "is it a repo?" plus "does anything
    /// exist?" at the caller, because every caller needs the same three-way
    /// answer and assembling it twice is how the two assemblies diverge.
    fn target_state(&self, path: &Utf8Path) -> Result<TargetState, Error>;

    /// The branch `HEAD` points at in the repository at `git_dir`, without the
    /// `refs/heads/` prefix.
    ///
    /// Reads the symbolic ref rather than resolving it, so a freshly cloned
    /// repository whose default branch has no commits yet still answers. That
    /// is not a corner case: `git clone --bare` of an empty repository is how a
    /// hall gets its first repo when the remote was created moments ago.
    fn head_branch(&self, git_dir: &Utf8Path) -> Result<String, Error>;

    /// The git administrative directory backing the worktree at `path`.
    ///
    /// For a linked worktree this is `<bare>/worktrees/<name>/`, not the bare
    /// repository itself. That distinction is what makes it a good home for
    /// per-worktree bookkeeping: removing and re-adding the worktree destroys
    /// it, so nothing stale survives into a worktree that has been rebuilt from
    /// scratch.
    fn worktree_git_dir(&self, path: &Utf8Path) -> Result<Utf8PathBuf, Error>;

    /// Clone `url` into `dest` as a bare repository.
    ///
    /// Touches the network, so it shells out. `dest` must not exist; git
    /// refuses a non-empty target itself, and this module does not second-guess
    /// that.
    fn clone_bare(&self, url: &str, dest: &Utf8Path) -> Result<(), Error>;

    /// Configure the bare repository at `git_dir` to keep remote-tracking refs
    /// under `refs/remotes/origin/*`.
    ///
    /// [`Self::clone_bare`] already does this for a repo it created; this is
    /// the repair for the ones cloned before it did. Idempotent, so `sync` can
    /// call it on every run without asking whether it is needed.
    ///
    /// A bare clone with no fetch refspec — git's own default — has an empty
    /// `refs/remotes/`, and that is what makes `git push --force-with-lease`
    /// answer "stale info" in a hall and nowhere else: with no tracking ref
    /// there is nothing to lease against.
    fn ensure_remote_tracking(&self, git_dir: &Utf8Path) -> Result<(), Error>;

    /// Add a worktree at `dest`, checked out on the existing `branch`, off the
    /// bare repository at `git_dir`.
    fn add_worktree(&self, git_dir: &Utf8Path, dest: &Utf8Path, branch: &str) -> Result<(), Error>;

    /// Fetch from the remote configured in `git_dir`, pruning deleted
    /// refs. `Ok(())` means the fetch completed — with `--quiet`, a
    /// no-op fetch is indistinguishable from one that pulled new commits,
    /// and that is fine: the exit code is the answer.
    fn fetch(&self, git_dir: &Utf8Path) -> Result<(), Error>;

    /// Every local branch in `git_dir`, without the `refs/heads/` prefix.
    fn list_branches(&self, git_dir: &Utf8Path) -> Result<Vec<String>, Error>;

    /// Create `branch` off `from_branch` in the bare repository at `git_dir`
    /// and add a worktree at `dest` checked out on it — `git worktree add -b
    /// <branch> <dest> <from_branch>` in one call.
    ///
    /// `from_branch` must exist; git refuses otherwise. This is the one
    /// worktree-creating operation that also creates its branch, which is
    /// exactly what `feature promote` needs and what `sync`'s
    /// [`Self::add_worktree`] deliberately does not do.
    fn create_branch_and_worktree(
        &self,
        git_dir: &Utf8Path,
        branch: &str,
        from_branch: &str,
        dest: &Utf8Path,
    ) -> Result<(), Error>;

    /// Fetch `branch` from the remote into the worktree at `path` — the fetch
    /// half of a default-branch refresh (`repo pull`).
    ///
    /// Runs *inside* the worktree (`git -C`), which is what makes it safe in
    /// this architecture: the fetch lands in `FETCH_HEAD` and moves no branch
    /// ref, so a feature worktree's branch — sharing this bare's refs — is
    /// untouched. Remote-tracking refs are updated alongside it, so a lease
    /// taken after a `repo pull` is current.
    fn fetch_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), Error>;

    /// Fast-forward the worktree at `path` to the tip its preceding
    /// [`Self::fetch_branch`] left in `FETCH_HEAD` — `git merge --ff-only
    /// FETCH_HEAD`.
    ///
    /// Advances the worktree's checked-out branch and its files. Refuses when
    /// the branch diverged and cannot be fast-forwarded, which the caller
    /// reports as "skipped" — never as a batch abort.
    fn fast_forward(&self, worktree: &Utf8Path) -> Result<(), Error>;

    /// Remove the worktree at `dest` from the repository at `git_dir`.
    ///
    /// Forced: git refuses to remove a worktree with uncommitted changes, and
    /// that refusal is the guard a cascade caller (repo deregister) has
    /// already decided to override before it calls here.
    fn remove_worktree(&self, git_dir: &Utf8Path, dest: &Utf8Path) -> Result<(), Error>;

    /// Whether the worktree at `path` has uncommitted changes — tracked or
    /// untracked. Empty `git status --porcelain` output means clean.
    fn worktree_dirty(&self, path: &Utf8Path) -> Result<bool, Error>;

    /// The worktree at `path`'s uncommitted divergence from its last commit —
    /// `git diff HEAD`, staged and unstaged, tracked files only. Empty when
    /// the worktree is clean; untracked files are invisible to `git diff` by
    /// design.
    fn diff_worktree(&self, path: &Utf8Path) -> Result<String, Error>;

    /// Every path in the worktree at `path` that diverges from its last
    /// commit, worktree-relative — tracked edits *and* untracked files.
    ///
    /// [`Self::worktree_dirty`] answers whether anything changed and
    /// [`Self::diff_worktree`] answers how the tracked content changed; this
    /// answers *which files*, which is the question the write-contract audit
    /// asks. Untracked files are included because a file created outside a
    /// contract is exactly the violation worth catching, and `git diff` cannot
    /// see one.
    fn changed_paths(&self, path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, Error>;

    /// How many commits `branch` has that `base` does not, in the repository
    /// at `git_dir` — `git rev-list --count <base>..<branch>`.
    fn commits_ahead(&self, git_dir: &Utf8Path, base: &str, branch: &str) -> Result<u64, Error>;

    /// Whether `branch` has an upstream configured in the repository at
    /// `git_dir` — whether `branch@{upstream}` resolves.
    ///
    /// `Ok(false)` covers both "no upstream" and git refusing for any other
    /// reason, so callers must ensure `git_dir` is a repository before asking.
    fn has_upstream(&self, git_dir: &Utf8Path, branch: &str) -> Result<bool, Error>;

    /// Push `from` to `to` on `remote`, from the repository at `git_dir`.
    ///
    /// `to` is a full ref (`refs/heads/<name>`); `remote` is the URL, so the
    /// push goes exactly where the preview said it would.
    fn push(&self, git_dir: &Utf8Path, remote: &str, from: &str, to: &str) -> Result<(), Error>;

    /// `git -C <worktree> rebase <branch>` — replay the worktree's branch on
    /// top of `<branch>`.
    ///
    /// A conflict (or any refusal) is [`Error::Refused`] carrying git's own
    /// stderr; the caller runs [`Self::abort_rebase`] and reports the repo as
    /// conflicted rather than aborting the batch.
    fn rebase_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), Error>;

    /// `git -C <worktree> rebase --abort` — abandon an in-progress rebase and
    /// restore the branch to where it was before it started.
    fn abort_rebase(&self, worktree: &Utf8Path) -> Result<(), Error>;
}

/// The production [`Git`]: `git2` for reads, the `git` binary for mutations.
///
/// A unit struct with no configuration. Everything that varies — which
/// repository, which branch — is an argument, so one value serves the whole
/// process.
#[derive(Debug, Clone, Copy, Default)]
pub struct System;

impl Git for System {
    fn target_state(&self, path: &Utf8Path) -> Result<TargetState, Error> {
        read::target_state(path)
    }

    fn head_branch(&self, git_dir: &Utf8Path) -> Result<String, Error> {
        read::head_branch(git_dir)
    }

    fn worktree_git_dir(&self, path: &Utf8Path) -> Result<Utf8PathBuf, Error> {
        read::worktree_git_dir(path)
    }

    fn clone_bare(&self, url: &str, dest: &Utf8Path) -> Result<(), Error> {
        exec::clone_bare(url, dest)
    }

    fn ensure_remote_tracking(&self, git_dir: &Utf8Path) -> Result<(), Error> {
        exec::ensure_remote_tracking(git_dir)
    }

    fn add_worktree(&self, git_dir: &Utf8Path, dest: &Utf8Path, branch: &str) -> Result<(), Error> {
        exec::add_worktree(git_dir, dest, branch)
    }

    fn fetch(&self, git_dir: &Utf8Path) -> Result<(), Error> {
        exec::fetch(git_dir)
    }

    fn list_branches(&self, git_dir: &Utf8Path) -> Result<Vec<String>, Error> {
        read::list_branches(git_dir)
    }

    fn create_branch_and_worktree(
        &self,
        git_dir: &Utf8Path,
        branch: &str,
        from_branch: &str,
        dest: &Utf8Path,
    ) -> Result<(), Error> {
        exec::create_branch_and_worktree(git_dir, branch, from_branch, dest)
    }

    fn fetch_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), Error> {
        exec::fetch_branch(worktree, branch)
    }

    fn fast_forward(&self, worktree: &Utf8Path) -> Result<(), Error> {
        exec::fast_forward(worktree)
    }

    fn remove_worktree(&self, git_dir: &Utf8Path, dest: &Utf8Path) -> Result<(), Error> {
        exec::remove_worktree(git_dir, dest)
    }

    fn worktree_dirty(&self, path: &Utf8Path) -> Result<bool, Error> {
        exec::worktree_dirty(path)
    }

    fn diff_worktree(&self, path: &Utf8Path) -> Result<String, Error> {
        exec::diff_worktree(path)
    }

    fn changed_paths(&self, path: &Utf8Path) -> Result<Vec<Utf8PathBuf>, Error> {
        exec::changed_paths(path)
    }

    fn commits_ahead(&self, git_dir: &Utf8Path, base: &str, branch: &str) -> Result<u64, Error> {
        exec::commits_ahead(git_dir, base, branch)
    }

    fn has_upstream(&self, git_dir: &Utf8Path, branch: &str) -> Result<bool, Error> {
        exec::has_upstream(git_dir, branch)
    }

    fn push(&self, git_dir: &Utf8Path, remote: &str, from: &str, to: &str) -> Result<(), Error> {
        exec::push(git_dir, remote, from, to)
    }

    fn rebase_branch(&self, worktree: &Utf8Path, branch: &str) -> Result<(), Error> {
        exec::rebase_branch(worktree, branch)
    }

    fn abort_rebase(&self, worktree: &Utf8Path) -> Result<(), Error> {
        exec::abort_rebase(worktree)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/git/mod.rs"]
mod tests;
