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
