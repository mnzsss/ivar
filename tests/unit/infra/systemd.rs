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
