//! Linux `/proc` working-directory attribution: which processes are running
//! inside a directory.
//!
//! The one Linux-specific capability in this crate, isolated here so the
//! subprocess module stays portable. Best-effort throughout: a missing `/proc`
//! (non-Linux) or an unreadable entry yields an empty list or `false`, never an
//! error — absence is a warning, not a failure.

use camino::Utf8Path;

/// Every process whose working directory is inside `dir`.
///
/// `/proc/<pid>/cwd` is a symlink to the process's working directory. An entry
/// that cannot be read (another user's process, or one that exited mid-walk)
/// is skipped, and a non-Linux host yields an empty list.
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
/// The program filter is what makes the answer useful: `ivar` itself, and the
/// shell that invoked it, are routinely cwd'd inside a View Dir, so asking
/// "any process at all" would call every session in use — including the one
/// asking. Only the session's own harness binary counts as an occupant.
#[must_use]
pub fn is_program_running_in(dir: &Utf8Path, program: &str) -> bool {
    pids_with_cwd_under(dir).into_iter().any(|pid| {
        let cmdline = std::fs::read_to_string(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        cmdline.replace('\0', " ").contains(program)
    })
}
