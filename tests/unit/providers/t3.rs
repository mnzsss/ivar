#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::infra::proc::Command;
use crate::test_support::{
    FakeT3, fake_t3_command, fake_t3_on_this_thread, hall_root, redirect_t3,
};
use camino::Utf8PathBuf;

fn view_dir(root: &Utf8PathBuf) -> Utf8PathBuf {
    let dir = root.join("view");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn open_runs_t3_app_with_the_view_dir_and_is_idempotent() {
    let (_guard, root) = hall_root();
    let fake = FakeT3::install(&root);
    let _t3 = fake_t3_on_this_thread(&fake);
    let dir = view_dir(&root);

    host::open(&dir).unwrap();
    host::open(&dir).unwrap();

    assert_eq!(fake.log(), format!("app {dir}\napp {dir}\n"));
    assert_eq!(fake.projects(), vec![dir.to_string()]);
}

#[test]
fn close_removes_without_force_and_treats_an_unknown_project_as_closed() {
    let (_guard, root) = hall_root();
    let fake = FakeT3::install(&root);
    let _t3 = fake_t3_on_this_thread(&fake);
    let dir = view_dir(&root);
    host::open(&dir).unwrap();

    host::close(&dir).unwrap();
    host::close(&dir).unwrap();

    assert!(fake.projects().is_empty());
    assert!(!fake.log().contains("--force"));
    assert!(fake.log().contains(&format!("project remove {dir}")));
}

#[test]
fn a_failure_is_reported_as_its_first_line() {
    let (_guard, root) = hall_root();
    let fake = FakeT3::install(&root);
    let _t3 = redirect_t3(fake_t3_command(&fake).env("T3_FAKE_FAIL", "1"));
    let dir = view_dir(&root);

    assert_eq!(
        host::open(&dir).unwrap_err(),
        "DesktopAppRequestFailedError: fake failure"
    );
    assert_eq!(
        host::close(&dir).unwrap_err(),
        "DesktopAppRequestFailedError: fake failure"
    );
}

#[test]
fn probe_tells_ready_unreachable_and_missing_apart() {
    let (_guard, root) = hall_root();
    let fake = FakeT3::install(&root);
    let dir = view_dir(&root);

    {
        let _t3 = fake_t3_on_this_thread(&fake);
        assert_eq!(host::probe(&dir), host::Probe::Ready);
    }
    {
        let _t3 = redirect_t3(fake_t3_command(&fake).env("T3_FAKE_FAIL", "1"));
        assert_eq!(
            host::probe(&dir),
            host::Probe::Unreachable("DesktopAppRequestFailedError: fake failure".to_owned())
        );
    }
    {
        let _gone = redirect_t3(Command::new("/nonexistent/t3"));
        assert_eq!(host::probe(&dir), host::Probe::Missing);
    }
}

#[test]
fn preflight_names_the_missing_binary() {
    let _gone = redirect_t3(Command::new("/nonexistent/t3"));
    assert_eq!(host::preflight().unwrap_err().code, "t3.binary_missing");
}

#[test]
fn parse_version_reads_the_numbers_and_ignores_suffixes() {
    assert_eq!(host::parse_version("t3 v0.1.0"), Some((0, 1, 0)));
    assert_eq!(host::parse_version("0.1.0"), Some((0, 1, 0)));
    assert_eq!(host::parse_version("t3 v0.0.45"), Some((0, 0, 45)));
    assert_eq!(host::parse_version("t3 v1.2.3-dev+abc"), Some((1, 2, 3)));
    assert_eq!(host::parse_version("not a version"), None);
}

#[test]
fn program_prefers_a_non_empty_t3code_cli_path() {
    assert_eq!(host::program_from(None), "t3");
    assert_eq!(host::program_from(Some("/x/t3")), "/x/t3");
    assert_eq!(host::program_from(Some("")), "t3");
}

#[test]
fn preflight_refuses_a_t3_older_than_the_measured_version() {
    let (_guard, root) = hall_root();
    let fake = FakeT3::install(&root);
    {
        let _t3 = redirect_t3(fake_t3_command(&fake).env("T3_FAKE_VERSION", "0.0.45"));
        let failure = host::preflight().unwrap_err();
        assert_eq!(failure.code, "t3.version_unsupported");
        assert!(failure.what.contains("v0.0.45"));
        assert!(failure.what.contains("older than 0.1.0"));
    }
    {
        let _t3 = fake_t3_on_this_thread(&fake);
        host::preflight().unwrap();
    }
}

#[test]
fn probe_stops_at_an_outdated_t3_without_running_a_project_command() {
    let (_guard, root) = hall_root();
    let fake = FakeT3::install(&root);
    let dir = view_dir(&root);
    let _t3 = redirect_t3(fake_t3_command(&fake).env("T3_FAKE_VERSION", "0.0.45"));

    assert_eq!(
        host::probe(&dir),
        host::Probe::Outdated("0.0.45".to_owned())
    );
    assert_eq!(fake.log(), "--version\n");
}
