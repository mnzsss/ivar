#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn service_runs_the_scheduled_trigger_from_the_hall() {
    let text = render_service("1a2b3c4d", "nightly", "/home/me/hall", "/usr/bin/ivar");
    assert_eq!(
        text,
        "[Unit]\nDescription=ivar agent nightly (/home/me/hall)\nX-Ivar-Owned=1a2b3c4d\n\n\
         [Service]\nType=oneshot\nTimeoutStartSec=infinity\nWorkingDirectory=/home/me/hall\n\
         ExecStart=/usr/bin/ivar agent run nightly --trigger schedule\n"
    );
    assert_eq!(exec_binary(&text), Some("/usr/bin/ivar"));
}

#[test]
fn timer_points_at_its_service_and_installs_into_timers_target() {
    let text = render_timer("1a2b3c4d", "nightly", "*-*-* 09:00:00");
    assert!(text.contains("\nOnCalendar=*-*-* 09:00:00\n"));
    assert!(text.contains("\nUnit=ivar-agent-1a2b3c4d-nightly.service\n"));
    assert!(text.contains("\n[Install]\nWantedBy=timers.target\n"));
    assert!(owned_by(&text, "1a2b3c4d"));
}

#[test]
fn ownership_needs_this_halls_exact_marker() {
    assert!(!owned_by("[Unit]\nX-Ivar-Owned=ffffffff\n", "1a2b3c4d"));
    assert!(!owned_by(
        "[Unit]\nDescription=X-Ivar-Owned=1a2b3c4d\n",
        "1a2b3c4d"
    ));
    assert_eq!(
        unit_stem("1a2b3c4d", "nightly"),
        "ivar-agent-1a2b3c4d-nightly"
    );
}
