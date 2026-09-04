//! Refusing a commit on a repository's default branch.
//!
//! A hall mounts every repo's default branch as a real worktree, so the branch
//! nobody should commit to is checked out and writable at all times. The
//! session guard classifies structured tool calls, which leaves a plain shell
//! command free to commit there. A `pre-commit` hook is the layer that catches
//! what the guard cannot see, because git enforces it wherever the commit is
//! invoked from.
//!
//! This sits apart from `exec.rs` because it is the one thing in `git/` that
//! configures a repository rather than operating on one.

use camino::Utf8Path;

use crate::infra::fs;

use super::Error;
use super::exec::{git, run};

/// What a call to [`protect_default_branch`] actually had to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protection {
    /// The hook was written or rewritten.
    Installed,
    /// Everything was already in place; nothing was touched.
    AlreadyInstalled,
}

/// `value` as a single-quoted POSIX shell literal.
///
/// Single quotes are the only shell quoting that protects everything — `$`,
/// backtick, `;`, newline — and the single quote itself is the one character
/// they cannot contain, so it is emitted as `'\''`: close, escaped quote,
/// reopen.
///
/// This exists because [`hook_bytes`] writes a value into a script that runs
/// on every commit, and a branch name is not a safe shell token.
/// [`crate::domain::name`]'s `BranchName` validates git's *ref* grammar, which
/// has never excluded `$`, backtick, `;` or `|` — `refs/heads/$(id)` is a legal
/// git branch. The name is therefore valid and still dangerous, and the fix
/// belongs at the sink that builds shell source, not in a newtype that would
/// have to reject names git accepts.
fn shell_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// The bytes of the `pre-commit` hook, with `default_branch` baked in.
///
/// Baked in rather than read from config at commit time: a hook that looks up
/// its own configuration has a second way to fail, and its failure mode is to
/// let the commit through. The branch name is known at install time and never
/// changes without a re-install, so there is nothing to look up.
///
/// `symbolic-ref` rather than `rev-parse --abbrev-ref`, because on a branch
/// with no commits the latter answers the literal string `HEAD` — which is not
/// the branch name, so the guard would wave the very first commit through.
/// A detached HEAD makes `symbolic-ref` exit non-zero, which is the one case
/// this hook deliberately allows: there is no branch to protect.
///
/// See [`shell_single_quoted`] for the quoting rationale on the interpolated
/// branch name.
fn hook_bytes(default_branch: &str) -> String {
    let quoted = shell_single_quoted(default_branch);
    format!(
        r#"#!/bin/sh
# Installed by ivar. Refuses commits on this repository's default branch.
branch=$(git symbolic-ref --short -q HEAD) || exit 0
if [ "$branch" = {quoted} ]; then
    printf '%s\n' "ivar: refusing a commit on the default branch ${{branch}}." >&2
    printf '%s\n' "ivar: promote this repo onto a feature and commit there." >&2
    exit 1
fi
exit 0
"#
    )
}

/// Make committing on `default_branch` refuse, in the worktree at
/// `default_worktree` and nowhere else.
///
/// # Why worktree-local config
///
/// A hook in `<bare>/hooks` is silently disabled the moment anything writes
/// `core.hooksPath` into the shared config — which is exactly what husky does
/// from `pnpm install`, and ivar's own setup script runs `pnpm install`. So the
/// hook lives in its own directory and is selected per worktree, where a
/// project's hook manager does not reach. A feature worktree keeps the
/// project's hooks; only the default worktree gets ivar's.
///
/// # Why `core.bare` has to move
///
/// `extensions.worktreeConfig` makes each worktree read its own config file,
/// but `core.bare=true` still lives in the *shared* config, where every linked
/// worktree inherits it. A worktree that believes it is bare answers
/// `fatal: this operation must be run in a work tree` to almost everything.
///
/// The fix is git's own documented migration for this extension: move
/// `core.bare` into the bare repository's own worktree config and unset it from
/// the shared one. Writing `core.bare=false` into each existing worktree
/// instead would fix only the worktrees that exist right now — every worktree
/// added afterwards would be born broken, which is most of them in a hall.
pub(crate) fn protect_default_branch(
    bare_path: &Utf8Path,
    default_worktree: &Utf8Path,
    default_branch: &str,
) -> Result<Protection, Error> {
    let bare_git = || git().arg("--git-dir").arg(bare_path.as_str());

    run(bare_git()
        .arg("config")
        .arg("extensions.worktreeConfig")
        .arg("true"))?;

    // Order matters: the bare repo must own `core.bare=true` before it is
    // removed from the shared config, or in between the two commands the
    // repository is not bare to anyone.
    // `--local` asks about the shared config alone. A plain `--get` would read
    // back the worktree config this migration just wrote and try to unset the
    // shared key a second time, which exits non-zero for "nothing to unset".
    let unmigrated = run(bare_git()
        .arg("config")
        .arg("--local")
        .arg("--get")
        .arg("core.bare"))
    .map(|value| value.trim() == "true")
    .unwrap_or(false);
    if unmigrated {
        run(bare_git()
            .arg("config")
            .arg("--worktree")
            .arg("core.bare")
            .arg("true"))?;
        run(bare_git().arg("config").arg("--unset").arg("core.bare"))?;
    }

    let hooks_dir = bare_path.join("ivar-hooks");
    fs::ensure_dir(&hooks_dir)?;
    let hook = hooks_dir.join("pre-commit");
    let wanted = hook_bytes(default_branch);

    let installed = fs::read_text(&hook)?.as_deref() == Some(wanted.as_str());
    if !installed {
        fs::write_text(&hook, &wanted)?;
        fs::chmod(&hook, 0o755)?;
    }

    // Absolute: a relative `core.hooksPath` resolves against the worktree, and
    // the hook does not live there.
    run(git()
        .cwd(default_worktree.as_str())
        .arg("config")
        .arg("--worktree")
        .arg("core.hooksPath")
        .arg(hooks_dir.as_str()))?;

    Ok(if installed {
        Protection::AlreadyInstalled
    } else {
        Protection::Installed
    })
}

#[cfg(test)]
#[path = "../../tests/unit/git/protect.rs"]
mod tests;
