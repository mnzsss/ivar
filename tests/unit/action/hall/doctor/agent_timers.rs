#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::Ctx;
use crate::action::agent::requirements::Probes;
use crate::action::agent::sync::{Clock, SyncEnv, sync_with};
use crate::test_support::{FakeSystemctl, fake_systemctl_on_this_thread, seeded_hall};

fn yes() -> bool {
    true
}
fn rootless() -> Option<bool> {
    Some(true)
}

const NOW: i64 = 1_793_966_400;

fn utc_wall(wall: &str) -> Option<i64> {
    let parsed = crate::domain::agent::cron::WallTime::parse(wall)?;
    Some(crate::domain::session::epoch_secs_from_civil(
        parsed.year,
        i64::from(parsed.month),
        i64::from(parsed.day),
        i64::from(parsed.hour),
        i64::from(parsed.minute),
    ))
}

fn utc_wall_of(secs: i64) -> Option<String> {
    Some(crate::domain::session::utc_calendar_from_epoch_secs(secs)[..16].to_owned())
}

struct Setup {
    root: camino::Utf8PathBuf,
    units: camino::Utf8PathBuf,
    data: camino::Utf8PathBuf,
    bin: camino::Utf8PathBuf,
}

impl Setup {
    fn sync(&self) {
        sync_with(
            &Ctx::new(self.root.clone()),
            &SyncEnv {
                unit_dir: self.units.clone(),
                data_dir: self.data.clone(),
                ivar_bin: self.bin.clone(),
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
                clock: Clock {
                    now_secs: NOW,
                    local_epoch_secs: &utc_wall,
                    local_wall: &utc_wall_of,
                },
            },
        )
        .unwrap();
    }

    fn findings(&self) -> Vec<Diagnosis> {
        let layout = Layout::at(self.root.clone());
        let specs: Vec<_> = crate::store::agent::list_files(&layout)
            .unwrap()
            .iter()
            .map(|file| crate::store::agent::parse(file).unwrap())
            .collect();
        catchup_stamp_diagnoses(&layout, &specs, &self.units, &self.data, &self.bin)
    }
}

fn setup(root: &camino::Utf8PathBuf, agents: &[(&str, bool)]) -> Setup {
    std::fs::create_dir_all(root.join("agents")).unwrap();
    for (name, catchup) in agents {
        std::fs::write(
            root.join(format!("agents/{name}.md")),
            format!("---\nschedule: \"0 7 * * *\"\ncatchup: {catchup}\ntool: codex\n---\n"),
        )
        .unwrap();
    }
    let bin = root.join("ivar-bin");
    std::fs::write(&bin, "").unwrap();
    Setup {
        root: root.clone(),
        units: root.join("units"),
        data: root.join("data"),
        bin,
    }
}

#[test]
fn an_installed_catchup_timer_without_a_stamp_is_named_with_the_fix_and_clears_after_sync() {
    let (_guard, root) = seeded_hall();
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    let setup = setup(&root, &[("metrics", true), ("plain", false)]);
    setup.sync();
    let hash = crate::action::agent::hall_hash(&Layout::at(root.clone()));
    std::fs::remove_file(crate::infra::systemd::stamp_path(
        &setup.data,
        &format!("ivar-agent-{hash}-metrics.timer"),
    ))
    .unwrap();

    let findings = setup.findings();

    let [finding] = findings.as_slice() else {
        panic!("{findings:?}");
    };
    assert_eq!(finding.code, "agent.catchup_stamp_missing");
    assert!(finding.what.contains("`metrics`"));
    assert!(finding.fix.contains("ivar agent sync"));

    setup.sync();
    assert!(setup.findings().is_empty());
}

#[test]
fn nothing_is_reported_for_an_agent_that_is_not_installed_or_whose_unit_is_outdated() {
    let (_guard, root) = seeded_hall();
    let fake = FakeSystemctl::install(&root);
    let _r = fake_systemctl_on_this_thread(&fake);
    let setup = setup(&root, &[("metrics", true)]);
    assert!(setup.findings().is_empty());

    setup.sync();
    let hash = crate::action::agent::hall_hash(&Layout::at(root.clone()));
    std::fs::remove_file(crate::infra::systemd::stamp_path(
        &setup.data,
        &format!("ivar-agent-{hash}-metrics.timer"),
    ))
    .unwrap();
    std::fs::write(
        root.join("agents/metrics.md"),
        "---\nschedule: \"0 8 * * *\"\ncatchup: true\ntool: codex\n---\n",
    )
    .unwrap();
    assert!(setup.findings().is_empty());
}
