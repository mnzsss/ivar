//! The `ivar upgrade` action.

use std::fmt;
use std::io::{self, Write};
use std::time::Duration;

use camino::Utf8PathBuf;
use serde::Serialize;

use super::cache::{self, CacheEntry};
use super::notice::current_version;
use crate::domain::upgrade::{
    Channel, ChannelEnv, UpgradePlan, Version, classify, tag_from_location, upgrade_plan,
};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;
use crate::infra::proc;
use crate::infra::release::{GithubRelease, LatestRelease};

const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Input for the `upgrade` action.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpgradeInput {
    /// If `true`, checks for available updates without applying them.
    pub check: bool,
}

/// Outcome of the `upgrade` action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpgradeReport {
    /// Current installed version.
    pub current: Version,
    /// Latest available version.
    pub latest: Version,
    /// Detected installation channel.
    pub channel: &'static str,
    /// Action taken or required.
    #[serde(flatten)]
    pub action: UpgradeAction,
}

/// Action taken or recommended during upgrade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum UpgradeAction {
    /// Already at the latest version.
    UpToDate,
    /// An update is available (`--check` mode).
    Available,
    /// Upgrade command was executed successfully.
    Ran {
        /// The executed command line.
        command: String,
    },
    /// Upgrade requires manual commands by the user.
    Manual {
        /// Suggested commands to run.
        commands: Vec<String>,
    },
}

impl WriteHuman for UpgradeReport {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        match &self.action {
            UpgradeAction::UpToDate => {
                writeln!(w, "ivar {} is the latest version", self.current)
            }
            UpgradeAction::Available => {
                writeln!(
                    w,
                    "ivar {} is available (you have {}) — run `ivar upgrade` to install it",
                    self.latest, self.current
                )
            }
            UpgradeAction::Ran { .. } => {
                writeln!(
                    w,
                    "upgraded ivar {} → {} ({})",
                    self.current, self.latest, self.channel
                )
            }
            UpgradeAction::Manual { commands } => {
                writeln!(
                    w,
                    "ivar {} is available (you have {}); this {} install is upgraded outside ivar:",
                    self.latest, self.current, self.channel
                )?;
                for cmd in commands {
                    writeln!(w, "  {cmd}")?;
                }
                Ok(())
            }
        }
    }
}

/// Injected dependencies for upgrading ivar.
pub struct UpgradeDeps<'a> {
    /// Source to query for latest release location.
    pub source: &'a dyn LatestRelease,
    /// Current installed version.
    pub current: Version,
    /// Canonicalized executable path, or `None` if unknown.
    pub exe: Option<Utf8PathBuf>,
    /// Environment variables for channel detection.
    pub env: ChannelEnv,
    /// Path to the update-check cache file.
    pub cache: Option<Utf8PathBuf>,
    /// Current timestamp in seconds.
    pub now: u64,
    /// Function to execute a command.
    pub run: &'a mut dyn FnMut(&proc::Command) -> Result<Option<i32>, Failure>,
}

impl fmt::Debug for UpgradeDeps<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpgradeDeps")
            .field("current", &self.current)
            .field("exe", &self.exe)
            .field("env", &self.env)
            .field("cache", &self.cache)
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

/// Reads channel detection environment variables from the process environment.
#[must_use]
pub fn channel_env() -> ChannelEnv {
    let get_path = |key: &str| -> Option<Utf8PathBuf> {
        std::env::var(key)
            .ok()
            .filter(|val| !val.is_empty())
            .map(Utf8PathBuf::from)
    };
    ChannelEnv {
        home: get_path("HOME"),
        cargo_home: get_path("CARGO_HOME"),
        ivar_install_dir: get_path("IVAR_INSTALL_DIR"),
    }
}

/// Upgrades ivar with injected dependencies.
///
/// # Errors
///
/// Returns [`Failure`] if:
/// - `upgrade.latest_unknown`: could not determine the latest ivar release.
/// - `upgrade.command_failed`: delegated upgrade command exited with a non-zero status or signal.
/// - `upgrade.spawn_failed`: delegated upgrade command could not be spawned.
pub fn upgrade_with(deps: UpgradeDeps<'_>, input: &UpgradeInput) -> Outcome<UpgradeReport> {
    let UpgradeDeps {
        source,
        current,
        exe,
        env,
        cache,
        now,
        run,
    } = deps;
    let latest = source
        .latest_location(FETCH_TIMEOUT)
        .ok()
        .and_then(|location| tag_from_location(&location))
        .ok_or_else(|| {
            Failure::failed(
                "upgrade.latest_unknown",
                "could not determine the latest ivar release",
            )
            .fix(FixAction::safe(
                "upgrade.retry",
                "check your network and run it again",
            ))
        })?;
    if let Some(path) = &cache {
        let _ = cache::write(
            path,
            &CacheEntry {
                last_checked_at: now,
                latest_version: Some(latest.to_string()),
            },
        );
    }
    let channel = exe
        .as_deref()
        .map_or(Channel::Unknown, |exe| classify(exe, &env));
    let report = |action| UpgradeReport {
        current,
        latest,
        channel: channel.name(),
        action,
    };

    if latest <= current {
        return Ok(Report::new(report(UpgradeAction::UpToDate)));
    }
    if input.check {
        return Ok(Report::new(report(UpgradeAction::Available)));
    }
    match upgrade_plan(&channel) {
        UpgradePlan::Print { commands } => {
            Ok(Report::new(report(UpgradeAction::Manual { commands })))
        }
        UpgradePlan::Run { program, args, env } => {
            let command = env
                .into_iter()
                .fold(proc::Command::new(program).args(args), |c, (k, v)| {
                    c.env(k, v)
                });
            let shown = command.display();
            match run(&command)? {
                Some(0) => Ok(Report::new(report(UpgradeAction::Ran { command: shown }))),
                code => Err(Failure::failed(
                    "upgrade.command_failed",
                    format!(
                        "`{shown}` exited with {}",
                        code.map_or("a signal".to_owned(), |c| format!("status {c}"))
                    ),
                )),
            }
        }
    }
}

/// Upgrades ivar to the latest release or checks for updates.
///
/// # Errors
///
/// Returns [`Failure`] if:
/// - `upgrade.dev_build`: running a development build with an unparseable package version.
/// - `upgrade.latest_unknown`: could not determine the latest ivar release.
/// - `upgrade.command_failed`: delegated upgrade command exited with a non-zero status or signal.
/// - `upgrade.spawn_failed`: delegated upgrade command could not be spawned.
pub fn upgrade(input: &UpgradeInput) -> Outcome<UpgradeReport> {
    let current = current_version().ok_or_else(|| {
        Failure::blocked(
            "upgrade.dev_build",
            "this is a development build of ivar; it has no release to upgrade from",
        )
    })?;
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| Utf8PathBuf::from_path_buf(p).ok())
        .and_then(|p| fs::canonicalize(&p).ok());
    let mut run = |command: &proc::Command| {
        let envs: String = command
            .envs()
            .iter()
            .map(|(k, v)| format!("{k}={v} "))
            .collect();
        let _ = writeln!(io::stderr().lock(), "running: {envs}{}", command.display());
        proc::inherit(command).map_err(|e| {
            Failure::failed(
                "upgrade.spawn_failed",
                format!("could not start `{}`: {e}", command.display()),
            )
        })
    };
    upgrade_with(
        UpgradeDeps {
            source: &GithubRelease,
            current,
            exe,
            env: channel_env(),
            cache: cache::cache_path(),
            now: cache::now_secs(),
            run: &mut run,
        },
        input,
    )
}

#[cfg(test)]
#[path = "../../../tests/unit/action/upgrade/command.rs"]
mod tests;
