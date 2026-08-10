//! Everything that can go wrong talking to git, and how each becomes a
//! user-facing [`Failure`].
//!
//! [`Error`] is this module's own error type; its `Failure` conversion is the
//! single place a git problem acquires a code, an expected/actual pair, and a
//! fix action. The `Git` trait and its [`System`](super::System)
//! implementation stay in `mod.rs` — this file owns only the errors.

use camino::Utf8PathBuf;

use crate::error::{Failure, FixAction};
use crate::infra::{fs, proc};

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
