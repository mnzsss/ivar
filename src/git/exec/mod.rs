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

mod branch;
mod clone;
mod commit;
mod conflict;
mod remote;
mod status;
mod worktree;

pub(crate) use self::branch::{
    abort_rebase, commit_messages, commits_ahead, continue_rebase, create_branch, delete_branch,
    fast_forward, fast_forward_to, first_parent_reaches, merge_no_ff, rebase_branch, rename_branch,
    reset_hard, skip_rebase, squash_merge,
};
pub(crate) use self::clone::{clone_bare, clone_bare_prefixed, ensure_remote_tracking};
pub(crate) use self::commit::{
    commit_all, commit_paths, stage_paths, staged_changes, tracked_changes,
};
pub(crate) use self::conflict::{conflicts, take_side};
pub(crate) use self::remote::{
    delete_remote_branch, fetch, publish_remote_branch, pull_rebase, push, push_upstream,
    remote_branch_tip,
};
pub(crate) use self::status::{
    changed_paths, commit_patch_id, diff_patch_id, diff_worktree, head_commit,
    paths_changed_since_fork, paths_committed_since, worktree_dirty,
};
pub(crate) use self::worktree::{
    add_detached_worktree, add_worktree, create_branch_and_worktree, fetch_branch, list_worktrees,
    move_worktree, prune_worktrees, remove_worktree,
};

#[cfg(test)]
pub(crate) use self::{clone::REF_PREFIX_KEY, worktree::parse_worktree_list};

use crate::infra::proc;

use super::Error;

/// A `git` invocation with this module's fail-fast environment already applied.
///
/// Every command in this module starts here, so the discipline described in the
/// module doc comment cannot be forgotten at one call site. `protect` shares it
/// for the same reason: a second `Command::new("git")` anywhere in `git/` is a
/// second chance to forget the environment.
pub(super) fn git() -> proc::Command {
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
pub(super) fn run(command: &proc::Command) -> Result<String, Error> {
    let output = proc::capture(command)?;
    if output.success() {
        return Ok(output.stdout);
    }
    Err(Error::Refused {
        command: command.display(),
        detail: output.diagnostic(),
    })
}

/// [`run`] with a deadline, for the calls that reach a remote.
///
/// Both streams go to one scratch file, which is what lets a child that is
/// still running be killed at the deadline without a reader thread holding its
/// pipes open. The scratch file holds git's own words, so a failure carries
/// them as its `detail` exactly as [`run`] would — stderr and stdout interleaved
/// in the order git wrote them.
///
/// Only the child is killed on timeout, not its process group: a group kill
/// would take a user's long-lived `ssh` multiplexing master with it.
pub(super) fn run_with_deadline(
    command: &proc::Command,
    deadline: std::time::Duration,
) -> Result<String, Error> {
    let refused = |detail: String| Error::Refused {
        command: command.display(),
        detail,
    };
    let log = tempfile::NamedTempFile::new()
        .map_err(|source| refused(format!("could not create a scratch log: {source}")))?;
    let path = camino::Utf8Path::from_path(log.path())
        .ok_or_else(|| refused("the scratch log's path is not valid UTF-8".to_owned()))?;
    let finished = proc::to_log_with_deadline(command, path, deadline, &|_| false)?;
    let text = std::fs::read(log.path())
        .map(|bytes| String::from_utf8_lossy(&bytes).trim_end().to_owned())
        .unwrap_or_default();
    match finished {
        proc::Finished::Exited(Some(0)) => Ok(text),
        proc::Finished::Exited(code) => Err(refused(if text.is_empty() {
            code.map_or_else(
                || "killed by a signal".to_owned(),
                |code| format!("exited {code} with no output"),
            )
        } else {
            text
        })),
        proc::Finished::TimedOut | proc::Finished::Stopped => Err(refused(format!(
            "timed out after {}s{}",
            deadline.as_secs(),
            if text.is_empty() {
                String::new()
            } else {
                format!(": {text}")
            }
        ))),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/git/exec.rs"]
mod tests;
