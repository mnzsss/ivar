//! Per-session execution guard materialisation: the artefact that arbitrates
//! an executor's *file-editing tool calls* against its workstream's write
//! contract — every write it can see, which is not every write. See "What the
//! guard cannot see" below, and the post-run audit that covers the rest.
//!
//! [`materialise`] writes one of two things into a session's view dir,
//! depending on [`Provider`]:
//!
//! - Claude Code: `<view_dir>/.claude/hooks/ivar-execution-guard.sh`,
//!   registered as a `PreToolUse` hook in `<view_dir>/.claude/settings.json`.
//! - OpenCode: `<view_dir>/.opencode/plugins/ivar-execution-guard.ts`,
//!   intercepting `tool.execute.before`.
//!
//! Both shell back into `ivar feature execute guard-check`
//! ([`crate::action::execute::guard_check`], read but not modified here) with
//! the feature, the session id, and the attempted path, and refuse the write
//! unless that command answers with an explicit `allowed: true`.
//!
//! # The default is deny
//!
//! Every branch a generated artefact can take — a tool call it cannot pull a
//! path out of, a `guard-check` invocation that cannot even run, a non-zero
//! exit, output that fails to parse, an `allowed` field that is present but
//! not `true` — ends in a refusal. Nothing here ever allows by omission; see
//! the safeguard in the plan and `guard_check`'s own module doc, which states
//! the same rule for the command side of this pair.
//!
//! One consequence of that rule is easy to miss by reading `guard_check.rs`
//! only for its exit behaviour: **every** answer it computes, including a
//! denial, returns cleanly — an unknown session, a path outside the
//! contract, and a missing board all render as `Ok(Report { allowed: false,
//! .. })`, not an `Err`. The process exit code is therefore `0` for an
//! allowed *and* a denied answer alike, and only a genuinely missing
//! argument (a caller bug, not a workstream's business) makes it non-zero.
//! A generated script that only checked `$?` would allow every write
//! `guard-check` managed to compute an answer for, denying solely on a
//! crash — the opposite of this module's contract. Both generated artefacts
//! therefore always inspect the `allowed` field of `guard-check`'s `--json`
//! output, and treat a non-zero exit as one more reason that field is
//! unavailable, not as the primary signal.
//!
//! # What the guard cannot see
//!
//! The matcher lists the tools whose call carries a path: `Write`, `Edit`,
//! `MultiEdit`, `NotebookEdit`. `Bash` is deliberately not among them, and
//! that is a hole, not an oversight — but not one this layer can close.
//!
//! A `Bash` call carries a *command*, not a path. Deciding which files
//! `python3 - <<EOF` writes means deciding what the program does; a hook that
//! guessed would deny `cargo test | tee log` and allow the heredoc that
//! rewrites the repo. Denying `Bash` outright is the only rule this module's
//! own default-deny discipline can state honestly, and it leaves an executor
//! unable to run the tests it was launched to make pass.
//!
//! This mattered exactly once, expensively: with the harness launched without
//! a permission mode (see [`super::Harness::execute_command`]), `Write` and
//! `Edit` were denied by the *harness* before the guard was ever consulted,
//! and the one workstream that delivered anything did so through fifty-two
//! `python3` heredocs — every one of them past this guard, none of them
//! refused. The guard covered precisely the tools that could not run, and not
//! the one that could.
//!
//! So the hole is closed a layer down instead, where intentions have become
//! effects: `tick` audits the feature's worktrees against the write contract
//! after each run (see `action::execute::tick::launch`'s `audit_run`). That
//! audit detects rather than prevents — the bytes are on disk by the time it
//! looks — but it cannot be talked past, because it reads the filesystem
//! rather than the agent's stated intent.
//!
//! `Bash` is also how an agent reaches *git*, and that took a second pass to
//! close. The audit's first oracle was `git status`, which reports divergence
//! from the current commit — so a run that committed emptied it, and the audit
//! read a run that had written anything at all as a run that had written
//! nothing. Committing is not an exotic way past the guard, either: it is the
//! expected end state, since `feature deliver` counts a dirty worktree as a
//! blocker. The audit now diffs the worktree against the commit it was on when
//! the run started, so what the run committed is as visible as what it left
//! uncommitted.
//!
//! # Why the hall path is baked in, not discovered
//!
//! [`crate::action::discover_hall`] finds a hall by walking up from the
//! current directory looking for `ivar.json`. That works when the process's
//! cwd is still inside the hall tree — but a generated hook or plugin runs
//! as a child of the executor process, whose shell cwd can have drifted
//! anywhere in the course of a session (the agent's own commands can `cd`
//! wherever they like). Relying on walk-up discovery would make the guard's
//! reliability hostage to wherever the agent happened to leave its shell,
//! which is exactly backwards for the one thing in a session that must not
//! be foolable. Both generated artefacts instead carry the hall root
//! resolved once, at materialisation time, as an absolute path baked into
//! the script/plugin text — a Claude Code script `cd`s into it before
//! shelling out; the OpenCode plugin passes it as the child process's `cwd`
//! directly. Neither `ivar` command line has a `--hall`/`--cwd` flag to pass
//! this any other way — see `cli::root::ExecuteGuardCheckArgs`, which has
//! only `--feature`, `--session` and `--path`.
//!
//! # Merge, never clobber
//!
//! `<view_dir>/.claude/settings.json` is not this module's file alone — a
//! user, or the harness itself, may have written permissions or other hooks
//! into it. [`register_claude_hook`] reads whatever is there, replaces only
//! the `PreToolUse` entry that already targets this generated script (so
//! re-materialising a session, which happens on every `session connect`, is
//! idempotent rather than accumulating duplicates), and leaves every other
//! key untouched. A file that exists but fails to parse as a JSON object is
//! treated the same as no file at all: there is nothing coherent to merge
//! into, and a session's guard must still materialise rather than be held
//! hostage by an earlier malformed write. OpenCode needs no equivalent
//! registration step — its plugin loader discovers everything under
//! `.opencode/plugins/` on its own.
//!
//! # Rejected: reusing `Harness` from `super`
//!
//! The generated artefact only needs to know which dotdir and which template
//! to use, which is exactly [`Provider::config_dir`]. Keying this module off
//! `Provider` instead of the sibling [`super::Harness`] enum keeps it usable
//! wherever a `Provider` is already at hand (as it is on `SessionState`)
//! without a conversion, and keeps this module's compile-time surface
//! independent of the process-spawning concerns `Harness` owns.

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::error::Failure;

mod claude;
mod opencode;

// Test-only bridge: the mirrored tests in `tests/unit/harness/guard.rs` use
// `use super::*` to reach the module's internals; with the per-provider code
// now split into child modules, the items the tests exercise are re-exported
// here so the glob import still finds them.
#[cfg(test)]
pub(crate) use claude::{WRITE_TOOL_MATCHER, render_claude_guard_script};
#[cfg(test)]
pub(crate) use opencode::{js_string_literal, render_opencode_guard_plugin};

/// Filename of the generated Claude Code guard hook, under
/// `<view_dir>/.claude/hooks/`.
pub const CLAUDE_GUARD_SCRIPT: &str = "ivar-execution-guard.sh";

/// Filename of the generated OpenCode guard plugin, under
/// `<view_dir>/.opencode/plugins/`.
pub const OPENCODE_GUARD_PLUGIN: &str = "ivar-execution-guard.ts";

/// Materialise the execution guard for `provider` into `view_dir`, so that
/// every write the executor's harness attempts is arbitrated against
/// `feature`/`session_id`'s workstream on the board.
///
/// `hall_root` is the absolute hall path baked into the generated artefact —
/// see the module doc for why it cannot instead be discovered at guard-check
/// time. Returns the path to the artefact written (the hook script for
/// Claude Code, the plugin file for OpenCode), matching what the two
/// TypeScript predecessors this ports return.
pub fn materialise(
    provider: Provider,
    view_dir: &Utf8Path,
    hall_root: &Utf8Path,
    feature: &FeatureName,
    session_id: &SessionId,
) -> Result<Utf8PathBuf, Failure> {
    match provider {
        Provider::ClaudeCode => claude::materialise(view_dir, hall_root, feature, session_id),
        Provider::OpenCode => opencode::materialise(view_dir, hall_root, feature, session_id),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/harness/guard.rs"]
mod tests;
