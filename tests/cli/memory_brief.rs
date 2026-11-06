#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::time::{Duration, Instant};

use crate::common::{hall_root, ivar};

fn init_hall() -> (tempfile::TempDir, camino::Utf8PathBuf) {
    let (guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    (guard, root)
}

fn begin(root: &camino::Utf8Path, summary: &str) {
    ivar()
        .current_dir(root)
        .env_remove("IVAR_SESSION_ID")
        .args(["memory", "handoff", "begin", "--hall", "--summary", summary])
        .assert()
        .success();
}

fn brief(root: &camino::Utf8Path) -> (String, Duration) {
    let started = Instant::now();
    let out = ivar()
        .current_dir(root)
        .env_remove("IVAR_SESSION_ID")
        .args(["memory", "brief", "--provider", "claude-code"])
        .write_stdin(format!(
            r#"{{"hook_event_name":"SessionStart","session_id":"n1","cwd":"{root}"}}"#
        ))
        .assert()
        .success();
    (
        String::from_utf8(out.get_output().stdout.clone()).unwrap(),
        started.elapsed(),
    )
}

#[test]
fn an_open_handoff_appears_in_the_brief() {
    let (_guard, root) = init_hall();
    begin(&root, "continue the sweep task");
    let (out, _) = brief(&root);
    assert!(out.contains("continue the sweep task"), "{out}");
}

#[test]
fn the_brief_is_silent_without_a_hall_with_a_corrupt_database_and_with_a_bad_flag() {
    let dir = tempfile::tempdir().unwrap();
    ivar()
        .current_dir(dir.path())
        .args(["memory", "brief", "--provider", "claude-code"])
        .write_stdin("{}")
        .assert()
        .success()
        .stdout("");

    let (_guard, root) = init_hall();
    std::fs::write(root.join(".ivar/memory.db"), b"not a database").unwrap();
    let (out, _) = brief(&root);
    assert_eq!(out, "");

    ivar()
        .current_dir(&root)
        .args(["memory", "brief", "--bogus"])
        .write_stdin("{}")
        .assert()
        .success()
        .stdout("");
}

#[test]
#[ignore = "set IVAR_BRIEF_BENCH=1 and run with --profile e2e on a quiet machine"]
fn the_brief_p95_stays_inside_one_hundred_milliseconds_with_a_thousand_handoffs() {
    if std::env::var_os("IVAR_BRIEF_BENCH").is_none() {
        return;
    }
    let (_guard, root) = init_hall();
    for n in 0..1000 {
        begin(&root, &format!("handoff {n}"));
    }
    let mut samples: Vec<Duration> = (0..40).map(|_| brief(&root).1).collect();
    samples.sort();
    assert!(
        samples[37] < Duration::from_millis(100),
        "p95 {:?}",
        samples[37]
    );
}
