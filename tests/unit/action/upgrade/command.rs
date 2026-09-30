#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::cell::RefCell;
use std::time::Duration;

use camino::Utf8PathBuf;

use super::*;
use crate::action::upgrade::cache::{self, CacheEntry};
use crate::domain::upgrade::{ChannelEnv, Version};
use crate::error::{Failure, WriteHuman};
use crate::infra::proc;
use crate::infra::release::LatestRelease;
use crate::test_support::utf8_temp_dir;

struct Fake(Result<&'static str, &'static str>);

impl LatestRelease for Fake {
    fn latest_location(&self, _timeout: Duration) -> Result<String, Failure> {
        self.0
            .map(str::to_owned)
            .map_err(|code| Failure::failed(code, "offline"))
    }
}

const TAG_13: &str = "https://github.com/mnzsss/ivar/releases/tag/v0.13.0";

fn env() -> ChannelEnv {
    ChannelEnv {
        home: Some(Utf8PathBuf::from("/home/u")),
        ..ChannelEnv::default()
    }
}

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

/// Runs `upgrade_with` against a fake source, recording every command it
/// would have run and answering each with `exit`.
fn run_with(
    source: Fake,
    exe: &str,
    current: &str,
    check: bool,
    exit: Option<i32>,
    cache: Option<Utf8PathBuf>,
) -> (Outcome<UpgradeReport>, Vec<String>) {
    let ran = RefCell::new(Vec::new());
    let mut runner = |command: &proc::Command| -> Result<Option<i32>, Failure> {
        ran.borrow_mut().push(command.display());
        Ok(exit)
    };
    let outcome = upgrade_with(
        UpgradeDeps {
            source: &source,
            current: v(current),
            exe: Some(Utf8PathBuf::from(exe)),
            env: env(),
            cache,
            now: 1_000,
            run: &mut runner,
        },
        &UpgradeInput { check },
    );
    (outcome, ran.into_inner())
}

#[test]
fn an_installer_binary_runs_the_installer_into_its_own_directory() {
    let (outcome, ran) = run_with(
        Fake(Ok(TAG_13)),
        "/home/u/.local/bin/ivar",
        "0.12.0",
        false,
        Some(0),
        None,
    );

    let report = outcome.unwrap().value;
    assert_eq!(report.channel, "installer");
    assert!(matches!(report.action, UpgradeAction::Ran { .. }));
    assert_eq!(ran.len(), 1);
    assert!(ran[0].contains("ivar.run/install"), "{}", ran[0]);
}

#[test]
fn a_cargo_binary_runs_cargo_install() {
    let (outcome, ran) = run_with(
        Fake(Ok(TAG_13)),
        "/home/u/.cargo/bin/ivar",
        "0.12.0",
        false,
        Some(0),
        None,
    );

    assert_eq!(outcome.unwrap().value.channel, "cargo");
    assert_eq!(ran, vec!["cargo install ivar --locked".to_owned()]);
}

#[test]
fn a_system_binary_prints_the_package_manager_command_and_runs_nothing() {
    let (outcome, ran) = run_with(
        Fake(Ok(TAG_13)),
        "/usr/bin/ivar",
        "0.12.0",
        false,
        Some(0),
        None,
    );

    let report = outcome.unwrap().value;
    assert!(ran.is_empty());
    let UpgradeAction::Manual { commands } = report.action else {
        panic!("{report:?}")
    };
    assert!(
        commands.iter().any(|c| c.contains("ivar-bin")),
        "{commands:?}"
    );
}

#[test]
fn already_latest_runs_nothing() {
    let (outcome, ran) = run_with(
        Fake(Ok(TAG_13)),
        "/home/u/.cargo/bin/ivar",
        "0.13.0",
        false,
        Some(0),
        None,
    );

    assert_eq!(outcome.unwrap().value.action, UpgradeAction::UpToDate);
    assert!(ran.is_empty());
}

#[test]
fn check_reports_and_records_without_running_anything() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("update-check.json");

    let (outcome, ran) = run_with(
        Fake(Ok(TAG_13)),
        "/home/u/.cargo/bin/ivar",
        "0.12.0",
        true,
        Some(0),
        Some(path.clone()),
    );

    let report = outcome.unwrap().value;
    assert_eq!(report.action, UpgradeAction::Available);
    assert_eq!(report.latest, v("0.13.0"));
    assert!(ran.is_empty());
    assert_eq!(
        cache::read(&path),
        Some(CacheEntry {
            last_checked_at: 1_000,
            latest_version: Some("0.13.0".to_owned())
        })
    );
}

#[test]
fn an_unreachable_release_is_a_failure() {
    let (outcome, ran) = run_with(
        Fake(Err("release.request_failed")),
        "/home/u/.cargo/bin/ivar",
        "0.12.0",
        false,
        Some(0),
        None,
    );

    assert_eq!(outcome.unwrap_err().code, "upgrade.latest_unknown");
    assert!(ran.is_empty());
}

#[test]
fn a_failing_delegated_command_is_a_failure_naming_it() {
    let (outcome, _) = run_with(
        Fake(Ok(TAG_13)),
        "/home/u/.cargo/bin/ivar",
        "0.12.0",
        false,
        Some(101),
        None,
    );

    let failure = outcome.unwrap_err();
    assert_eq!(failure.code, "upgrade.command_failed");
    assert!(
        failure.what.contains("cargo install ivar --locked"),
        "{}",
        failure.what
    );
}

#[test]
fn the_json_shape_names_the_action() {
    let report = UpgradeReport {
        current: v("0.12.0"),
        latest: v("0.13.0"),
        channel: "cargo",
        action: UpgradeAction::Available,
    };
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["current"], "0.12.0");
    assert_eq!(json["latest"], "0.13.0");
    assert_eq!(json["action"], "available");
}

#[test]
fn the_human_form_of_available_points_at_the_command() {
    let report = UpgradeReport {
        current: v("0.12.0"),
        latest: v("0.13.0"),
        channel: "cargo",
        action: UpgradeAction::Available,
    };
    let mut out = Vec::new();
    report.write_human(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("0.13.0") && text.contains("0.12.0") && text.contains("ivar upgrade"),
        "{text}"
    );
}
