//! Browser launching boundary.
//!
//! Spawns the platform's default web browser to open an external URL.

use crate::infra::proc;

/// Constructs the platform opener [`proc::Command`] for `url`.
#[must_use]
pub fn opener(url: &str) -> proc::Command {
    #[cfg(target_os = "macos")]
    let command = proc::Command::new("open").arg(url);

    #[cfg(target_os = "windows")]
    let command = proc::Command::new("cmd").args(["/c", "start", "", url]);

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let command = proc::Command::new("xdg-open").arg(url);

    command
}

/// Open `url` in the system default browser via background process detachment.
///
/// # Errors
///
/// Returns [`proc::Error`] if the platform opener command cannot be spawned.
pub fn open(url: &str) -> Result<(), proc::Error> {
    let command = opener(url);
    proc::detach(&command)
}

#[cfg(test)]
#[path = "../../tests/unit/infra/browser.rs"]
mod tests;
