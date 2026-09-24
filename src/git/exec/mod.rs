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
mod remote;
mod status;
mod worktree;

pub(crate) use self::branch::{
    abort_rebase, commit_messages, commits_ahead, create_branch, delete_branch, fast_forward,
    fast_forward_to, merge_no_ff, rebase_branch, rename_branch, reset_hard, squash_merge,
};
pub(crate) use self::clone::{
    clone_bare, clone_bare_prefixed, ensure_remote_tracking, REF_PREFIX_KEY,
};
pub(crate) use self::remote::{delete_remote_branch, fetch, publish_remote_branch, push, remote_branch_tip};
pub(crate) use self::status::{
    changed_paths, commit_patch_id, diff_patch_id, diff_worktree, head_commit,
    paths_committed_since, worktree_dirty,
};
pub(crate) use self::worktree::{
    add_detached_worktree, add_worktree, create_branch_and_worktree, fetch_branch, list_worktrees,
    move_worktree, parse_worktree_list, prune_worktrees, remove_worktree,
};

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

#[cfg(test)]
#[path = "../../../tests/unit/git/exec.rs"]
mod tests;
