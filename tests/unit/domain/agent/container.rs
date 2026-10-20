#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn plan() -> ContainerPlan {
    ContainerPlan {
        name: "ivar-agent-r1".into(),
        network: "ivar-agent-r1-net".into(),
        image: "img@sha256:abc".into(),
        env: vec![("OPENCODE_CONFIG_CONTENT".into(), "{}".into())],
        env_file: "/hall/.ivar/agents/tmp/r1/secrets.env".into(),
        mounts: vec![Mount {
            source: "/hall/.ivar/repos".into(),
            target: "/workspace".into(),
            read_only: true,
        }],
        workdir: "/workspace/ivar/main".into(),
        argv: vec!["claude".into(), "-p".into(), "go".into()],
    }
}

#[test]
fn agent_runs_unprivileged_behind_the_proxy_with_stdin_closed() {
    let args = agent_run_args(&plan());
    let joined = args.join(" ");
    for required in [
        "run --rm --name ivar-agent-r1 --network ivar-agent-r1-net",
        "--user 1000:1000",
        "--cap-drop ALL",
        "--security-opt no-new-privileges",
        "--env-file /hall/.ivar/agents/tmp/r1/secrets.env",
        "-e HTTPS_PROXY=http://proxy:8888",
        "-e HTTP_PROXY=http://proxy:8888",
        "--tmpfs /home/agent",
        "-e OPENCODE_CONFIG_CONTENT={}",
        "-v /hall/.ivar/repos:/workspace:ro",
        "-w /workspace/ivar/main img@sha256:abc claude -p go",
    ] {
        assert!(
            joined.contains(required),
            "missing `{required}` in `{joined}`"
        );
    }
    assert!(
        !args
            .iter()
            .any(|a| a == "-i" || a == "-t" || a.contains("docker.sock"))
    );
    assert!(args.ends_with(&["claude".to_owned(), "-p".to_owned(), "go".to_owned()]));
}

#[test]
fn a_writable_mount_has_no_ro_suffix() {
    let mut p = plan();
    p.mounts = vec![Mount {
        source: "/wt".into(),
        target: "/workspace/ivar".into(),
        read_only: false,
    }];
    assert!(agent_run_args(&p).contains(&"/wt:/workspace/ivar".to_owned()));
}

#[test]
fn proxy_joins_the_internal_network_under_its_alias_and_reaches_the_host() {
    let joined = proxy_run_args(
        "ivar-agent-r1-proxy",
        "ivar-agent-r1-net",
        "/cfg",
        "proxy@sha256:def",
    )
    .join(" ");
    assert_eq!(
        joined,
        "run -d --rm --name ivar-agent-r1-proxy --network ivar-agent-r1-net --network-alias proxy \
         --add-host=host.docker.internal:host-gateway --user 65534:65534 --cap-drop ALL \
         --security-opt no-new-privileges -v /cfg:/etc/tinyproxy:ro proxy@sha256:def"
    );
}

#[test]
fn filter_denies_by_default_and_anchors_each_domain() {
    assert!(tinyproxy_conf().contains("\nFilterDefaultDeny Yes\n"));
    assert!(tinyproxy_conf().starts_with("Port 8888\n"));
    assert!(!tinyproxy_conf().contains("User ") && !tinyproxy_conf().contains("Group "));
    assert!(tinyproxy_conf().contains("\nFilter \"/etc/tinyproxy/filter\"\n"));
    assert_eq!(
        tinyproxy_filter(&["api.anthropic.com".into(), "host.docker.internal".into()]),
        "^api\\.anthropic\\.com$\n^host\\.docker\\.internal$\n"
    );
}
