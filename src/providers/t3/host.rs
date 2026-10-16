//! The only place ivar runs `t3`. The contract is measured against `t3` 0.1.0 with the
//! desktop app running. The `t3` run is the one named by `T3CODE_CLI_PATH` (T3 Code sets it
//! for its agents) and otherwise the first `t3` on PATH, which can be an older, stale install:
//! the `project` commands of 0.0.45 do not reach the running server. `t3 app <dir>` registers
//! and opens a project in one idempotent call and exits 1 for a directory it cannot open;
//! `t3 project remove <path>` exits 0 for a project and 1 with `ProjectNotFoundError` for an
//! unknown one. Without `--force`, removing a project that holds threads exits 1 with
//! `ProjectLiveServerDeclaredResponseError`, so every failure other than `ProjectNotFoundError`
//! is reported as "left behind". `t3 app` with the desktop app stopped is unmeasured.

use camino::Utf8Path;

use super::launch;
use crate::error::{Failure, FixAction};
use crate::infra::proc::{self, Command};

const NOT_REGISTERED: &str = "ProjectNotFoundError";
const MIN_VERSION: (u64, u64, u64) = (0, 1, 0);
const MIN_VERSION_TEXT: &str = "0.1.0";
pub const VERSION_ADVICE: &str = "Run ivar from inside T3 Code (it sets `T3CODE_CLI_PATH`), point `T3CODE_CLI_PATH` or PATH at t3 0.1.0 or newer, or start the session with another provider.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    Ready,
    Missing,
    Unreachable(String),
    /// Carries the version found, empty when it could not be read.
    Outdated(String),
}

#[must_use]
pub(crate) fn program() -> String {
    program_from(std::env::var("T3CODE_CLI_PATH").ok().as_deref())
}

#[must_use]
pub(crate) fn program_from(env: Option<&str>) -> String {
    env.filter(|path| !path.is_empty())
        .unwrap_or(launch::contract().binary)
        .to_owned()
}

fn t3() -> Command {
    Command::new(program())
}

#[must_use]
pub(crate) fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    text.split_whitespace().find_map(|token| {
        let core = token.trim_start_matches('v').split(['-', '+']).next()?;
        let mut parts = core.split('.').map(str::parse::<u64>);
        let version = (
            parts.next()?.ok()?,
            parts.next()?.ok()?,
            parts.next()?.ok()?,
        );
        parts.next().is_none().then_some(version)
    })
}

enum VersionCheck {
    Unrunnable,
    Unsupported(String),
}

fn check_version() -> Result<(), VersionCheck> {
    let output = proc::capture(&t3().arg("--version")).map_err(|_| VersionCheck::Unrunnable)?;
    let found = output
        .success()
        .then(|| parse_version(&output.stdout))
        .flatten();
    match found {
        Some(version) if version >= MIN_VERSION => Ok(()),
        Some((major, minor, patch)) => Err(VersionCheck::Unsupported(format!(
            "{major}.{minor}.{patch}"
        ))),
        None => Err(VersionCheck::Unsupported(String::new())),
    }
}

#[must_use]
pub fn unsupported_summary(found: &str) -> String {
    if found.is_empty() {
        format!(
            "the version of `t3` could not be read, and ivar's T3 integration needs {MIN_VERSION_TEXT} or newer"
        )
    } else {
        format!(
            "`t3` v{found} is older than {MIN_VERSION_TEXT}, the version ivar's T3 integration was measured against"
        )
    }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().trim().to_owned()
}

/// # Errors
/// `t3.binary_missing` when `t3 --version` cannot run, `t3.version_unsupported` when it
/// fails, prints no version, or reports one older than 0.1.0.
pub fn preflight() -> Result<(), Failure> {
    match check_version() {
        Ok(()) => Ok(()),
        Err(VersionCheck::Unrunnable) => Err(Failure::blocked(
            "t3.binary_missing",
            "`t3` is not installed or does not run",
        )
        .expected("`t3 --version` to succeed")
        .actual("`t3` could not be run")
        .fix(FixAction::safe(
            "t3.install",
            "Install T3 Code and put `t3` on PATH, or start the session with another provider.",
        ))),
        Err(VersionCheck::Unsupported(found)) => Err(Failure::blocked(
            "t3.version_unsupported",
            unsupported_summary(&found),
        )
        .expected(format!("`t3` {MIN_VERSION_TEXT} or newer"))
        .actual(if found.is_empty() {
            "the version could not be read".to_owned()
        } else {
            format!("`t3` {found}")
        })
        .fix(FixAction::safe("t3.version", VERSION_ADVICE))),
    }
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
    match check_version() {
        Err(VersionCheck::Unrunnable) => return Probe::Missing,
        Err(VersionCheck::Unsupported(found)) => return Probe::Outdated(found),
        Ok(()) => {}
    }
    match remove(never_a_project) {
        Err(_) => Probe::Missing,
        Ok(output) if output.diagnostic().contains(NOT_REGISTERED) => Probe::Ready,
        Ok(output) => Probe::Unreachable(first_line(&output.diagnostic())),
    }
}

fn remove(path: &Utf8Path) -> Result<proc::Output, proc::Error> {
    proc::capture(&t3().args(["project", "remove"]).arg(path.as_str()))
}
