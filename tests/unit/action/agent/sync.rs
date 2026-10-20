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

fn env(unit_dir: Utf8PathBuf) -> SyncEnv<'static> {
    SyncEnv {
        unit_dir,
        ivar_bin: Utf8PathBuf::from("/opt/ivar/bin/ivar"),
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
