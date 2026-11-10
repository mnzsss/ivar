//! `ivar hall sync` — commit everything pending in the hall and push it.
//!
//! The hall is a git repository that `ivar` and the agents it schedules keep
//! writing to: memory pages, reports, skill state. Nothing else commits those
//! files, so they pile up in the working tree and never reach another machine.
//! [`sync_hall`] is the one path that records them and sends them upstream.
//!
//! # What one run does
//!
//! In this order:
//!
//! 1. **The operation's commit**, from the paths an operation says it wrote and
//!    the message that describes it — and only those paths. Skipped when there
//!    are none.
//! 2. **The general commit**, `chore(hall): Sync pending hall changes`: all the
//!    rest — modified, new or deleted, staged or not — minus what `.gitignore`
//!    ignores.
//! 3. **The push** of the current branch to its upstream, whenever the branch
//!    is ahead of it, including commits an earlier failed attempt left.
//!
//! Two commits rather than one so that the operation's message stays true to
//! what it did, and so that a user's file the commit hook refuses cannot block
//! the operation's own commit.
//!
//! # Failure never fails the caller
//!
//! A hook that refuses, a network that is down, a remote that wants a password
//! — none is an error here. Each is a [`Warning`] with a stable code
//! (`hall.commit_failed`, `hall.push_failed`, …) and the commits stay local.
//! The operation that called this already did what it was asked; a sync that
//! cannot finish must not take that away. No commit uses `--no-verify`, and
//! no push is forced or allowed to wait for input.
//!
//! # A push the remote refuses
//!
//! When the remote has commits the hall lacks, the hall runs `git pull
//! --rebase` and tries once more. If the rebase stops on a conflict,
//! [`super::resolve::resolve_conflicts`] applies fixed rules; if it cannot settle every path,
//! the rebase is aborted — the hall is left exactly as it was before the pull —
//! and the push is reported as failed.
//!
//! # One run at a time
//!
//! The whole run holds `.ivar/hall-sync.lock`. A second run that cannot take it
//! returns at once, without an error: the first is already doing the work.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::action::Ctx;
use crate::error::{Outcome, Report, Warning, WriteHuman};
use crate::git::{self, CommitOutcome, Git, RepoState, SkipReason};
use crate::infra::fs;
use crate::store::layout::Layout;

use super::{discover_hall, read_manifest};

mod push;

/// The message of the commit that holds everything the operation's own commit
/// did not.
pub const GENERAL_MESSAGE: &str = "chore(hall): Sync pending hall changes";

/// A rebase that stops on more commits than this is not being resolved, it is
/// being repeated. The hall's history is a handful of commits per machine.
const MAX_REBASE_STOPS: usize = 64;

/// Who is asking, which decides whether the hall's `auto_commit` and
/// `auto_push` switches apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// A person ran `ivar hall sync`. The switches are ignored: they stop
    /// `ivar` acting on its own, not a person asking for it.
    Manual,
    /// An operation `ivar` ran by itself. `auto_commit: false` stops it
    /// committing and pushing; `auto_push: false` stops it pushing.
    Automatic,
}

/// What an operation wrote, and how to describe it in the commit.
#[derive(Debug, Clone, Copy)]
pub struct OperationCommit<'a> {
    /// Hall-relative paths the operation wrote. An absolute path under the hall
    /// is made relative; one outside it, or one that leaves it through `..`,
    /// is dropped. Empty means no operation commit.
    pub paths: &'a [Utf8PathBuf],
    /// The operation commit's message.
    pub message: &'a str,
}

/// One call to [`sync_hall`].
#[derive(Debug, Clone, Copy)]
pub struct SyncRequest<'a> {
    pub trigger: Trigger,
    /// The operation's commit, when there is one.
    pub operation: Option<OperationCommit<'a>>,
}

impl SyncRequest<'_> {
    /// `ivar hall sync`: no operation commit, and the `auto_*` switches ignored.
    #[must_use]
    pub fn manual() -> Self {
        Self {
            trigger: Trigger::Manual,
            operation: None,
        }
    }
}

/// What [`sync_hall`] did.
#[derive(Debug, Clone, Serialize)]
pub struct HallSync {
    /// The hall root.
    pub root: Utf8PathBuf,
    /// The operation's own commit was recorded.
    pub operation_committed: bool,
    /// The general commit was recorded.
    pub general_committed: bool,
    /// Commits were pushed to the upstream.
    pub pushed: bool,
    /// Another sync held the lock, so this one did nothing.
    pub busy: bool,
    /// What needs attention. Left out of the JSON body: the report envelope
    /// carries warnings itself, and a caller that folds this into its own
    /// report takes them from here.
    #[serde(skip)]
    pub warnings: Vec<Warning>,
}

impl WriteHuman for HallSync {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.busy {
            return writeln!(w, "Hall at {}: another sync is running.", self.root);
        }
        let mut done = Vec::new();
        if self.operation_committed {
            done.push("committed the operation's changes");
        }
        if self.general_committed {
            done.push("committed the pending changes");
        }
        if self.pushed {
            done.push("pushed");
        }
        if done.is_empty() {
            writeln!(w, "Hall at {}: nothing to sync.", self.root)
        } else {
            writeln!(w, "Hall at {}: {}.", self.root, done.join(", "))
        }
    }
}

/// `ivar hall sync` — commit everything pending in the hall and push it.
pub fn sync(ctx: &Ctx) -> Outcome<HallSync> {
    let layout = discover_hall(ctx)?;
    let mut outcome = sync_hall(&layout, &git::System, &SyncRequest::manual());
    let warnings = std::mem::take(&mut outcome.warnings);
    Ok(if warnings.is_empty() {
        Report::new(outcome)
    } else {
        Report::with_warnings(outcome, warnings)
    })
}

/// Where the hall's repository stands against its upstream, and whether an
/// operation is half-done in it: the branch, how far ahead and behind, whether
/// the two diverged.
///
/// Read-only, and never reaches the network, so ahead and behind are as fresh
/// as the last fetch. This is the question the sync asks before it pushes, and
/// the one `ivar doctor` asks to say a hall's sync is stuck.
///
/// # Errors
///
/// Returns the git error if the repository exists but cannot be read.
pub fn sync_state(layout: &Layout, git: &impl Git) -> Result<RepoState, git::Error> {
    git.repo_state(layout.root())
}

/// Commit what is pending in the hall and push it. See the module docs for the
/// order, the warnings and what never happens.
///
/// Infallible by design: whatever goes wrong — git, the network, an unreadable
/// `ivar.json` for an [`Trigger::Automatic`] run, a lock file that cannot be
/// created — is a [`Warning`] in the result. The operation that called this has
/// already done what it was asked, and a sync must not take that away.
pub fn sync_hall(layout: &Layout, git: &impl Git, request: &SyncRequest<'_>) -> HallSync {
    let mut run = Run {
        layout,
        git,
        subject: layout.root().to_string(),
        out: HallSync {
            root: layout.root().to_path_buf(),
            operation_committed: false,
            general_committed: false,
            pushed: false,
            busy: false,
            warnings: Vec::new(),
        },
    };

    let push = match request.trigger {
        Trigger::Manual => true,
        Trigger::Automatic => match read_manifest(layout) {
            Ok(manifest) if !manifest.auto_commit() => return run.out,
            Ok(manifest) => manifest.auto_push(),
            Err(failure) => {
                run.warn(
                    "hall.commit_failed",
                    format!(
                        "could not read ivar.json, so nothing was committed or pushed: {}",
                        failure.what
                    ),
                );
                return run.out;
            }
        },
    };

    let _lock = match fs::try_lock_exclusive(&layout.hall_sync_lock()) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            run.out.busy = true;
            return run.out;
        }
        Err(error) => {
            run.warn(
                "hall.commit_failed",
                format!("could not take the hall sync lock: {error}"),
            );
            return run.out;
        }
    };

    run.sync(request, push);
    run.out
}

/// One sync, with the pieces every step shares.
struct Run<'a, G: Git> {
    layout: &'a Layout,
    git: &'a G,
    /// The warnings' subject: which hall.
    subject: String,
    out: HallSync,
}

impl<G: Git> Run<'_, G> {
    fn root(&self) -> &Utf8Path {
        self.layout.root()
    }

    fn warn(&mut self, code: &'static str, what: String) {
        self.out
            .warnings
            .push(Warning::new(code, self.subject.clone(), what));
    }

    fn sync(&mut self, request: &SyncRequest<'_>, push: bool) {
        let state = match self.git.repo_state(self.root()) {
            Ok(state) => state,
            Err(error) => {
                self.warn(
                    "hall.commit_failed",
                    format!("could not read the hall's git state: {}", detail(&error)),
                );
                return;
            }
        };
        if let Some(reason) = state.skip_reason() {
            self.skipped(reason);
            return;
        }

        // The operation's commit going wrong must not stop its siblings from
        // being pushed, but it does stop the general commit: that one would
        // sweep the very files the operation's commit was refused for under a
        // message that says nothing about them.
        let mut general = true;
        if let Some(operation) = request.operation {
            let paths = self.relative(operation.paths);
            if !paths.is_empty() {
                let result = self
                    .git
                    .commit_paths(self.layout.root(), &paths, operation.message);
                match self.settle("the operation's changes", result) {
                    Step::Committed => self.out.operation_committed = true,
                    Step::Nothing => {}
                    Step::Failed => general = false,
                    Step::Skipped => return,
                }
            }
        }
        if general && !self.commit_pending() {
            return;
        }
        if push {
            self.push();
        }
    }

    /// Hall-relative form of `paths`: an absolute path under the hall is
    /// stripped of the hall's root, one outside it is dropped.
    fn relative(&self, paths: &[Utf8PathBuf]) -> Vec<Utf8PathBuf> {
        paths
            .iter()
            .filter_map(|path| {
                if path.is_absolute() {
                    path.strip_prefix(self.root())
                        .ok()
                        .map(Utf8Path::to_path_buf)
                } else {
                    Some(path.clone())
                }
            })
            .collect()
    }

    /// The general commit. `false` when the hall turned out not to be in a state
    /// to commit to, after it was checked — a warning has been recorded and the
    /// run should end.
    fn commit_pending(&mut self) -> bool {
        let result = self.git.commit_all(self.layout.root(), GENERAL_MESSAGE);
        match self.settle("the pending hall changes", result) {
            Step::Committed => {
                self.out.general_committed = true;
                true
            }
            Step::Nothing | Step::Failed => true,
            Step::Skipped => false,
        }
    }

    /// Turn one commit's result into a [`Step`], recording the warning it
    /// earned.
    fn settle(&mut self, what: &str, result: Result<CommitOutcome, git::Error>) -> Step {
        match result {
            Ok(CommitOutcome::Committed) => Step::Committed,
            Ok(CommitOutcome::Nothing) => Step::Nothing,
            Ok(CommitOutcome::Skipped(reason)) => {
                self.skipped(reason);
                Step::Skipped
            }
            Err(error) => {
                self.warn(
                    "hall.commit_failed",
                    format!("could not commit {what}: {}", detail(&error)),
                );
                Step::Failed
            }
        }
    }

    fn skipped(&mut self, reason: SkipReason) {
        self.warn(
            "hall.commit_skipped",
            format!("nothing was committed or pushed: {reason}"),
        );
    }
}

/// How one commit ended, for the sync's control flow.
enum Step {
    Committed,
    Nothing,
    /// Refused or failed; a warning has been recorded and the run goes on.
    Failed,
    /// The hall is not in a state to commit to; the run ends.
    Skipped,
}

/// git's own sentence for `error`, without the command line in front of it.
fn detail(error: &git::Error) -> String {
    match error {
        git::Error::Refused { detail, .. } => detail.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/hall/sync.rs"]
mod tests;
