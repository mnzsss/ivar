//! The systemd user manager, through `systemctl --user` and `loginctl`.

use camino::{Utf8Path, Utf8PathBuf};

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

/// `$XDG_DATA_HOME`, else `~/.local/share`: where the systemd user manager keeps
/// its state, whatever the OS convention for ivar's own data.
///
/// # Errors
///
/// `systemd.no_home` when neither variable is usable.
pub fn user_data_dir() -> Result<Utf8PathBuf, Failure> {
    let absolute = |value: Option<String>| {
        value
            .map(Utf8PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    absolute(std::env::var("XDG_DATA_HOME").ok())
        .or_else(|| {
            absolute(std::env::var("HOME").ok()).map(|home| home.join(".local/share"))
        })
        .ok_or_else(|| {
            Failure::blocked(
                "systemd.no_home",
                "neither an absolute XDG_DATA_HOME nor HOME is set, so the systemd user state directory is unknown",
            )
        })
}

/// Where systemd records when a `Persistent=true` timer last fired:
/// `<data_dir>/systemd/timers/stamp-<timer unit>`. Its mtime is the base the
/// timer counts its next elapse from when it starts.
#[must_use]
pub fn stamp_path(data_dir: &Utf8Path, timer_unit: &str) -> Utf8PathBuf {
    data_dir
        .join("systemd/timers")
        .join(format!("stamp-{timer_unit}"))
}

/// Whether `timer_unit` has a stamp.
#[must_use]
pub fn stamp_exists(data_dir: &Utf8Path, timer_unit: &str) -> bool {
    stamp_path(data_dir, timer_unit).exists()
}

/// Writes an empty stamp for `timer_unit` with `mtime_epoch_secs` as its mtime
/// unless one exists; returns whether it wrote one. An existing stamp is never
/// touched.
///
/// # Errors
///
/// `systemd.stamp_failed` when the directory or the file cannot be written.
pub fn write_stamp_if_missing(
    data_dir: &Utf8Path,
    timer_unit: &str,
    mtime_epoch_secs: u64,
) -> Result<bool, Failure> {
    let path = stamp_path(data_dir, timer_unit);
    let failed = |source: &dyn std::fmt::Display| {
        Failure::failed(
            "systemd.stamp_failed",
            format!("could not write the timer stamp {path}: {source}"),
        )
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| failed(&e))?;
    }
    let file = match std::fs::File::options()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(failed(&e)),
    };
    let mtime = std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime_epoch_secs);
    if let Err(e) = file.set_modified(mtime) {
        // A stamp with the wrong mtime would hide the missed run it exists to catch.
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(failed(&e));
    }
    Ok(true)
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
/// `systemd.command_failed` when the unit cannot be restarted.
pub fn restart(unit: &str) -> Result<(), Failure> {
    run(&["restart", unit])
}

/// Queues a start and returns without waiting for the unit to finish, which
/// for a oneshot service would be the whole agent run.
///
/// # Errors
///
/// `systemd.command_failed` when the start cannot be queued.
pub fn start_no_block(unit: &str) -> Result<(), Failure> {
    run(&["start", "--no-block", unit])
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
