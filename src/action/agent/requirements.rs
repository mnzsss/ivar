//! What a machine needs before it can run scheduled agents.

use crate::action::hall::Diagnosis;
use crate::error::Warning;
use crate::infra::{docker, systemd};

/// The machine checks, injectable so tests never spawn a probe.
pub struct Probes<'a> {
    pub systemd_user: &'a dyn Fn() -> bool,
    pub linger: &'a dyn Fn() -> bool,
    pub docker_client: &'a dyn Fn() -> bool,
    pub docker_daemon: &'a dyn Fn() -> bool,
    pub docker_rootless: &'a dyn Fn() -> Option<bool>,
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
    }
}

/// Every missing requirement, probed only when the hall defines agents.
#[must_use]
pub fn diagnose_agents(has_agents: bool, probes: &Probes<'_>) -> Vec<Diagnosis> {
    if !has_agents {
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
