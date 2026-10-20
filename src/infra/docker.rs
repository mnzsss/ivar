//! The Docker CLI.

use crate::infra::proc::{Command, capture};

fn succeeds(command: &Command) -> bool {
    capture(command).is_ok_and(|output| output.success())
}

/// Whether a `docker` client runs.
#[must_use]
pub fn client_present() -> bool {
    succeeds(&Command::new("docker").arg("--version"))
}

/// Whether the Docker daemon answers.
#[must_use]
pub fn daemon_reachable() -> bool {
    succeeds(&Command::new("docker").args(["info", "--format", "{{.ServerVersion}}"]))
}

/// Whether the daemon runs rootless; `None` when it does not answer.
#[must_use]
pub fn rootless() -> Option<bool> {
    let output =
        capture(&Command::new("docker").args(["info", "--format", "{{json .SecurityOptions}}"]))
            .ok()
            .filter(|output| output.success())?;
    Some(output.stdout.contains("rootless"))
}

#[cfg(test)]
#[path = "../../tests/unit/infra/docker.rs"]
mod tests;
