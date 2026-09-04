//! Active profile resolution and profile command bridge for OMP.
//!
//! OMP (Oh My Pi) discovers slash commands from the active profile's
//! commands directory:
//! - Default profile: `~/.omp/agent/commands` (or `$PI_CONFIG_DIR/agent/commands`)
//! - Named profile: `~/.omp/profiles/<name>/agent/commands` (or `$PI_CONFIG_DIR/profiles/<name>/agent/commands`)
//!
//! The profile command bridge creates/updates symlinks in the active profile's
//! `commands/` directory pointing back to the hall's `.omp/commands/ivar-*.md`
//! files, and removes stale ivar links belonging to this hall. Real user files
//! and commands not managed for this hall are preserved.

use std::collections::HashSet;

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction, Warning};
use crate::infra::fs::{self, SymlinkTarget};

const CONFIG_DIR_NAME: &str = ".omp";

/// Normalizes and validates an OMP profile name according to OMP rules.
///
/// Returns `None` for default profile (empty, whitespace, or `"default"`).
/// Returns `Some(name)` for a valid named profile.
/// Returns an error if the profile name is invalid (e.g. `.` or `..`, invalid characters, etc.).
pub fn normalize_profile_name(profile: Option<&str>) -> Result<Option<String>, Failure> {
    let Some(raw) = profile else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "default" {
        return Ok(None);
    }

    if trimmed == "."
        || trimmed == ".."
        || trimmed.ends_with('.')
        || !is_valid_profile_chars(trimmed)
        || is_windows_reserved_basename(trimmed)
    {
        return Err(Failure::failed(
            "omp.invalid_profile",
            format!("Invalid OMP profile \"{trimmed}\""),
        )
        .expected("profile name matching [a-z0-9][a-z0-9._-]{0,63}")
        .actual(trimmed.to_owned())
        .fix(FixAction::safe(
            "omp.set_valid_profile",
            "Set OMP_PROFILE or PI_PROFILE to a valid profile name.",
        )));
    }

    Ok(Some(trimmed.to_owned()))
}

fn is_valid_profile_chars(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    // Must start with [a-z0-9]
    let Some(&first) = bytes.first() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    // Remainder must be [a-z0-9._-]
    bytes.iter().all(|&b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_' || b == b'-'
    })
}

fn is_windows_reserved_basename(name: &str) -> bool {
    let stem = match name.split_once('.') {
        Some((stem, _)) => stem,
        None => name,
    };
    let stem_upper = stem.to_ascii_uppercase();
    matches!(
        stem_upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM0"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT0"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Resolves the active profile name from env vars: `OMP_PROFILE` takes precedence over `PI_PROFILE`.
pub fn resolve_profile_from_env(
    omp_env: Option<&str>,
    pi_env: Option<&str>,
) -> Result<Option<String>, Failure> {
    // If OMP_PROFILE is set (even if empty), PI_PROFILE is ignored.
    let selected = match omp_env {
        Some(val) => Some(val),
        None => pi_env,
    };
    normalize_profile_name(selected)
}

/// Resolves user home directory from `HOME` (or `USERPROFILE` on Windows).
pub fn user_home_from(
    home: Option<String>,
    userprofile: Option<String>,
    os: &str,
) -> Result<Utf8PathBuf, Failure> {
    if let Some(path) = home.filter(|h| !h.is_empty()) {
        let p = Utf8PathBuf::from(path);
        if p.is_absolute() {
            return Ok(p);
        }
    }
    if os == "windows"
        && let Some(path) = userprofile.filter(|u| !u.is_empty())
    {
        let p = Utf8PathBuf::from(&path);
        // On Windows (or cross-platform test), path starting with "C:\" or "\" or "/" is absolute
        if p.is_absolute() || path.chars().nth(1) == Some(':') {
            return Ok(p);
        }
    }
    Err(
        Failure::failed("fs.user_home", "could not resolve user home directory")
            .expected("$HOME (or on Windows, %USERPROFILE%) set to an absolute path")
            .actual("no home directory variable resolved to an absolute path")
            .fix(FixAction::safe(
                "fs.set_home",
                "Set $HOME or %USERPROFILE% to an absolute path.",
            )),
    )
}

/// Resolves user home directory from live process environment.
pub fn user_home() -> Result<Utf8PathBuf, Failure> {
    user_home_from(
        std::env::var("HOME").ok(),
        std::env::var("USERPROFILE").ok(),
        std::env::consts::OS,
    )
}

/// Resolves the active OMP agent commands directory for the current environment.
pub fn resolve_active_commands_dir() -> Result<Utf8PathBuf, Failure> {
    let home = user_home()?;
    let config_dir = std::env::var("PI_CONFIG_DIR").ok();
    let omp_profile = std::env::var("OMP_PROFILE").ok();
    let pi_profile = std::env::var("PI_PROFILE").ok();
    resolve_commands_dir_from(
        &home,
        config_dir.as_deref(),
        omp_profile.as_deref(),
        pi_profile.as_deref(),
    )
}

/// Resolves the OMP agent commands directory from explicit inputs.
pub fn resolve_commands_dir_from(
    home: &Utf8Path,
    pi_config_dir: Option<&str>,
    omp_profile: Option<&str>,
    pi_profile: Option<&str>,
) -> Result<Utf8PathBuf, Failure> {
    let profile = resolve_profile_from_env(omp_profile, pi_profile)?;
    let base_name = pi_config_dir
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(CONFIG_DIR_NAME);

    let config_root = home.join(base_name);
    let agent_dir = match profile {
        Some(name) => config_root.join("profiles").join(name).join("agent"),
        None => config_root.join("agent"),
    };
    Ok(agent_dir.join("commands"))
}

/// Reconciles ivar command symlinks in the active OMP profile commands directory.
///
/// For each shipped command in `hall/.omp/commands/ivar-*.md`:
/// - Creates or replaces a symlink `<profile_commands_dir>/ivar-<id>.md` pointing to `<hall>/.omp/commands/ivar-<id>.md`.
/// - If the target in `<profile_commands_dir>` is a regular file or not a symlink, it is NEVER overwritten; a warning is issued.
///
/// Any stale `ivar-*.md` symlinks in `<profile_commands_dir>` pointing to this hall's `.omp/commands/` are removed.
pub fn bridge_sync(
    hall_commands_dir: &Utf8Path,
    command_file_names: &[&str],
    warnings: &mut Vec<Warning>,
) {
    let profile_commands_dir = match resolve_active_commands_dir() {
        Ok(dir) => dir,
        Err(err) => {
            warnings.push(Warning::new(
                "omp.profile_bridge_failed",
                Provider::Omp.id(),
                format!("could not resolve OMP profile commands directory: {err}"),
            ));
            return;
        }
    };
    bridge_sync_under(
        &profile_commands_dir,
        hall_commands_dir,
        command_file_names,
        warnings,
    );
}

/// Parameterised variant of [`bridge_sync`] for testing and custom paths.
pub fn bridge_sync_under(
    profile_commands_dir: &Utf8Path,
    hall_commands_dir: &Utf8Path,
    command_file_names: &[&str],
    warnings: &mut Vec<Warning>,
) {
    if let Err(err) = fs::ensure_dir(profile_commands_dir) {
        warnings.push(Warning::new(
            "omp.profile_bridge_failed",
            Provider::Omp.id(),
            format!("could not create OMP commands directory at {profile_commands_dir}: {err}"),
        ));
        return;
    }

    let expected_names: HashSet<String> = command_file_names
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    // 1. Project symlinks for catalog commands
    for file_name in command_file_names {
        let hall_target = hall_commands_dir.join(file_name);
        let link_path = profile_commands_dir.join(file_name);
        match fs::read_symlink(&link_path) {
            Ok(SymlinkTarget::NotASymlink) => {
                warnings.push(Warning::new(
                    "omp.profile_bridge_conflict",
                    Provider::Omp.id(),
                    format!(
                        "cannot bridge command {file_name}: {link_path} exists and is not a symlink"
                    ),
                ));
            }
            Ok(SymlinkTarget::Target(current)) if current == hall_target => {
                // Already pointing correctly
            }
            Ok(SymlinkTarget::Target(_) | SymlinkTarget::Absent) => {
                if let Err(err) = fs::replace_symlink_if_changed(&hall_target, &link_path) {
                    warnings.push(Warning::new(
                        "omp.profile_bridge_failed",
                        Provider::Omp.id(),
                        format!("failed to bridge command {file_name} to {link_path}: {err}"),
                    ));
                }
            }
            Err(err) => {
                warnings.push(Warning::new(
                    "omp.profile_bridge_failed",
                    Provider::Omp.id(),
                    format!("failed to inspect {link_path}: {err}"),
                ));
            }
        }
    }

    // 2. Clean up stale ivar symlinks pointing into this hall's commands dir
    cleanup_stale_links(
        profile_commands_dir,
        hall_commands_dir,
        &expected_names,
        warnings,
    );
}

pub fn bridge_remove(hall_commands_dir: &Utf8Path, warnings: &mut Vec<Warning>) {
    let profile_commands_dir = match resolve_active_commands_dir() {
        Ok(dir) => dir,
        Err(_) => return, // If profile commands dir cannot be resolved, nothing to clean up
    };
    bridge_remove_under(&profile_commands_dir, hall_commands_dir, warnings);
}

/// Parameterised variant of [`bridge_remove`] for testing and custom paths.
pub fn bridge_remove_under(
    profile_commands_dir: &Utf8Path,
    hall_commands_dir: &Utf8Path,
    warnings: &mut Vec<Warning>,
) {
    if !fs::is_dir(profile_commands_dir).unwrap_or(false) {
        return;
    }

    cleanup_stale_links(
        profile_commands_dir,
        hall_commands_dir,
        &HashSet::new(),
        warnings,
    );
}

fn cleanup_stale_links(
    profile_commands_dir: &Utf8Path,
    hall_commands_dir: &Utf8Path,
    expected_names: &HashSet<String>,
    warnings: &mut Vec<Warning>,
) {
    let entries = match fs::read_dir(profile_commands_dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries {
        let Some(name) = entry.file_name() else {
            continue;
        };

        // Only manage entries matching ivar-*.md
        if !name.starts_with("ivar-") || !name.ends_with(".md") {
            continue;
        }

        // If it's expected and already managed, do not remove
        if expected_names.contains(name) {
            continue;
        }

        // Check if the symlink points into this hall's commands dir
        match fs::read_symlink(&entry) {
            Ok(SymlinkTarget::Target(target)) => {
                if target.starts_with(hall_commands_dir)
                    && let Err(err) = fs::remove_file(&entry)
                {
                    warnings.push(Warning::new(
                        "omp.profile_bridge_failed",
                        Provider::Omp.id(),
                        format!("failed to remove stale profile link {entry}: {err}"),
                    ));
                }
            }
            Ok(SymlinkTarget::NotASymlink | SymlinkTarget::Absent) => {
                // Not a symlink or absent: leave untouched
            }
            Err(err) => {
                warnings.push(Warning::new(
                    "omp.profile_bridge_failed",
                    Provider::Omp.id(),
                    format!("failed to inspect profile command {entry}: {err}"),
                ));
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/providers/omp/commands.rs"]
mod tests;
