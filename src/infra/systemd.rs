//! The systemd user manager, through `systemctl --user` and `loginctl`.

use camino::Utf8PathBuf;

use crate::error::Failure;
use crate::infra::proc::{Command, capture};

/// `$XDG_CONFIG_HOME/systemd/user`, else `~/.config/systemd/user`.
///
/// # Errors
///
/// `systemd.no_home` when neither variable is set.
pub fn user_unit_dir() -> Result<Utf8PathBuf, Failure> {
    let config = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|dir| !dir.is_empty())
        .map(Utf8PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|home| !home.is_empty())
                .map(|home| Utf8PathBuf::from(home).join(".config"))
        })
        .ok_or_else(|| {
            Failure::blocked(
                "systemd.no_home",
                "neither XDG_CONFIG_HOME nor HOME is set, so the systemd user unit directory is unknown",
            )
        })?;
    Ok(config.join("systemd/user"))
}

fn systemctl(args: &[&str]) -> Command {
    Command::new("systemctl")
        .arg("--user")
        .args(args.iter().copied())
}

fn run(args: &[&str]) -> Result<(), Failure> {
    let command = systemctl(args);
    let output = capture(&command)?;
    if output.success() {
        return Ok(());
    }
    Err(Failure::failed(
        "systemd.command_failed",
        format!("`{}` failed: {}", command.display(), output.diagnostic()),
    ))
}

/// # Errors
///
/// `systemd.command_failed` when `systemctl --user daemon-reload` fails.
pub fn daemon_reload() -> Result<(), Failure> {
    run(&["daemon-reload"])
}

/// # Errors
///
/// `systemd.command_failed` when the unit cannot be enabled and started.
pub fn enable_now(unit: &str) -> Result<(), Failure> {
    run(&["enable", "--now", unit])
}

/// # Errors
///
/// `systemd.command_failed` when the unit cannot be disabled and stopped.
pub fn disable_now(unit: &str) -> Result<(), Failure> {
    run(&["disable", "--now", unit])
}

/// When the timer fires next, as systemd prints it; `None` when unknown.
#[must_use]
pub fn next_elapse(unit: &str) -> Option<String> {
    let output = capture(&systemctl(&[
        "show",
        unit,
        "-p",
        "NextElapseUSecRealtime",
        "--value",
    ]))
    .ok()?;
    let value = output.stdout.trim();
    (output.success() && !value.is_empty() && value != "n/a").then(|| value.to_owned())
}

/// Whether a systemd user manager answers.
#[must_use]
pub fn user_instance_reachable() -> bool {
    capture(&systemctl(&["show-environment"])).is_ok_and(|output| output.success())
}

/// The `PATH` the user manager hands its units; `None` when it does not answer.
#[must_use]
pub fn user_path() -> Option<String> {
    let output = capture(&systemctl(&["show-environment"])).ok()?;
    if !output.success() {
        return None;
    }
    output
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("PATH="))
        .map(str::to_owned)
}

/// Whether the current user's systemd instance outlives their sessions.
#[must_use]
pub fn linger_enabled() -> bool {
    let Ok(user) = std::env::var("USER") else {
        return false;
    };
    capture(&Command::new("loginctl").args(["show-user", user.as_str(), "-p", "Linger", "--value"]))
        .is_ok_and(|output| output.success() && output.stdout.trim() == "yes")
}

#[cfg(test)]
#[path = "../../tests/unit/infra/systemd.rs"]
mod tests;
