//! What happened last time a worktree's setup script ran.
//!
//! A git worktree shares history with its siblings but not untracked files, so
//! a fresh worktree has no `.env` and no `node_modules`. The per-repo setup
//! script at `.ivar/setups/<repo>.sh` is the answer — and `ivar sync` is what
//! people run after every `git pull`, so it needs to know when *not* to run it.
//! Re-running `pnpm install` on every sync is how a one-second command becomes
//! a two-minute one and people stop running it.
//!
//! # Where this lives, and why it is not a hall path
//!
//! Inside the worktree's **git administrative directory** — for a linked
//! worktree, `<bare>/worktrees/<name>/ivar/setup-receipt.json`. That is the one
//! path in this crate not computed by [`crate::store::layout`], and
//! deliberately: the receipt's correct lifetime is exactly the worktree's, and
//! only git knows where that directory is.
//!
//! `git worktree remove` takes the admin directory with it. So a worktree that
//! is deleted and rebuilt at the same path, on the same branch, starts with no
//! receipt and gets its setup script run again — which is right, because the
//! rebuilt worktree also has no `node_modules`. A receipt filed under
//! `.ivar/` keyed by repo and branch would survive that deletion and skip the
//! run, leaving a worktree that looks set up and is not.
//!
//! # An unreadable receipt means "run it again", never "fail"
//!
//! [`Receipt::read`] returns [`Option`], not [`Result`]. This file is a cache,
//! and every reason it might not read — absent, truncated, hand-edited, written
//! by a newer `ivar` — has the same correct answer: run the script. Setup
//! scripts are required to be idempotent (the sample script says so in its
//! first comment), so running one that did not need to run costs time and
//! nothing else. Refusing to sync because a cache file is corrupt would trade a
//! slow run for a broken one.
//!
//! Writing is a `Result`, because failing to write means the *next* sync
//! re-runs the script, and that is worth a warning.
//!
//! # Reference
//!
//! `packages/bifrost/src/lib/setup-script-receipt.ts`, whose placement argument
//! is the one above and is correct. Its `timestamp` field is not ported —
//! nothing read it there, nothing would read it here, and a clock in a file
//! this module round-trips in tests buys a fixture for no benefit.

use camino::{Utf8Path, Utf8PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Failure, FixAction};
use crate::store::versioned::{self, Policy, Store};

/// The receipt's schema version. First public version, no predecessor — the
/// migration chain is empty, which [`versioned::Store`] treats as "unversioned
/// data is not one of these", exactly as for `ivar.json`.
const CURRENT_VERSION: u32 = 1;

/// The receipt's home inside a worktree's git admin directory. Namespaced under
/// `ivar/` so nothing here collides with git's own bookkeeping.
const RECEIPT_RELATIVE_PATH: [&str; 2] = ["ivar", "setup-receipt.json"];

/// Whether the last run of a worktree's setup script succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    /// The script exited zero.
    Success,
    /// The script exited non-zero, or was killed.
    Failure,
}

/// What happened last time this worktree's setup script ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    version: u32,
    /// The digest of the script's *content*, from [`crate::infra::hash::file`].
    /// Content and not mtime: a `git pull` that rewrites a script byte-identical
    /// must not trigger a re-run, and one that changes a single line must.
    fingerprint: String,
    outcome: RunOutcome,
    /// The exit code, or `None` when a signal killed the script. Recorded so
    /// `doctor` can say *how* it failed without re-running it.
    exit_code: Option<i32>,
}

impl Receipt {
    /// A receipt for a run of a script with digest `fingerprint` that exited
    /// `exit_code`.
    ///
    /// The outcome is derived rather than passed: "exited zero" and "succeeded"
    /// are the same fact, and letting a caller supply both invites them to
    /// disagree.
    #[must_use]
    pub fn of_run(fingerprint: impl Into<String>, exit_code: Option<i32>) -> Self {
        Self {
            version: CURRENT_VERSION,
            fingerprint: fingerprint.into(),
            outcome: if exit_code == Some(0) {
                RunOutcome::Success
            } else {
                RunOutcome::Failure
            },
            exit_code,
        }
    }

    /// Whether the run this receipt describes succeeded.
    ///
    /// The one accessor: [`Self::should_run`] is what callers actually ask, and
    /// `fingerprint` and `exit_code` have no reader outside this module — they
    /// exist to be serialised, so `doctor` can read a receipt off disk later
    /// without this module having guessed at its questions today.
    #[must_use]
    pub fn outcome(&self) -> RunOutcome {
        self.outcome
    }

    /// The receipt in `git_dir`, if there is a readable, current one.
    ///
    /// Total by design — see the module doc comment. Every failure mode reduces
    /// to `None`, which means "run the script", which is always safe.
    #[must_use]
    pub fn read(git_dir: &Utf8Path) -> Option<Self> {
        Self::open(git_dir).read().ok().flatten()
    }

    /// Write `receipt` into `git_dir`, creating the `ivar/` namespace under it.
    pub fn write(git_dir: &Utf8Path, receipt: &Self) -> Result<(), Error> {
        let path = Self::path_in(git_dir);

        // The canonical writer writes a file, not a tree. `ivar/` under a git
        // admin directory is ours to create, and it will not exist the first
        // time a worktree runs its setup script.
        if let Some(parent) = path.parent() {
            crate::infra::fs::ensure_dir(parent).map_err(|source| Error::Namespace {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        Self::open(git_dir)
            .write(receipt)
            .map_err(|source| Error::Write {
                path,
                source: Box::new(source),
            })
    }

    /// Whether the setup script should run.
    ///
    /// Four reasons to run, in the order they are checked: the caller asked
    /// (`force`), there is no readable receipt, the script's content has
    /// changed since the last run, or the last run failed.
    ///
    /// That last one is not an optimisation gap — a failed setup that recorded
    /// "done" would leave every later sync silently skipping the repair the
    /// user is waiting for.
    #[must_use]
    pub fn should_run(existing: Option<&Self>, fingerprint: &str, force: bool) -> bool {
        if force {
            return true;
        }
        let Some(receipt) = existing else {
            return true;
        };
        receipt.fingerprint != fingerprint || receipt.outcome == RunOutcome::Failure
    }

    /// Where the receipt sits inside a worktree's git admin directory.
    #[must_use]
    pub fn path_in(git_dir: &Utf8Path) -> Utf8PathBuf {
        RECEIPT_RELATIVE_PATH
            .iter()
            .fold(git_dir.to_path_buf(), |path, segment| path.join(segment))
    }

    /// The [`Store`] this module reads and writes through. `Policy::Local`: the
    /// file is not committed, nobody reviews it, and a migration would be
    /// nobody's business but this module's.
    fn open(git_dir: &Utf8Path) -> Store<Self> {
        Store::new(
            Self::path_in(git_dir),
            Vec::new(),
            CURRENT_VERSION,
            Policy::Local,
        )
    }
}

/// The one thing that can go wrong here: the receipt would not land.
///
/// Reading has no error — see the module doc comment. The two variants split
/// the write in half only because they fail at different layers; they carry the
/// same code and the same fix action, because from where the user sits they are
/// one problem with one answer.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The `ivar/` namespace directory inside the git admin dir could not be
    /// created.
    #[error("could not create `{path}` to record the setup-script result")]
    Namespace {
        path: Utf8PathBuf,
        #[source]
        source: crate::infra::fs::Error,
    },

    /// The receipt file itself could not be written.
    #[error("could not record the setup-script result at `{path}`")]
    Write {
        path: Utf8PathBuf,
        // Boxed to keep this error small: `versioned::Error` carries paths and
        // wrapped `serde_json` errors, and this type is returned by value from
        // a function called once per repo.
        #[source]
        source: Box<versioned::Error>,
    },
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        // The `#[error(...)]` attribute is the single source of the sentence.
        let what = error.to_string();
        let actual = match &error {
            Error::Namespace { source, .. } => source.to_string(),
            Error::Write { source, .. } => source.to_string(),
        };

        Failure::failed("setup.receipt_write_failed", what)
            .expected("a writable git administrative directory")
            .actual(actual)
            .fix(
                FixAction::safe(
                    "setup.rerun_sync",
                    "Run `ivar sync` again — the setup script will simply run once more.",
                )
                .command("ivar sync"),
            )
    }
}

#[cfg(test)]
#[path = "../../tests/unit/store/setup_receipt.rs"]
mod tests;
