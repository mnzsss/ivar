//! Pure decisions behind the update notice and `ivar upgrade`: no env, no clock, no I/O.

use std::fmt;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Serialize, Serializer};

/// Time in seconds before checking again for a newer release.
pub const CHECK_INTERVAL_SECS: u64 = 20 * 60 * 60;

/// Base URL to fetch the install script.
pub const INSTALLER_URL: &str = "https://ivar.run/install";

/// A semver version parsed as `major.minor.patch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    /// Major component.
    pub major: u64,
    /// Minor component.
    pub minor: u64,
    /// Patch component.
    pub patch: u64,
}

impl Version {
    /// Parses a version string like `"0.13.0"` or `"v0.13.0"`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let bare = text.strip_prefix('v').unwrap_or(text);
        let mut parts = bare.split('.');
        let mut next = || -> Option<u64> {
            let part = parts.next()?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        };
        let (major, minor, patch) = (next()?, next()?, next()?);
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Extracts a version from a release redirect URL.
#[must_use]
pub fn tag_from_location(location: &str) -> Option<Version> {
    let (_, tag) = location.rsplit_once("/tag/")?;
    Version::parse(tag)
}

/// Determines whether the version cache is stale and should be checked again.
#[must_use]
pub fn is_stale(last_checked_at: Option<u64>, now: u64) -> bool {
    match last_checked_at {
        None => true,
        Some(at) if at > now => true,
        Some(at) => now - at >= CHECK_INTERVAL_SECS,
    }
}

/// Installation channel for ivar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Channel {
    /// Installed via curl installer script into a specific directory.
    Installer {
        /// Target binary directory.
        dir: Utf8PathBuf,
    },
    /// Installed via `cargo install`.
    Cargo,
    /// Installed via system package manager (e.g. `/usr/bin/ivar`).
    System,
    /// Unknown installation method.
    Unknown,
}

impl Channel {
    /// Returns the channel name as a static string slice.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Installer { .. } => "installer",
            Self::Cargo => "cargo",
            Self::System => "system",
            Self::Unknown => "unknown",
        }
    }
}

/// Environment hints for classifying the installation channel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelEnv {
    /// User's HOME directory.
    pub home: Option<Utf8PathBuf>,
    /// CARGO_HOME directory.
    pub cargo_home: Option<Utf8PathBuf>,
    /// IVAR_INSTALL_DIR override.
    pub ivar_install_dir: Option<Utf8PathBuf>,
}

/// Classifies how the executable was installed given its path and environment.
#[must_use]
pub fn classify(exe: &Utf8Path, env: &ChannelEnv) -> Channel {
    let Some(dir) = exe.parent() else {
        return Channel::Unknown;
    };
    let cargo_bin = env
        .cargo_home
        .clone()
        .or_else(|| env.home.as_ref().map(|h| h.join(".cargo")))
        .map(|c| c.join("bin"));
    if cargo_bin.as_deref() == Some(dir) {
        return Channel::Cargo;
    }
    if exe == Utf8Path::new("/usr/bin/ivar") {
        return Channel::System;
    }
    let local_bin = env.home.as_ref().map(|h| h.join(".local").join("bin"));
    if env.ivar_install_dir.as_deref() == Some(dir) || local_bin.as_deref() == Some(dir) {
        return Channel::Installer {
            dir: dir.to_path_buf(),
        };
    }
    Channel::Unknown
}

/// Context for deciding whether to display the update notice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoticeContext {
    /// User opted out via config or environment variable.
    pub opted_out: bool,
    /// Running in a CI environment.
    pub ci: bool,
    /// Stderr is an interactive terminal.
    pub stderr_tty: bool,
    /// Output is structured/machine-readable (e.g. JSON).
    pub machine_output: bool,
    /// Machine verb invoked (e.g. MCP / automation).
    pub machine_verb: bool,
    /// Built in release mode (not debug/development).
    pub release_build: bool,
}

/// Checks whether update notification should be printed.
#[must_use]
pub fn notice_enabled(ctx: &NoticeContext) -> bool {
    !ctx.opted_out
        && !ctx.ci
        && ctx.stderr_tty
        && !ctx.machine_output
        && !ctx.machine_verb
        && ctx.release_build
}

/// Formats the single-line update notification if `latest > current`.
#[must_use]
pub fn notice_line(current: Version, latest: Version) -> Option<String> {
    (latest > current)
        .then(|| format!("ivar {latest} is available (you have {current}) — run `ivar upgrade`"))
}

const INSTALLER_COMMAND: &str = concat!("curl -fsSL ", "https://ivar.run/install", " | sh");
const CARGO_COMMAND: &str = "cargo install ivar --locked";
const AUR_COMMANDS: [&str; 2] = ["paru -S ivar-bin", "yay -S ivar-bin"];

/// Plan of action for upgrading ivar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradePlan {
    /// Directly executable upgrade command.
    Run {
        /// Binary to execute.
        program: String,
        /// Command line arguments.
        args: Vec<String>,
        /// Environment variables to set.
        env: Vec<(String, String)>,
    },
    /// Manual commands to print for the user.
    Print {
        /// Suggested commands.
        commands: Vec<String>,
    },
}

/// Builds an upgrade plan based on the detected channel.
#[must_use]
pub fn upgrade_plan(channel: &Channel) -> UpgradePlan {
    match channel {
        Channel::Installer { dir } => UpgradePlan::Run {
            program: "sh".to_owned(),
            args: vec!["-c".to_owned(), INSTALLER_COMMAND.to_owned()],
            env: vec![("IVAR_INSTALL_DIR".to_owned(), dir.to_string())],
        },
        Channel::Cargo => UpgradePlan::Run {
            program: "cargo".to_owned(),
            args: vec![
                "install".to_owned(),
                "ivar".to_owned(),
                "--locked".to_owned(),
            ],
            env: vec![],
        },
        Channel::System => UpgradePlan::Print {
            commands: AUR_COMMANDS.iter().map(|c| (*c).to_owned()).collect(),
        },
        Channel::Unknown => UpgradePlan::Print {
            commands: [INSTALLER_COMMAND, CARGO_COMMAND, AUR_COMMANDS[0]]
                .iter()
                .map(|c| (*c).to_owned())
                .collect(),
        },
    }
}

#[cfg(test)]
#[path = "../../tests/unit/domain/upgrade.rs"]
mod tests;
