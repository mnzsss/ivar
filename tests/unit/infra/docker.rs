#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::infra::proc::redirect_on_this_thread;
use crate::test_support::{FakeDocker, fake_docker_command, hall_root};

#[test]
fn rootless_reads_the_security_options() {
    let (_guard, root) = hall_root();
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread(
        "docker",
        fake_docker_command(&fake).env(
            "DOCKER_FAKE_SECURITY",
            "[\"name=seccomp\",\"name=rootless\"]",
        ),
    );
    assert_eq!(rootless(), Some(true));
    assert!(daemon_reachable());
}

#[test]
fn an_unreachable_daemon_has_no_rootless_answer() {
    let (_guard, root) = hall_root();
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread(
        "docker",
        fake_docker_command(&fake).env("DOCKER_FAKE_DOWN", "1"),
    );
    assert!(!daemon_reachable());
    assert_eq!(rootless(), None);
}

#[test]
fn creates_an_internal_network_and_removes_it() {
    let (_guard, root) = hall_root();
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread("docker", fake_docker_command(&fake));
    network_create_internal("ivar-agent-r1-net").unwrap();
    network_connect("bridge", "ivar-agent-r1-proxy").unwrap();
    network_remove("ivar-agent-r1-net");
    assert_eq!(
        fake.log(),
        "network create --internal ivar-agent-r1-net\n\
                            network connect bridge ivar-agent-r1-proxy\n\
                            network rm ivar-agent-r1-net\n"
    );
}
