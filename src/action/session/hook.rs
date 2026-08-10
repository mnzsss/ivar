//! The per-repo **Session Hook** — `.ivar/setups/<repo>.session.sh`, run once
//! per `session start`, in each promoted repo's feature worktree.
//!
//! # Why this is not the setup script
//!
//! `.ivar/setups/<repo>.sh` bootstraps a *worktree*: it installs dependencies
//! and materialises `.env`, and a receipt in the worktree's git admin directory
//! makes sure it runs about once. That receipt is the whole reason the setup
//! script is cheap enough that people keep running `ivar sync` — see
//! [`crate::store::setup_receipt`].
//!
//! Per-session state cannot live there. A session's database or compose project
//! has to be brought up *every* time a session opens, and several sessions can
//! share one promoted worktree, so a receipt keyed to the worktree would skip
//! exactly the runs that matter. The two lifetimes are different, so they get
//! two files.
//!
//! | | setup script | session hook |
//! | --- | --- | --- |
//! | file | `.ivar/setups/<repo>.sh` | `.ivar/setups/<repo>.session.sh` |
//! | runs on | `sync`, `promote`, `repo setup` | `session start` |
//! | how often | once per worktree, receipt-gated | once per session, ungated |
//! | typical body | `pnpm install`, `cp .env.example .env` | `docker compose up -d` |
//!
//! # Failure is a warning, never a refusal
//!
//! `promote` already treats a failed setup script as non-fatal: the repo stays
//! promoted and the user gets a warning. The same reasoning applies here with
//! more force — the view dir exists, the agent is about to spawn, and refusing
//! to open a session because one repo's optional hook exited non-zero would
//! trade a working session for no session at all. Every hook is attempted even
//! after one fails, for the same reason `sync` keeps going.
//!
//! # The environment
//!
//! Everything the setup script gets on the promote path, plus the two session
//! variables that have no meaning outside a session:
//!
//! - `IVAR_SESSION_ID` — this session's id.
//! - `IVAR_SESSION_PATH` — the view dir.
//!
//! `ARCHITECTURE.md` lists both in the environment contract. This is the file
//! that makes that true for a script.

use crate::domain::feature::Feature;
use crate::domain::name::{RepoName, SessionId};
use crate::error::Warning;
use crate::infra::{fs, proc};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

/// The interpreter a session hook runs under — `bash`, the same choice the
/// setup script makes, and for the same reason: a `.sh` arriving through a
/// clone may lack its executable bit.
const HOOK_INTERPRETER: &str = "bash";

/// Run every promoted repo's session hook, in manifest order.
///
/// A repo without a hook is skipped silently — the common case. A repo that is
/// not promoted is skipped too: its worktree is held read-only for this
/// session, and a hook that cannot write is a hook with nothing to do.
///
/// Returns one [`Warning`] per hook that failed. An empty vector is the happy
/// path and the usual one.
pub(crate) fn run_session_hooks(
    layout: &Layout,
    manifest: &Manifest,
    feature: &Feature,
    view_dir: &camino::Utf8Path,
    session: &SessionId,
) -> Vec<Warning> {
    manifest
        .repos()
        .iter()
        .filter(|repo| feature.is_promoted(repo.name()))
        .filter_map(|repo| run_hook(layout, repo.name(), feature, view_dir, session).err())
        .collect()
}

/// Run one repo's hook. `Ok(())` covers both "there is no hook" and "it ran and
/// exited zero" — from the caller's side those are the same outcome, and the
/// only thing worth returning is the warning when there is one.
fn run_hook(
    layout: &Layout,
    repo: &RepoName,
    feature: &Feature,
    view_dir: &camino::Utf8Path,
    session: &SessionId,
) -> Result<(), Warning> {
    let hook = layout.session_hook(repo);
    match fs::is_file(&hook) {
        Ok(true) => {}
        Ok(false) => return Ok(()),
        Err(error) => {
            return Err(Warning::new(
                "session.hook_unreadable",
                repo.to_string(),
                error.to_string(),
            ));
        }
    }

    // The hook runs in the worktree, not the view dir: a `docker compose` file
    // lives in the repo, and a hook that had to `cd` its way there would be
    // guessing at a path this module already knows.
    let worktree = layout.repo_worktree(repo, &feature.branch);
    match fs::is_dir(&worktree) {
        Ok(true) => {}
        Ok(false) => {
            return Err(Warning::new(
                "session.hook_no_worktree",
                repo.to_string(),
                format!("`{worktree}` does not exist — run `ivar sync`"),
            ));
        }
        Err(error) => {
            return Err(Warning::new(
                "session.hook_no_worktree",
                repo.to_string(),
                error.to_string(),
            ));
        }
    }

    let command = proc::Command::new(HOOK_INTERPRETER)
        .arg(hook.as_str())
        .cwd(&worktree)
        .env("IVAR_HALL", layout.root().as_str())
        .env("IVAR_REPO", repo.as_str())
        .env("IVAR_BRANCH", feature.branch.as_str())
        .env("IVAR_WORKTREE", worktree.as_str())
        .env("IVAR_WORKTREE_KIND", "feature")
        .env("IVAR_FEATURE", feature.name.as_str())
        .env("IVAR_SECRETS_DIR", layout.secrets_dir().as_str())
        .env("IVAR_SESSION_ID", session.as_str())
        .env("IVAR_SESSION_PATH", view_dir.as_str());

    // Streamed, not captured, for the same reason the setup script is: a
    // `docker compose up` pulling an image is minutes of output, and a frozen
    // line is indistinguishable from a hang.
    match proc::inherit(&command) {
        Ok(Some(0)) => Ok(()),
        Ok(code) => Err(Warning::new(
            "session.hook_failed",
            repo.to_string(),
            format!("`{hook}` {}", ended(code)),
        )),
        Err(error) => Err(Warning::new(
            "session.hook_failed",
            repo.to_string(),
            error.to_string(),
        )),
    }
}

/// How a process ended, in the words `sync` and `promote` already use for the
/// setup script. Same failure, same sentence.
fn ended(code: Option<i32>) -> String {
    match code {
        Some(code) => format!("exited {code}"),
        None => "was killed by a signal".to_owned(),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/hook.rs"]
mod tests;
