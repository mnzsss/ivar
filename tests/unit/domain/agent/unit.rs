#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn service_runs_the_scheduled_trigger_from_the_hall() {
    let text = render_service(
        "1a2b3c4d",
        "nightly",
        "/home/me/hall",
        "/usr/bin/ivar",
        None,
    );
    assert_eq!(
        text,
        "[Unit]\nDescription=ivar agent nightly (/home/me/hall)\nX-Ivar-Owned=1a2b3c4d\n\n\
         [Service]\nType=oneshot\nTimeoutStartSec=infinity\nWorkingDirectory=/home/me/hall\n\
         ExecStart=/usr/bin/ivar agent run nightly --trigger schedule\n"
    );
    assert_eq!(exec_binary(&text), Some("/usr/bin/ivar"));
}

#[test]
fn service_pins_the_path_it_was_synced_with_so_a_boot_catchup_finds_the_tool() {
    let text = render_service(
        "1a2b3c4d",
        "nightly",
        "/home/me/hall",
        "/usr/bin/ivar",
        Some("/home/me/.local/bin:/usr/bin"),
    );
    assert!(
        text.contains("\nEnvironment=\"PATH=/home/me/.local/bin:/usr/bin\"\nExecStart="),
        "{text}"
    );
    assert_eq!(
        service_path(&text),
        Some("/home/me/.local/bin:/usr/bin".to_owned())
    );
}

#[test]
fn a_path_with_specifiers_or_quotes_is_escaped_and_reads_back_unchanged() {
    let path = r#"/opt/100%/bin:/a "b"\c"#;
    let text = render_service("1a2b3c4d", "nightly", "/h", "/usr/bin/ivar", Some(path));
    assert!(
        text.contains(r#"Environment="PATH=/opt/100%%/bin:/a \"b\"\\c""#),
        "{text}"
    );
    assert_eq!(service_path(&text), Some(path.to_owned()));
}

#[test]
fn timer_points_at_its_service_and_installs_into_timers_target() {
    let text = render_timer("1a2b3c4d", "nightly", "*-*-* 09:00:00", false);
    assert!(text.contains("\nOnCalendar=*-*-* 09:00:00\n"));
    assert!(text.contains("\nUnit=ivar-agent-1a2b3c4d-nightly.service\n"));
    assert!(text.contains("\n[Install]\nWantedBy=timers.target\n"));
    assert!(owned_by(&text, "1a2b3c4d"));
}

#[test]
fn timer_is_persistent_only_when_the_agent_asks_for_catchup() {
    let without = render_timer("1a2b3c4d", "nightly", "*-*-* 09:00:00", false);
    let with = render_timer("1a2b3c4d", "nightly", "*-*-* 09:00:00", true);
    assert!(!without.contains("Persistent="));
    assert!(with.contains("\n[Timer]\nOnCalendar=*-*-* 09:00:00\nPersistent=true\nUnit="));
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

#[test]
fn a_task_timer_is_one_shot_and_always_persistent() {
    let timer = render_once_timer("abcd1234", "recheck", "2026-11-08 12:00:00 UTC");
    assert!(timer.contains("OnCalendar=2026-11-08 12:00:00 UTC\n"));
    assert!(timer.contains("Persistent=true\n"));
    assert!(timer.contains("Unit=ivar-agent-abcd1234-recheck.service\n"));
    assert!(owned_by(&timer, "abcd1234"));
}
