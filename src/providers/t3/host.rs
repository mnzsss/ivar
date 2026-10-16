//! The only place ivar runs `t3`. The contract is measured against `t3` 0.1.0 with the
//! desktop app running: `t3 app <dir>` registers and opens a project in one idempotent
//! call and exits 1 for a directory it cannot open; `t3 project remove <path>` exits 0 for
//! a project and 1 with `ProjectNotFoundError` for an unknown one. The refusal to remove a
//! project that still holds threads is assumed, not measured, so every other failure is
//! reported as "left behind".

use camino::Utf8Path;

use super::launch;
use crate::error::{Failure, FixAction};
use crate::infra::proc::{self, Command};

const NOT_REGISTERED: &str = "ProjectNotFoundError";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    Ready,
    Missing,
    Unreachable(String),
}

fn t3() -> Command {
    Command::new(launch::contract().binary)
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().trim().to_owned()
}

/// # Errors
/// `t3.binary_missing` when `t3 --version` cannot run or exits non-zero.
pub fn preflight() -> Result<(), Failure> {
    let ran = proc::capture(&t3().arg("--version")).is_ok_and(|output| output.success());
    if ran {
        return Ok(());
    }
    Err(
        Failure::blocked("t3.binary_missing", "`t3` is not installed or does not run")
            .expected("`t3 --version` to succeed")
            .actual("`t3` could not be run")
            .fix(FixAction::safe(
                "t3.install",
                "Install T3 Code and put `t3` on PATH, or start the session with another provider.",
            )),
    )
}

/// # Errors
/// The first line of what `t3 app` said, or why it could not run.
pub fn open(view_dir: &Utf8Path) -> Result<(), String> {
    let output = proc::capture(&t3().arg("app").arg(view_dir.as_str()))
        .map_err(|error| error.to_string())?;
    if output.success() {
        return Ok(());
    }
    Err(first_line(&output.diagnostic()))
}

/// Removes without `--force`: `--force` also deletes the project's threads.
///
/// # Errors
/// The first line of what `t3 project remove` said, or why it could not run.
pub fn close(view_dir: &Utf8Path) -> Result<(), String> {
    let output = remove(view_dir).map_err(|error| error.to_string())?;
    if output.success() || output.diagnostic().contains(NOT_REGISTERED) {
        return Ok(());
    }
    Err(first_line(&output.diagnostic()))
}

/// Asks for a path that is never a project: only a reachable server answers `ProjectNotFoundError`.
#[must_use]
pub fn probe(never_a_project: &Utf8Path) -> Probe {
    match remove(never_a_project) {
        Err(_) => Probe::Missing,
        Ok(output) if output.diagnostic().contains(NOT_REGISTERED) => Probe::Ready,
        Ok(output) => Probe::Unreachable(first_line(&output.diagnostic())),
    }
}

fn remove(path: &Utf8Path) -> Result<proc::Output, proc::Error> {
    proc::capture(&t3().args(["project", "remove"]).arg(path.as_str()))
}
