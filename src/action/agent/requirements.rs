//! What a machine needs before it can run scheduled agents.

use crate::action::hall::Diagnosis;
use crate::domain::agent::spec::{AgentSpec, Launch, Runtime};
use crate::domain::agent::tool::Tool;
use crate::error::Warning;
use crate::infra::{docker, systemd};

/// The machine checks, injectable so tests never spawn a probe.
pub struct Probes<'a> {
    pub systemd_user: &'a dyn Fn() -> bool,
    pub linger: &'a dyn Fn() -> bool,
    pub docker_client: &'a dyn Fn() -> bool,
    pub docker_daemon: &'a dyn Fn() -> bool,
    pub docker_rootless: &'a dyn Fn() -> Option<bool>,
    pub tool_on_path: &'a dyn Fn(Tool) -> bool,
    pub landlock: &'a dyn Fn() -> bool,
}

impl std::fmt::Debug for Probes<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Probes").finish_non_exhaustive()
    }
}

/// The probes that ask this machine.
#[must_use]
pub fn system_probes() -> Probes<'static> {
    Probes {
        systemd_user: &systemd::user_instance_reachable,
        linger: &systemd::linger_enabled,
        docker_client: &docker::client_present,
        docker_daemon: &docker::daemon_reachable,
        docker_rootless: &docker::rootless,
        tool_on_path: &tool_present,
        landlock: &crate::action::session::sandbox::landlock_available,
    }
}

fn tool_present(tool: Tool) -> bool {
    std::env::var_os("PATH").is_some_and(|path| on_path(tool.as_str(), &path))
}

fn on_path(program: &str, path: &std::ffi::OsStr) -> bool {
    std::env::split_paths(path).any(|dir| is_executable_file(&dir.join(program)))
}

#[cfg(unix)]
fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &std::path::Path) -> bool {
    path.is_file()
}

/// Every missing requirement, probed only when the hall defines agents.
#[must_use]
pub fn diagnose_agents(specs: &[AgentSpec], probes: &Probes<'_>) -> Vec<Diagnosis> {
    if specs.is_empty() {
        return Vec::new();
    }
    let mut findings = Vec::new();
    let mut need = |ok: bool, code, what: &str, fix: &str| {
        if !ok {
            findings.push(Diagnosis {
                code,
                what: what.to_owned(),
                fix: fix.to_owned(),
            });
        }
    };
    need(
        (probes.systemd_user)(),
        "agent.systemd_user_missing",
        "No systemd user instance answers, so agent timers cannot be installed.",
        "Log in through a systemd user session so `systemctl --user status` succeeds.",
    );
    need(
        (probes.linger)(),
        "agent.linger_disabled",
        "Linger is off, so agent timers stop when you log out.",
        "Run `loginctl enable-linger $USER`.",
    );
    if specs.iter().any(|s| s.runtime == Runtime::Docker) {
        let client = (probes.docker_client)();
        need(
            client,
            "agent.docker_missing",
            "The `docker` client is not on PATH.",
            "Install Docker Engine, preferably rootless: https://docs.docker.com/engine/security/rootless/",
        );
        if client {
            let daemon = (probes.docker_daemon)();
            need(
                daemon,
                "agent.docker_unreachable",
                "The Docker daemon does not answer.",
                "Start it: `systemctl --user start docker` (rootless) or `sudo systemctl start docker`.",
            );
            need(
                !daemon || (probes.docker_rootless)() != Some(false),
                "agent.docker_rootful",
                "Docker runs as root; access to its socket is root-equivalent on this machine.",
                "Prefer rootless Docker: https://docs.docker.com/engine/security/rootless/",
            );
        }
    }
    let mut host_tools: Vec<Tool> = Vec::new();
    for spec in specs.iter().filter(|s| s.runtime == Runtime::Host) {
        if let Launch::Tool { tool, .. } = spec.launch
            && !host_tools.contains(&tool)
        {
            host_tools.push(tool);
        }
    }
    for tool in &host_tools {
        need(
            (probes.tool_on_path)(*tool),
            "agent.tool_missing",
            &format!(
                "`{}` is not on PATH, so `runtime: host` agents using it cannot start.",
                tool.as_str()
            ),
            &format!("Install `{}` and log in once interactively.", tool.as_str()),
        );
    }
    need(
        host_tools.is_empty() || (probes.landlock)(),
        "agent.landlock_unavailable",
        "This kernel has no Landlock, so `runtime: host` agents refuse to run.",
        "Use Linux 5.13+ with Landlock enabled (`lsm=landlock,...`), or set `runtime: docker`.",
    );
    findings
}

/// The findings as `sync` warnings about `agents`.
#[must_use]
pub fn as_warnings(findings: &[Diagnosis]) -> Vec<Warning> {
    findings
        .iter()
        .map(|d| Warning::new(d.code, "agents", format!("{} {}", d.what, d.fix)))
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/agent/requirements.rs"]
mod tests;
