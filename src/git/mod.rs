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
//! # What this slice does not do yet
//!
//! [`Git::clone_bare`] clones the URL it is given, once. The predecessor tried
//! SSH first and fell back to HTTPS authenticated through the `gh` credential
//! helper, which is a real convenience for private GitHub repos. That fallback
//! needs `infra::github` and `git::credential`, neither of which exists yet —
//! it is deferred with the cost named, not forgotten. A private repo that the
//! user's own git is not already configured for fails here with git's own
//! message, which is at least the message they can act on.

// `pub(crate)`, so the boundary above is a fact and not a promise: nothing
// outside this module — in this crate or out of it — can name
// `git::exec::clone_bare` and freeze which backend serves an operation.
pub(crate) mod exec;
pub(crate) mod read;

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::{Failure, FixAction};
use crate::infra::{fs, proc};

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
}

/// Everything that can go wrong talking to git.
///
/// The two shapes are deliberate and mean different things to a caller.
/// [`Self::Refused`] is git answering — it ran, understood the request, and
/// declined, and its own stderr is the best sentence anyone has about why.
/// The rest are git never getting that far.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The `git` binary could not be started at all.
    #[error(transparent)]
    Spawn(#[from] proc::Error),

    /// The filesystem would not answer a question this module had to ask it.
    #[error(transparent)]
    Fs(#[from] fs::Error),

    /// git ran and exited non-zero. `detail` is its own stderr.
    #[error("`{command}` failed: {detail}")]
    Refused {
        /// The invocation, as `proc::Command::display` renders it.
        command: String,
        /// git's own diagnostic — the sentence a user can search for.
        detail: String,
    },

    /// A path was expected to be a git repository and was not, or could not be
    /// opened as one.
    #[error("`{path}` is not a git repository ({detail})")]
    NotARepository {
        path: Utf8PathBuf,
        /// libgit2's own description.
        detail: String,
    },

    /// The repository's `HEAD` is not a symbolic ref to a branch — it is
    /// detached, or points somewhere this tool has no name for.
    #[error("`{path}` has no branch checked out (HEAD is detached)")]
    DetachedHead { path: Utf8PathBuf },

    /// libgit2 handed back a path that is not valid UTF-8.
    #[error("git reported a path that is not valid UTF-8: {display}")]
    NotUtf8 {
        /// Lossy rendering, for the human message only.
        display: String,
    },
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        // The `#[error(...)]` attribute is the single source of the sentence.
        let what = error.to_string();

        match error {
            // Both already carry their own code and fix action.
            Error::Spawn(source) => source.into(),
            Error::Fs(source) => source.into(),

            // Failed, not Blocked: git got as far as trying. A clone that dies
            // halfway leaves a partial directory behind, and telling the caller
            // "nothing happened" would be a lie they would act on.
            Error::Refused { detail, .. } => Failure::failed("git.command_failed", what)
                .expected("git to complete the operation")
                .actual(detail)
                .fix(FixAction::safe(
                    "git.read_the_error",
                    "Run the command shown above by hand — git's own message names what it needs.",
                )),

            Error::NotARepository { path, detail } => {
                Failure::blocked("git.not_a_repository", what)
                    .expected(format!("`{path}` to be a git repository"))
                    .actual(detail)
                    .fix(
                        FixAction::safe(
                            "git.resync_hall",
                            "Run `ivar sync` to rebuild what is missing under `.ivar/`.",
                        )
                        .command("ivar sync"),
                    )
            }

            Error::DetachedHead { path } => Failure::blocked("git.detached_head", what)
                .expected("HEAD to name a branch")
                .actual(format!("`{path}` has a detached HEAD"))
                .fix(FixAction::unsafe_(
                    "git.checkout_a_branch",
                    "Check out a branch in that worktree — ivar cannot pick one for you.",
                )),

            Error::NotUtf8 { display } => Failure::blocked("git.path_not_utf8", what)
                .expected("a path that is valid UTF-8")
                .actual(display)
                .fix(FixAction::unsafe_(
                    "git.rename_to_utf8",
                    "Rename the offending path to valid UTF-8.",
                )),
        }
    }
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
    use crate::error::Status;
    use crate::test_support::{empty_repo, git, seeded_repo, utf8_temp_dir};

    // -- System routes each operation to a backend that answers ---------------

    #[test]
    fn system_clones_a_bare_repository_and_reads_its_head_branch() {
        let (_guard, dir) = utf8_temp_dir();
        let origin = seeded_repo(&dir.join("origin"), "main");
        let bare = dir.join("api.bare");

        System.clone_bare(origin.as_str(), &bare).unwrap();

        assert_eq!(System.target_state(&bare).unwrap(), TargetState::Repository);
        assert_eq!(System.head_branch(&bare).unwrap(), "main");
    }

    #[test]
    fn system_adds_a_worktree_on_an_existing_branch() {
        let (_guard, dir) = utf8_temp_dir();
        let origin = seeded_repo(&dir.join("origin"), "main");
        let bare = dir.join("api.bare");
        System.clone_bare(origin.as_str(), &bare).unwrap();

        let worktree = dir.join("api-main");
        System.add_worktree(&bare, &worktree, "main").unwrap();

        assert!(worktree.join("README.md").is_file());
        assert_eq!(
            System.target_state(&worktree).unwrap(),
            TargetState::Repository
        );
    }

    /// The admin dir of a linked worktree is `<bare>/worktrees/<name>/`, not
    /// the bare repository. Bookkeeping written there dies with the worktree,
    /// which is the property `sync`'s setup-script receipt depends on.
    #[test]
    fn a_worktrees_git_dir_is_its_own_and_not_the_bare_repository() {
        let (_guard, dir) = utf8_temp_dir();
        let origin = seeded_repo(&dir.join("origin"), "main");
        let bare = dir.join("api.bare");
        System.clone_bare(origin.as_str(), &bare).unwrap();
        let worktree = dir.join("api-main");
        System.add_worktree(&bare, &worktree, "main").unwrap();

        let git_dir = System.worktree_git_dir(&worktree).unwrap();

        assert!(
            git_dir.as_str().contains("worktrees"),
            "expected a linked-worktree admin dir, got {git_dir}"
        );
        assert_ne!(git_dir, bare);
    }

    // -- fetch -----------------------------------------------------------------

    /// Fetching updates the bare clone with commits the origin gained since
    /// the clone. A bare clone has no worktree, so the update is only visible
    /// to git — this asserts the fetch reached the object database.
    #[test]
    fn fetch_pulls_new_commits_from_the_origin_into_the_bare_clone() {
        let (_guard, dir) = utf8_temp_dir();
        let origin = seeded_repo(&dir.join("origin"), "main");
        let bare = dir.join("api.bare");
        System.clone_bare(origin.as_str(), &bare).unwrap();

        std::fs::write(origin.join("CHANGELOG.md"), "v1\n").unwrap();
        git(&origin, &["add", "CHANGELOG.md"]);
        git(&origin, &["commit", "-m", "v1"]);

        System.fetch(&bare).unwrap();

        // A bare clone has no `refs/remotes/*` — it copies `refs/heads/*`
        // straight over, so the fetched branch is `main`, not `origin/main`.
        let status = std::process::Command::new("git")
            .args(["--git-dir", bare.as_str(), "rev-parse", "main"])
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "main did not update: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    /// An up-to-date clone still fetches — `--quiet` makes it a no-op, but
    /// the operation succeeds. The caller reports "up to date" through the
    /// `Ok`, not through an error.
    #[test]
    fn fetch_succeeds_when_there_is_nothing_to_fetch() {
        let (_guard, dir) = utf8_temp_dir();
        let origin = seeded_repo(&dir.join("origin"), "main");
        let bare = dir.join("api.bare");
        System.clone_bare(origin.as_str(), &bare).unwrap();

        System.fetch(&bare).unwrap();
    }

    // -- list_branches ----------------------------------------------------------

    #[test]
    fn list_branches_returns_local_branch_names_without_the_prefix() {
        let (_guard, dir) = utf8_temp_dir();
        let origin = seeded_repo(&dir.join("origin"), "main");
        git(&origin, &["branch", "dev"]);
        let bare = dir.join("api.bare");
        System.clone_bare(origin.as_str(), &bare).unwrap();

        let branches = System.list_branches(&bare).unwrap();

        // Sorted lexically, whatever order git2 happened to iterate in.
        assert_eq!(branches, vec!["dev".to_owned(), "main".to_owned()]);
    }

    /// An unborn default branch has no ref for `list_branches` to find —
    /// that is not an error, it is the truth about a repo created moments ago.
    #[test]
    fn list_branches_answers_empty_for_a_repository_with_no_commits() {
        let (_guard, dir) = utf8_temp_dir();
        let origin = empty_repo(&dir.join("origin"), "main");

        let branches = System.list_branches(&origin).unwrap();

        assert!(branches.is_empty());
    }

    #[test]
    fn list_branches_on_something_that_is_not_a_repository_says_so() {
        let (_guard, dir) = utf8_temp_dir();

        let error = System.list_branches(&dir).expect_err("not a repository");

        assert!(matches!(error, Error::NotARepository { .. }));
    }

    /// `target_state` answers about the path itself. If it walked up, every
    /// empty directory inside a hall that is itself a git repo would claim to
    /// be a materialised worktree.
    #[test]
    fn target_state_does_not_walk_up_to_a_parent_repository() {
        let (_guard, dir) = utf8_temp_dir();
        seeded_repo(&dir, "main");
        let child = dir.join("not-a-repo");
        std::fs::create_dir_all(&child).unwrap();

        assert_eq!(System.target_state(&dir).unwrap(), TargetState::Repository);
        assert_eq!(System.target_state(&child).unwrap(), TargetState::Occupied);
    }

    #[test]
    fn cloning_a_url_that_is_not_a_repository_is_refused_with_gits_own_message() {
        let (_guard, dir) = utf8_temp_dir();

        let error = System
            .clone_bare(dir.join("nowhere").as_str(), &dir.join("dest"))
            .expect_err("nothing to clone");

        assert!(matches!(error, Error::Refused { .. }));
    }

    #[test]
    fn head_branch_on_a_plain_directory_is_not_a_repository() {
        let (_guard, dir) = utf8_temp_dir();

        let error = System.head_branch(&dir).expect_err("not a repository");

        assert!(matches!(error, Error::NotARepository { .. }));
    }

    // -- Error -> Failure ------------------------------------------------------

    /// A refused command is `Failed`, not `Blocked`: git got as far as trying,
    /// and a half-finished clone leaves a directory behind. Claiming nothing
    /// happened would be a lie the caller acts on.
    #[test]
    fn a_refused_command_is_a_failed_failure_carrying_gits_own_diagnostic() {
        let failure: Failure = Error::Refused {
            command: "git clone --bare url dest".to_owned(),
            detail: "repository 'url' does not exist".to_owned(),
        }
        .into();

        assert_eq!(failure.status, Status::Failed);
        assert_eq!(failure.code, "git.command_failed");
        assert_eq!(
            failure.actual.as_deref(),
            Some("repository 'url' does not exist")
        );
    }

    #[test]
    fn a_missing_repository_is_blocked_and_points_at_sync() {
        let failure: Failure = Error::NotARepository {
            path: Utf8PathBuf::from("/hall/.ivar/repos/api/.bare"),
            detail: "could not find repository".to_owned(),
        }
        .into();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "git.not_a_repository");
        assert_eq!(failure.fix_actions[0].command.as_deref(), Some("ivar sync"));
    }

    #[test]
    fn a_detached_head_needs_a_human() {
        let failure: Failure = Error::DetachedHead {
            path: Utf8PathBuf::from("/hall/.ivar/repos/api/main"),
        }
        .into();

        assert_eq!(failure.code, "git.detached_head");
        assert!(!failure.fix_actions[0].safe);
    }

    #[test]
    fn a_spawn_error_delegates_its_failure_conversion() {
        let spawn = proc::capture(&proc::Command::new("ivar-no-such-program-exists-anywhere"))
            .expect_err("no binary");

        let failure: Failure = Error::Spawn(spawn).into();

        assert_eq!(failure.code, "proc.spawn_failed");
    }
}
