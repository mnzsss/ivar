#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::agent::spec::{AgentFrontmatter, AgentSpec, Runtime};
use crate::domain::agent::tool::Tool;

fn agent(runtime: Runtime, tool: &str) -> AgentSpec {
    AgentSpec::from_parts(
        "a",
        AgentFrontmatter {
            schedule: Some("@daily".into()),
            tool: Some(tool.into()),
            runtime,
            ..AgentFrontmatter::default()
        },
        "",
    )
    .unwrap()
}

fn docker() -> Vec<AgentSpec> {
    vec![agent(Runtime::Docker, "claude")]
}

fn probes<'a>(ok: &'a dyn Fn() -> bool, rootless: &'a dyn Fn() -> Option<bool>) -> Probes<'a> {
    Probes {
        systemd_user: ok,
        linger: ok,
        docker_client: ok,
        docker_daemon: ok,
        docker_rootless: rootless,
        tool_on_path: &|_| true,
        landlock: &|| true,
    }
}

fn yes() -> bool {
    true
}

#[test]
fn nothing_is_probed_when_the_hall_has_no_agents() {
    let boom = || -> bool { panic!("probed") };
    let none = || -> Option<bool> { panic!("probed") };
    assert!(diagnose_agents(&[], &probes(&boom, &none)).is_empty());
}

#[test]
fn a_ready_rootless_machine_is_clean() {
    assert!(diagnose_agents(&docker(), &probes(&|| true, &|| Some(true))).is_empty());
}

#[test]
fn every_missing_requirement_names_its_fix_and_a_missing_client_skips_the_daemon_probes() {
    let codes: Vec<_> = diagnose_agents(&docker(), &probes(&|| false, &|| None))
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
        tool_on_path: &|_| true,
        landlock: &|| true,
    };
    let codes: Vec<_> = diagnose_agents(&docker(), &p)
        .into_iter()
        .map(|d| d.code)
        .collect();
    assert_eq!(codes, ["agent.docker_unreachable"]);
}

#[test]
fn rootful_docker_is_reported_with_the_rootless_recommendation() {
    let findings = diagnose_agents(&docker(), &probes(&|| true, &|| Some(false)));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, "agent.docker_rootful");
    assert!(findings[0].fix.contains("rootless"));
}

#[test]
fn host_agents_skip_docker_and_report_their_tool_and_landlock() {
    let boom = || -> bool { panic!("docker probed") };
    let p = Probes {
        systemd_user: &yes,
        linger: &yes,
        docker_client: &boom,
        docker_daemon: &boom,
        docker_rootless: &|| panic!("docker probed"),
        tool_on_path: &|tool| tool != Tool::Codex,
        landlock: &|| false,
    };
    let specs = [
        agent(Runtime::Host, "codex"),
        agent(Runtime::Host, "codex"),
        agent(Runtime::Host, "claude"),
    ];
    let findings = diagnose_agents(&specs, &p);
    assert_eq!(
        findings.iter().map(|d| d.code).collect::<Vec<_>>(),
        ["agent.tool_missing", "agent.landlock_unavailable"]
    );
    assert!(findings[0].what.contains("`codex`"));
}

#[cfg(unix)]
#[test]
fn a_tool_is_found_only_as_an_executable_file_on_path() {
    use std::os::unix::fs::PermissionsExt;
    let (_guard, dir) = crate::test_support::utf8_temp_dir();
    let bin = dir.join("bin");
    std::fs::create_dir_all(bin.join("codex")).unwrap();
    std::fs::write(bin.join("claude"), "").unwrap();
    std::fs::write(bin.join("opencode"), "").unwrap();
    std::fs::set_permissions(bin.join("opencode"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::join_paths([dir.join("empty"), bin]).unwrap();
    assert!(on_path("opencode", &path));
    assert!(!on_path("claude", &path));
    assert!(!on_path("codex", &path));
    assert!(!on_path("missing", &path));
}

#[test]
fn agent_files_that_fail_to_parse_still_count_as_agents() {
    let codes: Vec<_> = diagnose_agents_present(&[], true, &probes(&|| false, &|| None))
        .into_iter()
        .map(|d| d.code)
        .collect();
    assert!(codes.contains(&"agent.systemd_user_missing"), "{codes:?}");
}

#[test]
fn a_tool_is_looked_up_on_the_systemd_managers_path() {
    let (_guard, root) = crate::test_support::hall_root();
    let bin = root.join("manager-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let claude = bin.join("claude");
    std::fs::write(&claude, "#!/bin/sh\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let fake = crate::test_support::FakeSystemctl::install(&root);
    let _r = crate::infra::proc::redirect_on_this_thread(
        "systemctl",
        crate::test_support::fake_systemctl_command(&fake).env("SYSTEMCTL_FAKE_PATH", bin.as_str()),
    );
    assert!(tool_present(Tool::Claude));
    assert!(!tool_present(Tool::Codex), "the process PATH was used");
}
