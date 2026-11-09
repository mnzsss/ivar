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
            ivar_bin: bin.clone(),
            path: Some("/a/shell/that/synced:/usr/bin".to_owned()),
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
                now_secs: 0,
                local_epoch_secs: &|_| None,
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

    let agents = status_in(&ctx, &units, &bin).unwrap().value.agents;
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
    let agents = status_in(&ctx, &units, &bin).unwrap().value.agents;
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
    let status = status_in(&ctx, &units, &bin).unwrap().value;
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
