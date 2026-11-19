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

#[test]
fn a_stamp_lives_in_the_user_data_dir_under_the_timer_unit_name() {
    assert_eq!(
        stamp_path(
            Utf8Path::new("/home/me/.local/share"),
            "ivar-agent-1a-x.timer"
        ),
        "/home/me/.local/share/systemd/timers/stamp-ivar-agent-1a-x.timer"
    );
}

fn mtime_secs(path: &Utf8Path) -> u64 {
    std::fs::metadata(path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn a_stamp_is_written_once_with_the_given_mtime_and_never_rewritten() {
    let (_guard, root) = hall_root();
    let data = root.join("data");
    let unit = "ivar-agent-1a-x.timer";
    assert!(!stamp_exists(&data, unit));

    assert!(write_stamp_if_missing(&data, unit, 1_793_000_000).unwrap());
    assert!(stamp_exists(&data, unit));
    let path = stamp_path(&data, unit);
    assert_eq!(mtime_secs(&path), 1_793_000_000);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);

    assert!(!write_stamp_if_missing(&data, unit, 1_794_000_000).unwrap());
    assert_eq!(mtime_secs(&path), 1_793_000_000);
}

#[test]
fn restart_targets_the_user_manager() {
    let (_guard, root) = hall_root();
    let fake = FakeSystemctl::install(&root);
    let _redirect = fake_systemctl_on_this_thread(&fake);
    restart("ivar-agent-1a-x.timer").unwrap();
    assert_eq!(fake.log(), "--user restart ivar-agent-1a-x.timer\n");
}
