#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::infra::proc::{Command, redirect_on_this_thread};
use crate::test_support::{FakeT3, fake_t3_command, fake_t3_on_this_thread, hall_root};
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
    let _t3 = redirect_on_this_thread("t3", fake_t3_command(&fake).env("T3_FAKE_FAIL", "1"));
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
        let _t3 = redirect_on_this_thread("t3", fake_t3_command(&fake).env("T3_FAKE_FAIL", "1"));
        assert_eq!(
            host::probe(&dir),
            host::Probe::Unreachable("DesktopAppRequestFailedError: fake failure".to_owned())
        );
    }
    {
        let _gone = redirect_on_this_thread("t3", Command::new("/nonexistent/t3"));
        assert_eq!(host::probe(&dir), host::Probe::Missing);
    }
}

#[test]
fn preflight_names_the_missing_binary() {
    let _gone = redirect_on_this_thread("t3", Command::new("/nonexistent/t3"));
    assert_eq!(host::preflight().unwrap_err().code, "t3.binary_missing");
}
