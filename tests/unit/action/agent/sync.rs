#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::store::layout::Layout;
use crate::test_support::{FakeSystemctl, fake_systemctl_on_this_thread, seeded_hall};

fn yes() -> bool {
    true
}
fn no() -> bool {
    false
}
fn rootless() -> Option<bool> {
    Some(true)
}

/// 2026-11-06T12:00:00Z, the moment every sync test runs at.
const NOW: i64 = 1_793_966_400;

/// A machine whose local time is UTC.
fn utc_wall(wall: &str) -> Option<i64> {
    let (date, time) = wall.split_once(' ')?;
    let [y, mo, d] = date
        .split('-')
        .map(|n| n.parse::<i64>().ok())
        .collect::<Option<Vec<_>>>()?[..]
    else {
        return None;
    };
    let [h, mi] = time
        .split(':')
        .map(|n| n.parse::<i64>().ok())
        .collect::<Option<Vec<_>>>()?[..]
    else {
        return None;
    };
    Some(crate::domain::session::epoch_secs_from_civil(
        y, mo, d, h, mi,
    ))
}

fn env(unit_dir: Utf8PathBuf) -> SyncEnv<'static> {
    SyncEnv {
        unit_dir,
        ivar_bin: Utf8PathBuf::from("/opt/ivar/bin/ivar"),
        path: Some("/home/me/.local/bin:/usr/bin".to_owned()),
        is_linux: true,
        probes: Probes {
            systemd_user: &yes,
            linger: &yes,
            docker_client: &yes,
            docker_daemon: &yes,
            docker_rootless: &rootless,
            tool_on_path: &|_| true,
            landlock: &yes,
        },
        clock: Clock {
            now_secs: NOW,
            local_epoch_secs: &utc_wall,
        },
    }
}

fn agent(root: &Utf8PathBuf, name: &str, schedule: &str) {
    let dir = root.join("agents");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{name}.md")),
        format!("---\nschedule: \"{schedule}\"\ntool: claude\n---\nGo.\n"),
    )
    .unwrap();
}

#[test]
fn creates_updates_and_removes_only_owned_units() {
    let (_guard, root) = seeded_hall();
    let units = root.join("units");
    std::fs::create_dir_all(&units).unwrap();
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    let ctx = Ctx::new(root.clone());
    let hash = hall_hash(&Layout::at(root.clone()));

    agent(&root, "keep", "@daily");
    agent(&root, "gone", "@hourly");
    let first = sync_with(&ctx, &env(units.clone())).unwrap().value;
    assert_eq!(first.created, ["gone", "keep"]);
    let timer =
        std::fs::read_to_string(units.join(format!("ivar-agent-{hash}-keep.timer"))).unwrap();
    assert!(timer.contains("OnCalendar=*-*-* 00:00:00"));

    std::fs::write(
        units.join(format!("ivar-agent-{hash}-foreign.timer")),
        "[Unit]\nDescription=mine\n",
    )
    .unwrap();
    std::fs::remove_file(root.join("agents/gone.md")).unwrap();
    agent(&root, "keep", "@weekly");
    let second = sync_with(&ctx, &env(units.clone())).unwrap().value;
    assert_eq!(second.updated, ["keep"]);
    assert_eq!(second.removed, ["gone"]);
    assert!(
        units
            .join(format!("ivar-agent-{hash}-foreign.timer"))
            .exists()
    );
    assert!(
        !units
            .join(format!("ivar-agent-{hash}-gone.service"))
            .exists()
    );
    assert!(fake.log().contains(&format!(
        "--user disable --now ivar-agent-{hash}-gone.timer"
    )));
    assert!(
        fake.log()
            .contains(&format!("--user enable --now ivar-agent-{hash}-keep.timer"))
    );

    let third = sync_with(&ctx, &env(units)).unwrap().value;
    assert_eq!(third.unchanged, ["keep"]);
}

#[test]
fn an_invalid_file_schedules_nothing() {
    let (_guard, root) = seeded_hall();
    let units = root.join("units");
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    agent(&root, "good", "@daily");
    agent(&root, "bad", "0 0 1 * 1");
    let failure = sync_with(&Ctx::new(root.clone()), &env(units.clone())).unwrap_err();
    assert_eq!(failure.code, "agent.invalid");
    assert!(failure.what.contains("agents/bad.md"));
    assert!(!units.exists() || std::fs::read_dir(&units).unwrap().next().is_none());
    assert_eq!(fake.log(), "");
}

#[test]
fn refuses_off_linux_with_one_message() {
    let (_guard, root) = seeded_hall();
    let mut e = env(root.join("units"));
    e.is_linux = false;
    let failure = sync_with(&Ctx::new(root), &e).unwrap_err();
    assert_eq!(failure.code, "agent.unsupported_platform");
    assert!(failure.what.contains("Linux with systemd"));
}

#[test]
fn missing_requirements_are_warnings_not_failures() {
    let (_guard, root) = seeded_hall();
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    agent(&root, "keep", "@daily");
    let mut e = env(root.join("units"));
    e.probes.linger = &no;
    let report = sync_with(&Ctx::new(root), &e).unwrap();
    assert_eq!(
        report.warnings.iter().map(|w| w.code).collect::<Vec<_>>(),
        ["agent.linger_disabled"]
    );
}

#[test]
fn a_host_agent_syncs_with_a_warning_about_ambient_reads() {
    let (_guard, root) = seeded_hall();
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(
        root.join("agents/local.md"),
        "---\nschedule: \"@daily\"\nruntime: host\ntool: claude\n---\nGo.\n",
    )
    .unwrap();
    let report = sync_with(&Ctx::new(root.clone()), &env(root.join("units"))).unwrap();
    let warning = report
        .warnings
        .iter()
        .find(|w| w.code == "agent.host_ambient_read")
        .expect("host warning");
    assert!(warning.what.contains("`local`"));
    assert!(warning.what.contains("SSH keys"));
}

fn host_agent(root: &Utf8PathBuf, name: &str, write: &str) {
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(
        root.join(format!("agents/{name}.md")),
        format!(
            "---\nschedule: \"@daily\"\nruntime: host\ntool: claude\nwrite: {write}\n---\nGo.\n"
        ),
    )
    .unwrap();
}

#[test]
fn a_host_agent_that_writes_proc_syncs_with_a_warning() {
    let (_guard, root) = seeded_hall();
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    host_agent(&root, "chrome", "[/proc]");
    host_agent(&root, "plain", "[/srv/process]");
    let report = sync_with(&Ctx::new(root.clone()), &env(root.join("units"))).unwrap();
    let proc_warnings: Vec<_> = report
        .warnings
        .iter()
        .filter(|w| w.code == "agent.host_write_proc")
        .collect();
    let [warning] = proc_warnings.as_slice() else {
        panic!("{proc_warnings:?}");
    };
    assert!(warning.what.contains("`chrome`"));
    assert!(warning.what.contains("ptrace"));
}

fn task(root: &Utf8PathBuf, name: &str, at: &str) {
    let dir = root.join("agents");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{name}.md")),
        format!("---\nat: {at}\ntool: claude\n---\nLook again.\n"),
    )
    .unwrap();
}

fn timer_of(units: &Utf8PathBuf, root: &Utf8PathBuf, name: &str) -> String {
    let hash = hall_hash(&Layout::at(root.clone()));
    std::fs::read_to_string(units.join(format!("ivar-agent-{hash}-{name}.timer"))).unwrap()
}

#[test]
fn a_future_task_gets_a_one_shot_persistent_timer_and_waits() {
    let (_guard, root) = seeded_hall();
    let units = root.join("units");
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    task(&root, "local", "2026-11-08T09:00");
    task(&root, "offset", "2026-11-08T09:00-03:00");

    let outcome = sync_with(&Ctx::new(root.clone()), &env(units.clone()))
        .unwrap()
        .value;
    assert_eq!(outcome.created, ["local", "offset"]);
    assert!(outcome.fired.is_empty());

    let local = timer_of(&units, &root, "local");
    assert!(
        local.contains("OnCalendar=2026-11-08 09:00:00\n"),
        "{local}"
    );
    assert!(local.contains("Persistent=true\n"), "{local}");
    let offset = timer_of(&units, &root, "offset");
    assert!(
        offset.contains("OnCalendar=2026-11-08 12:00:00 UTC\n"),
        "{offset}"
    );
    assert!(offset.contains("Persistent=true\n"), "{offset}");
    assert!(!fake.log().contains("start"), "{}", fake.log());
}

#[test]
fn a_past_task_fires_immediately_as_a_timer_run_and_says_so() {
    let (_guard, root) = seeded_hall();
    let units = root.join("units");
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    let hash = hall_hash(&Layout::at(root.clone()));
    task(&root, "overdue", "2026-11-05T09:00");
    task(&root, "overdue-offset", "2026-11-06T08:00-03:00");
    task(&root, "later", "2026-11-06T12:01");

    let report = sync_with(&Ctx::new(root.clone()), &env(units.clone())).unwrap();
    let outcome = report.value;
    assert_eq!(outcome.fired, ["overdue-offset", "overdue"]);
    let log = fake.log();
    assert!(log.contains(&format!(
        "--user start --no-block ivar-agent-{hash}-overdue.service"
    )));
    assert!(!log.contains("later.service"), "{log}");
    let service =
        std::fs::read_to_string(units.join(format!("ivar-agent-{hash}-overdue.service"))).unwrap();
    assert!(
        service.contains("agent run overdue --trigger schedule"),
        "{service}"
    );
    let mut human = Vec::new();
    outcome.write_human(&mut human).unwrap();
    let human = String::from_utf8(human).unwrap();
    assert!(human.contains("fired now"), "{human}");
    assert!(human.contains("overdue-offset, overdue"), "{human}");
}

#[test]
fn a_task_due_exactly_now_is_fired() {
    let (_guard, root) = seeded_hall();
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    task(&root, "now", "2026-11-06T12:00");
    let outcome = sync_with(&Ctx::new(root.clone()), &env(root.join("units")))
        .unwrap()
        .value;
    assert_eq!(outcome.fired, ["now"]);
}

#[test]
fn done_tasks_are_no_source_and_their_timers_are_removed() {
    let (_guard, root) = seeded_hall();
    let units = root.join("units");
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    let ctx = Ctx::new(root.clone());
    let hash = hall_hash(&Layout::at(root.clone()));
    task(&root, "recheck", "2026-11-08T09:00");
    agent(&root, "keep", "@daily");
    sync_with(&ctx, &env(units.clone())).unwrap();

    let done = root.join("agents/done");
    std::fs::create_dir_all(&done).unwrap();
    std::fs::rename(
        root.join("agents/recheck.md"),
        done.join("recheck.2026-11-08T0900.md"),
    )
    .unwrap();
    let second = sync_with(&ctx, &env(units.clone())).unwrap().value;

    assert_eq!(second.removed, ["recheck"]);
    assert_eq!(second.unchanged, ["keep"]);
    assert!(second.fired.is_empty());
    assert!(
        !units
            .join(format!("ivar-agent-{hash}-recheck.timer"))
            .exists()
    );
    assert!(
        !units
            .join(format!("ivar-agent-{hash}-recheck.service"))
            .exists()
    );
    assert!(fake.log().contains(&format!(
        "--user disable --now ivar-agent-{hash}-recheck.timer"
    )));
}

#[test]
fn a_file_in_agents_done_that_would_not_validate_is_ignored() {
    let (_guard, root) = seeded_hall();
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    agent(&root, "keep", "@daily");
    std::fs::create_dir_all(root.join("agents/done")).unwrap();
    std::fs::write(root.join("agents/done/broken.md"), "not frontmatter").unwrap();
    let outcome = sync_with(&Ctx::new(root.clone()), &env(root.join("units")))
        .unwrap()
        .value;
    assert_eq!(outcome.created, ["keep"]);
}

#[test]
fn an_unplaceable_local_time_is_not_fired_by_guesswork() {
    let (_guard, root) = seeded_hall();
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    task(&root, "gap", "2026-03-08T02:30");
    let mut e = env(root.join("units"));
    e.clock.local_epoch_secs = &|_| None;
    let outcome = sync_with(&Ctx::new(root.clone()), &e).unwrap().value;
    assert!(outcome.fired.is_empty());
}

#[test]
fn a_due_task_whose_run_is_still_going_is_not_started_again() {
    let (_guard, root) = seeded_hall();
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    task(&root, "busy", "2026-11-05T09:00");
    task(&root, "idle", "2026-11-05T09:00");
    let _held = crate::infra::fs::try_lock_exclusive(&Layout::at(root.clone()).agent_lock("busy"))
        .unwrap()
        .unwrap();

    let outcome = sync_with(&Ctx::new(root.clone()), &env(root.join("units")))
        .unwrap()
        .value;

    assert_eq!(outcome.fired, ["idle"]);
    assert!(!fake.log().contains("busy.service"), "{}", fake.log());
}

#[test]
fn the_service_carries_the_path_sync_ran_with() {
    let (_guard, root) = seeded_hall();
    let units = root.join("units");
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    agent(&root, "nightly", "0 9 * * *");

    sync_with(&Ctx::new(root.clone()), &env(units.clone())).unwrap();

    let service = std::fs::read_to_string(units.join(format!(
        "{}.service",
        unit_stem(&hall_hash(&Layout::at(root.clone())), "nightly")
    )))
    .unwrap();
    assert!(
        service.contains("Environment=\"PATH=/home/me/.local/bin:/usr/bin\""),
        "{service}"
    );
}

#[test]
fn a_binary_inside_a_cargo_target_dir_is_a_warning() {
    let (_guard, root) = seeded_hall();
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    agent(&root, "nightly", "0 9 * * *");
    let ctx = Ctx::new(root.clone());
    let codes = |bin: &str| {
        let env = SyncEnv {
            ivar_bin: Utf8PathBuf::from(bin),
            ..env(root.join("units"))
        };
        sync_with(&ctx, &env)
            .unwrap()
            .warnings
            .iter()
            .map(|w| w.code)
            .collect::<Vec<_>>()
    };

    assert!(codes("/src/ivar/target/debug/ivar").contains(&"agent.ivar_bin_in_build_dir"));
    assert!(codes("/src/ivar/target/release/ivar").contains(&"agent.ivar_bin_in_build_dir"));
    assert!(!codes("/home/me/.local/bin/ivar").contains(&"agent.ivar_bin_in_build_dir"));
}
