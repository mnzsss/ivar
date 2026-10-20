//! The Docker CLI.

use crate::error::Failure;
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

/// `docker <args>`.
#[must_use]
pub fn command(args: &[String]) -> Command {
    Command::new("docker").args(args.iter().map(String::as_str))
}

fn run(args: &[String]) -> Result<(), Failure> {
    let cmd = command(args);
    let output = capture(&cmd)?;
    if output.success() {
        return Ok(());
    }
    Err(Failure::failed(
        "docker.command_failed",
        format!("`{}` failed: {}", cmd.display(), output.diagnostic()),
    ))
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}

/// # Errors
///
/// `docker.command_failed` when the network cannot be created.
pub fn network_create_internal(name: &str) -> Result<(), Failure> {
    run(&owned(&["network", "create", "--internal", name]))
}

/// # Errors
///
/// `docker.command_failed` when the container cannot join the network.
pub fn network_connect(network: &str, container: &str) -> Result<(), Failure> {
    run(&owned(&["network", "connect", network, container]))
}

/// # Errors
///
/// `docker.command_failed` when the container does not start.
pub fn run_detached(args: &[String]) -> Result<(), Failure> {
    run(args)
}

// The cleanup wrappers ignore failures: they run on every exit path, including
// after the resource is already gone.
pub fn network_remove(name: &str) {
    let _ = capture(&command(&owned(&["network", "rm", name])));
}

pub fn stop(container: &str, grace_secs: u32) {
    let _ = capture(&command(&owned(&[
        "stop",
        "-t",
        &grace_secs.to_string(),
        container,
    ])));
}

pub fn remove_force(container: &str) {
    let _ = capture(&command(&owned(&["rm", "-f", container])));
}

#[cfg(test)]
#[path = "../../tests/unit/infra/docker.rs"]
mod tests;
