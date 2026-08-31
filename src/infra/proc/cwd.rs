//! Linux `/proc` working-directory attribution: which processes are running
//! inside a directory.
//!
//! The one Linux-specific capability in this crate, isolated here so the
//! subprocess module stays portable. Best-effort on Linux: an entry that
//! cannot be read yields no answer for that process, never an error.
//!
//! # Off Linux
//!
//! There is no `/proc` to walk, and nothing here replaces it, so
//! [`is_program_running_in`] answers `false` for every directory. Its one
//! caller, `session connect --create`, then reads every session as free and
//! attaches to the most recent one instead of starting a fresh session beside
//! it. Reusing a session someone is already in is the cost; opening a second
//! session on the same worktrees is what the check exists to avoid, and off
//! Linux it does not run. A `lsof -d cwd` fallback would close this.

use camino::Utf8Path;

/// Every process whose working directory is inside `dir`.
///
/// `/proc/<pid>/cwd` is a symlink to the process's working directory. An entry
/// that cannot be read — another user's process, or one that exited mid-walk —
/// is skipped.
#[cfg(target_os = "linux")]
fn pids_with_cwd_under(dir: &Utf8Path) -> Vec<u32> {
    let mut pids = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return pids;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(cwd) = std::fs::read_link(format!("/proc/{pid}/cwd")) else {
            continue;
        };
        if cwd.starts_with(dir.as_std_path()) {
            pids.push(pid);
        }
    }
    pids
}

/// Whether `program` is running with its working directory inside `dir` — the
/// "is this session in use?" primitive behind `session connect --create`.
///
/// The program filter is what makes the answer useful: `ivar` and the shell
/// that invoked it sit inside a View Dir whenever an agent calls in, so asking
/// "any process at all" would report every session in use, including the one
/// asking. Only the session's own harness binary counts as an occupant.
///
/// Always `false` off Linux — see the module docs for what that costs.
#[must_use]
pub fn is_program_running_in(dir: &Utf8Path, program: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        pids_with_cwd_under(dir).into_iter().any(|pid| {
            let cmdline =
                std::fs::read_to_string(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            cmdline.replace('\0', " ").contains(program)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (dir, program);
        false
    }
}
