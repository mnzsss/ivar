#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

/// The pid of a process that has exited and been reaped.
fn dead_pid() -> u32 {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

#[test]
fn this_process_is_alive_with_its_own_start_time() {
    let me = std::process::id();
    assert!(is_alive(me, None));
    assert!(is_alive(me, start_time(me).as_deref()));
}

#[test]
fn a_reaped_process_is_not_alive() {
    assert!(!is_alive(dead_pid(), None));
}

#[cfg(target_os = "linux")]
#[test]
fn a_pid_reused_by_another_process_is_not_the_recorded_one() {
    let me = std::process::id();
    assert!(start_time(me).is_some());
    assert!(!is_alive(me, Some("0")));
}
