#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::test_support::{FakeSystemctl, fake_systemctl_on_this_thread, hall_root};

#[test]
fn enable_now_targets_the_user_manager() {
    let (_guard, root) = hall_root();
    let fake = FakeSystemctl::install(&root);
    let _redirect = fake_systemctl_on_this_thread(&fake);
    enable_now("ivar-agent-1a2b3c4d-nightly.timer").unwrap();
    daemon_reload().unwrap();
    assert_eq!(
        fake.log(),
        "--user enable --now ivar-agent-1a2b3c4d-nightly.timer\n--user daemon-reload\n"
    );
}

#[test]
fn next_elapse_is_none_when_systemd_has_no_answer() {
    let (_guard, root) = hall_root();
    let fake = FakeSystemctl::install(&root);
    let _redirect = fake_systemctl_on_this_thread(&fake);
    assert_eq!(next_elapse("ivar-agent-x-y.timer"), None);
}

#[test]
fn user_path_is_the_managers_path_and_none_when_it_does_not_answer() {
    let (_guard, root) = hall_root();
    let fake = FakeSystemctl::install(&root);
    let answered = crate::infra::proc::redirect_on_this_thread(
        "systemctl",
        crate::test_support::fake_systemctl_command(&fake)
            .env("SYSTEMCTL_FAKE_PATH", "/u/bin:/bin"),
    );
    assert_eq!(user_path().as_deref(), Some("/u/bin:/bin"));
    drop(answered);
    let _down = crate::infra::proc::redirect_on_this_thread(
        "systemctl",
        crate::test_support::fake_systemctl_command(&fake).env("SYSTEMCTL_FAKE_DOWN", "1"),
    );
    assert_eq!(user_path(), None);
}
