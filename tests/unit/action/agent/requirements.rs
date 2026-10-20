#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

fn probes<'a>(ok: &'a dyn Fn() -> bool, rootless: &'a dyn Fn() -> Option<bool>) -> Probes<'a> {
    Probes {
        systemd_user: ok,
        linger: ok,
        docker_client: ok,
        docker_daemon: ok,
        docker_rootless: rootless,
    }
}

#[test]
fn nothing_is_probed_when_the_hall_has_no_agents() {
    let boom = || -> bool { panic!("probed") };
    let none = || -> Option<bool> { panic!("probed") };
    assert!(diagnose_agents(false, &probes(&boom, &none)).is_empty());
}

#[test]
fn a_ready_rootless_machine_is_clean() {
    assert!(diagnose_agents(true, &probes(&|| true, &|| Some(true))).is_empty());
}

#[test]
fn every_missing_requirement_names_its_fix_and_a_missing_client_skips_the_daemon_probes() {
    let codes: Vec<_> = diagnose_agents(true, &probes(&|| false, &|| None))
        .into_iter()
        .map(|d| (d.code, d.fix))
        .collect();
    assert_eq!(
        codes.iter().map(|(c, _)| *c).collect::<Vec<_>>(),
        [
            "agent.systemd_user_missing",
            "agent.linger_disabled",
            "agent.docker_missing",
        ]
    );
    assert!(codes[1].1.contains("loginctl enable-linger"));
}

#[test]
fn a_present_client_with_a_dead_daemon_is_unreachable_only() {
    let down = || false;
    let up = || true;
    let p = Probes {
        systemd_user: &up,
        linger: &up,
        docker_client: &up,
        docker_daemon: &down,
        docker_rootless: &|| None,
    };
    let codes: Vec<_> = diagnose_agents(true, &p)
        .into_iter()
        .map(|d| d.code)
        .collect();
    assert_eq!(codes, ["agent.docker_unreachable"]);
}

#[test]
fn rootful_docker_is_reported_with_the_rootless_recommendation() {
    let findings = diagnose_agents(true, &probes(&|| true, &|| Some(false)));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, "agent.docker_rootful");
    assert!(findings[0].fix.contains("rootless"));
}
