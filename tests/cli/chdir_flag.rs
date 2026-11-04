//! `-C <dir>` is a global option that runs any verb as if `ivar` had been
//! started in `<dir>`, so a caller that cannot set a working directory still
//! reaches its hall with a single argv.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::common::{hall_root, ivar};
use predicates::prelude::*;

fn stdout_of(cmd: &mut assert_cmd::Command) -> String {
    let out = cmd.assert().success().get_output().stdout.clone();
    String::from_utf8(out).unwrap()
}

#[test]
fn status_through_c_matches_status_run_inside_the_hall() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let (_other_guard, elsewhere) = hall_root();

    let inside = stdout_of(ivar().current_dir(&root).args(["--json", "status"]));
    let through_c =
        stdout_of(
            ivar()
                .current_dir(&elsewhere)
                .args(["-C", root.as_str(), "--json", "status"]),
        );
    // `-C` is global, so it also works after the subcommand.
    let trailing =
        stdout_of(
            ivar()
                .current_dir(&elsewhere)
                .args(["--json", "status", "-C", root.as_str()]),
        );

    assert_eq!(inside, through_c);
    assert_eq!(inside, trailing);
}

#[test]
fn a_missing_directory_fails_naming_it() {
    let (_guard, root) = hall_root();
    let missing = root.join("no-such-dir");

    ivar()
        .current_dir(&root)
        .args(["-C", missing.as_str(), "status"])
        .assert()
        .failure()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(missing.as_str()));
}

#[test]
fn a_missing_directory_keeps_the_json_failure_shape() {
    let (_guard, root) = hall_root();
    let missing = root.join("no-such-dir");

    let out = ivar()
        .current_dir(&root)
        .args(["-C", missing.as_str(), "--json", "status"])
        .assert()
        .failure()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["code"], "cli.chdir_failed");
    assert!(value["what"].as_str().unwrap().contains(missing.as_str()));
}
