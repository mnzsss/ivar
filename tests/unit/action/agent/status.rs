#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::agent::requirements::Probes;
use crate::action::agent::sync::{Clock, SyncEnv, sync_with};
use crate::domain::agent::receipt::{AgentRunReceipt, Trigger};
use crate::store::layout::Layout;
use crate::test_support::{FakeSystemctl, fake_systemctl_command, seeded_hall};

fn yes() -> bool {
    true
}
fn rootless() -> Option<bool> {
    Some(true)
}

#[test]
fn reports_schedule_next_run_last_outcome_and_drift() {
    let (_guard, root) = seeded_hall();
    let units = root.join("units");
    let bin = root.join("ivar-bin");
    std::fs::write(&bin, "").unwrap();
    let fake = FakeSystemctl::install(&root);
    let _r = crate::infra::proc::redirect_on_this_thread(
        "systemctl",
        fake_systemctl_command(&fake).env("SYSTEMCTL_FAKE_NEXT", "Tue 2026-10-20 09:00:00 UTC"),
    );
    std::fs::create_dir_all(root.join("agents")).unwrap();
    for name in ["alpha", "beta"] {
        std::fs::write(
            root.join(format!("agents/{name}.md")),
            "---\nschedule: \"0 9 * * *\"\ntool: codex\n---\n",
        )
        .unwrap();
    }
    let ctx = Ctx::new(root.clone());
    sync_with(
        &ctx,
        &SyncEnv {
            unit_dir: units.clone(),
            data_dir: root.join("data"),
            ivar_bin: bin.clone(),
            path: Some("/a/shell/that/synced:/usr/bin".to_owned()),
            is_linux: true,
            probes: Probes {
                is_linux: true,
                systemd_user: &yes,
                linger: &yes,
                docker_client: &yes,
                docker_daemon: &yes,
                docker_rootless: &rootless,
                tool_on_path: &|_| true,
                landlock: &yes,
            },
            clock: Clock {
                now_secs: 0,
                local_epoch_secs: &|_| None,
                local_wall: &|_| None,
            },
        },
    )
    .unwrap();
    std::fs::write(
        root.join("agents/beta.md"),
        "---\nschedule: \"@daily\"\ntool: codex\n---\n",
    )
    .unwrap();
    std::fs::write(
        root.join("agents/gamma.md"),
        "---\nschedule: \"@reboot\"\ntool: codex\n---\n",
    )
    .unwrap();
    std::fs::write(
        root.join("agents/delta.md"),
        "---\nschedule: \"@daily\"\ntool: codex\n---\n",
    )
    .unwrap();
    let layout = Layout::at(root.clone());
    crate::store::agent::write_receipt(
        &layout,
        &AgentRunReceipt::skipped(
            "20261019T090000Z-aaaaaaaa".into(),
            "alpha".into(),
            Trigger::Schedule,
            "t".into(),
        ),
    )
    .unwrap();

    let agents = status_in(&ctx, &units, &root.join("data"), &bin)
        .unwrap()
        .value
        .agents;
    let by = |n: &str| agents.iter().find(|a| a.name == n).unwrap().clone();
    assert_eq!(by("alpha").sync, UnitSync::InSync);
    assert_eq!(
        by("alpha").next_run.as_deref(),
        Some("Tue 2026-10-20 09:00:00 UTC")
    );
    assert_eq!(
        by("alpha").last_run_id.as_deref(),
        Some("20261019T090000Z-aaaaaaaa")
    );
    assert_eq!(by("beta").sync, UnitSync::Stale);
    assert_eq!(by("delta").sync, UnitSync::NotInstalled);
    assert!(
        matches!(by("gamma").sync, UnitSync::Invalid { ref reason } if reason.contains("@reboot"))
    );

    std::fs::remove_file(&bin).unwrap();
    let agents = status_in(&ctx, &units, &root.join("data"), &bin)
        .unwrap()
        .value
        .agents;
    assert!(matches!(
        agents.iter().find(|a| a.name == "alpha").unwrap().sync,
        UnitSync::BinaryMissing { .. }
    ));
}

#[test]
fn lists_a_pending_task_with_its_resolved_time_and_never_a_concluded_one() {
    let (_guard, root) = seeded_hall();
    let units = root.join("units");
    let bin = root.join("ivar-bin");
    std::fs::write(&bin, "").unwrap();
    let fake = FakeSystemctl::install(&root);
    let _r =
        crate::infra::proc::redirect_on_this_thread("systemctl", fake_systemctl_command(&fake));
    std::fs::create_dir_all(root.join("agents/done")).unwrap();
    std::fs::write(
        root.join("agents/recheck.md"),
        "---\nat: 2026-11-08T09:00-03:00\ntool: codex\n---\n",
    )
    .unwrap();
    std::fs::write(
        root.join("agents/done/old.2026-11-01T0900.md"),
        "---\nat: 2026-11-01T09:00\ntool: codex\n---\n",
    )
    .unwrap();

    let ctx = Ctx::new(root.clone());
    let status = status_in(&ctx, &units, &root.join("data"), &bin)
        .unwrap()
        .value;
    let [task] = status.agents.as_slice() else {
        panic!("{:?}", status.agents);
    };
    assert_eq!(task.name, "recheck");
    assert_eq!(task.schedule, None);
    assert_eq!(task.at.as_deref(), Some("2026-11-08T09:00-03:00"));
    assert_eq!(task.on_calendar.as_deref(), Some("2026-11-08 12:00:00 UTC"));
    assert_eq!(task.sync, UnitSync::NotInstalled);

    let mut human = Vec::new();
    status.write_human(&mut human).unwrap();
    let human = String::from_utf8(human).unwrap();
    assert!(human.contains("at 2026-11-08T09:00-03:00"), "{human}");
    assert!(human.contains("2026-11-08 12:00:00 UTC"), "{human}");
}

fn hall_with_alpha() -> (tempfile::TempDir, camino::Utf8PathBuf) {
    let (guard, root) = seeded_hall();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(
        root.join("agents/alpha.md"),
        "---\nschedule: \"0 9 * * *\"\ntool: codex\n---\n",
    )
    .unwrap();
    (guard, root)
}

fn alpha_run(root: &camino::Utf8PathBuf) -> RunState {
    let ctx = Ctx::new(root.clone());
    let status = status_in(
        &ctx,
        &root.join("units"),
        &root.join("data"),
        &root.join("ivar-bin"),
    )
    .unwrap();
    status.value.agents.into_iter().next().unwrap().run
}

fn record(root: &camino::Utf8PathBuf, pid: u32, pid_start: Option<String>) {
    crate::store::agent::write_current_run(
        &Layout::at(root.clone()),
        "alpha",
        &crate::domain::agent::current::CurrentRun {
            run_id: "20261118T090000Z-aaaaaaaa".into(),
            trigger: Trigger::Schedule,
            started_at: "2026-11-18T09:00:00Z".into(),
            pid,
            pid_start,
        },
    )
    .unwrap();
}

#[test]
fn an_agent_with_no_run_going_is_idle() {
    let (_guard, root) = hall_with_alpha();
    assert_eq!(alpha_run(&root), RunState::Idle);
}

#[test]
fn a_run_in_progress_shows_as_running_with_its_start() {
    let (_guard, root) = hall_with_alpha();
    let me = std::process::id();
    record(&root, me, crate::infra::proc::start_time(me));
    let expected = RunState::Running {
        run_id: "20261118T090000Z-aaaaaaaa".into(),
        trigger: Trigger::Schedule,
        started_at: "2026-11-18T09:00:00Z".into(),
    };
    assert_eq!(alpha_run(&root), expected);

    let ctx = Ctx::new(root.clone());
    let status = status_in(
        &ctx,
        &root.join("units"),
        &root.join("data"),
        &root.join("ivar-bin"),
    )
    .unwrap()
    .value;
    let mut human = Vec::new();
    status.write_human(&mut human).unwrap();
    let human = String::from_utf8(human).unwrap();
    assert!(
        human.contains("running since 2026-11-18T09:00:00Z"),
        "{human}"
    );
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(
        json["agents"][0]["run"],
        serde_json::json!({
            "state": "running",
            "run_id": "20261118T090000Z-aaaaaaaa",
            "trigger": "schedule",
            "started_at": "2026-11-18T09:00:00Z",
        })
    );
}

#[test]
fn a_finished_run_is_idle_and_shows_a_dash() {
    let (_guard, root) = hall_with_alpha();
    let me = std::process::id();
    record(&root, me, crate::infra::proc::start_time(me));
    crate::store::agent::remove_current_run(
        &Layout::at(root.clone()),
        "alpha",
        "20261118T090000Z-aaaaaaaa",
    )
    .unwrap();
    assert_eq!(alpha_run(&root), RunState::Idle);

    let ctx = Ctx::new(root.clone());
    let status = status_in(
        &ctx,
        &root.join("units"),
        &root.join("data"),
        &root.join("ivar-bin"),
    )
    .unwrap()
    .value;
    assert_eq!(
        serde_json::to_value(&status).unwrap()["agents"][0]["run"],
        serde_json::json!({"state": "idle"})
    );
}

#[test]
fn a_record_left_by_a_dead_process_is_not_running() {
    let (_guard, root) = hall_with_alpha();
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    record(&root, pid, None);
    assert_eq!(alpha_run(&root), RunState::Idle);
}

#[cfg(target_os = "linux")]
#[test]
fn a_record_whose_pid_now_belongs_to_another_process_is_not_running() {
    let (_guard, root) = hall_with_alpha();
    record(&root, std::process::id(), Some("0".into()));
    assert_eq!(alpha_run(&root), RunState::Idle);
}

#[test]
fn status_neither_takes_nor_waits_for_the_agents_lock() {
    let (_guard, root) = hall_with_alpha();
    let layout = Layout::at(root.clone());
    let me = std::process::id();
    record(&root, me, crate::infra::proc::start_time(me));
    // A run holds the lock for as long as it goes on; status reports it
    // without waiting for it.
    let held = crate::infra::fs::try_lock_exclusive(&layout.agent_lock("alpha"))
        .unwrap()
        .unwrap();
    assert!(matches!(alpha_run(&root), RunState::Running { .. }));
    drop(held);
    // And it leaves no hold of its own behind.
    assert!(
        crate::infra::fs::try_lock_exclusive(&layout.agent_lock("alpha"))
            .unwrap()
            .is_some()
    );
}

#[test]
fn a_run_that_starts_while_status_is_queried_is_never_skipped() {
    use crate::action::agent::run::{RunInput, run};
    use crate::domain::agent::receipt::AgentRunOutcome;
    use std::sync::atomic::{AtomicBool, Ordering};

    let (_guard, root) = hall_with_alpha();
    // No secret `NEEDED` exists, so each run takes the lock, fails and
    // releases it, with no container started.
    std::fs::write(
        root.join("agents/alpha.md"),
        "---\nschedule: \"0 9 * * *\"\ntool: codex\nsecrets: [NEEDED]\n---\n",
    )
    .unwrap();
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let querier = scope.spawn(|| {
            let mut queries = 0_u32;
            while !stop.load(Ordering::Relaxed) {
                alpha_run(&root);
                queries += 1;
            }
            queries
        });
        for _ in 0..25 {
            let receipt = run(
                &Ctx::new(root.clone()),
                RunInput {
                    name: "alpha".into(),
                    trigger: Trigger::Manual,
                },
            )
            .unwrap()
            .value;
            assert_eq!(receipt.outcome, AgentRunOutcome::Failed, "{receipt:?}");
        }
        stop.store(true, Ordering::Relaxed);
        assert!(querier.join().unwrap() > 0);
    });
    assert_eq!(alpha_run(&root), RunState::Idle);
}

fn status_after_sync(
    root: &camino::Utf8PathBuf,
    catchup: bool,
) -> (Ctx, camino::Utf8PathBuf, camino::Utf8PathBuf) {
    let units = root.join("units");
    let bin = root.join("ivar-bin");
    std::fs::write(&bin, "").unwrap();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(
        root.join("agents/alpha.md"),
        format!("---\nschedule: \"0 9 * * *\"\ncatchup: {catchup}\ntool: codex\n---\n"),
    )
    .unwrap();
    let ctx = Ctx::new(root.clone());
    sync_with(
        &ctx,
        &SyncEnv {
            unit_dir: units.clone(),
            data_dir: root.join("data"),
            ivar_bin: bin.clone(),
            path: None,
            is_linux: true,
            probes: Probes {
                is_linux: true,
                systemd_user: &yes,
                linger: &yes,
                docker_client: &yes,
                docker_daemon: &yes,
                docker_rootless: &rootless,
                tool_on_path: &|_| true,
                landlock: &yes,
            },
            // No clock: sync writes no stamp, as on a machine that never had one.
            clock: Clock {
                now_secs: 0,
                local_epoch_secs: &|_| None,
                local_wall: &|_| None,
            },
        },
    )
    .unwrap();
    (ctx, units, bin)
}

#[test]
fn a_catchup_timer_without_a_stamp_is_stale_catchup_and_in_sync_with_one() {
    let (_guard, root) = seeded_hall();
    let fake = FakeSystemctl::install(&root);
    let _r =
        crate::infra::proc::redirect_on_this_thread("systemctl", fake_systemctl_command(&fake));
    let (ctx, units, bin) = status_after_sync(&root, true);
    let data = root.join("data");

    let status = status_in(&ctx, &units, &data, &bin).unwrap().value;
    assert_eq!(status.agents[0].sync, UnitSync::StaleCatchup);
    let mut human = Vec::new();
    status.write_human(&mut human).unwrap();
    assert!(String::from_utf8(human).unwrap().contains("stale catchup"));
    assert_eq!(
        serde_json::to_value(&status).unwrap()["agents"][0]["sync"],
        serde_json::json!({"state": "stale_catchup"})
    );

    let hash = hall_hash(&Layout::at(root.clone()));
    let timer = format!("ivar-agent-{hash}-alpha.timer");
    assert!(crate::infra::systemd::write_stamp_if_missing(&data, &timer, 1_700_000_000).unwrap());
    let status = status_in(&ctx, &units, &data, &bin).unwrap().value;
    assert_eq!(status.agents[0].sync, UnitSync::InSync);
}

#[test]
fn an_agent_without_catchup_is_in_sync_without_a_stamp() {
    let (_guard, root) = seeded_hall();
    let fake = FakeSystemctl::install(&root);
    let _r =
        crate::infra::proc::redirect_on_this_thread("systemctl", fake_systemctl_command(&fake));
    let (ctx, units, bin) = status_after_sync(&root, false);
    let status = status_in(&ctx, &units, &root.join("data"), &bin)
        .unwrap()
        .value;
    assert_eq!(status.agents[0].sync, UnitSync::InSync);
}

#[test]
fn a_catchup_agent_that_is_not_installed_is_not_stale_catchup() {
    let (_guard, root) = seeded_hall();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(
        root.join("agents/alpha.md"),
        "---\nschedule: \"0 9 * * *\"\ncatchup: true\ntool: codex\n---\n",
    )
    .unwrap();
    let status = status_in(
        &Ctx::new(root.clone()),
        &root.join("units"),
        &root.join("data"),
        &root.join("ivar-bin"),
    )
    .unwrap()
    .value;
    assert_eq!(status.agents[0].sync, UnitSync::NotInstalled);
}
