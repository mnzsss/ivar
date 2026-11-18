//! Whether a process id still names the process that was recorded under it.

use super::{Command, capture};

/// `state` and `starttime` (clock ticks after boot) of `/proc/<pid>/stat`;
/// `None` when there is no such entry.
fn stat(pid: u32) -> Option<(char, String)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name, in parentheses, may hold spaces and parentheses.
    let (_, rest) = stat.rsplit_once(')')?;
    let mut fields = rest.split_whitespace();
    let state = fields.next()?.chars().next()?;
    // `state` is field 3 and `starttime` field 22.
    let start = fields.nth(18)?.to_owned();
    Some((state, start))
}

fn has_proc() -> bool {
    std::path::Path::new("/proc/self/stat").exists()
}

/// When `pid` started, as an opaque string to compare with itself later.
/// `None` where the platform has no `/proc` or the process is gone.
#[must_use]
pub fn start_time(pid: u32) -> Option<String> {
    stat(pid).map(|(_, start)| start)
}

/// Whether `pid` is a live process and, when `start` was recorded, the same
/// one. An exited process that has not been reaped yet does not count.
///
/// Without `/proc` only the pid is checked, through `kill -0`.
#[must_use]
pub fn is_alive(pid: u32, start: Option<&str>) -> bool {
    if has_proc() {
        return stat(pid)
            .is_some_and(|(state, now)| state != 'Z' && start.is_none_or(|then| then == now));
    }
    capture(&Command::new("kill").args(["-0", &pid.to_string()])).is_ok_and(|out| out.success())
}

#[cfg(test)]
#[path = "../../../tests/unit/infra/proc_alive.rs"]
mod tests;
