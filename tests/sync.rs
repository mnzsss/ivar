//! End-to-end tests for `ivar sync`, driving the compiled binary.
//!
//! The behaviour — idempotence, the receipt, the managed block, per-repo
//! failures becoming warnings — is unit-tested in `src/action/sync.rs` against
//! real git repositories in temp directories. These tests exist for the two
//! things only the real process can prove:
//!
//! - **the exit code contract**: `0` clean, `1` when a `Warning` came back, `2`
//!   for a `Failure`. That mapping lives in `bin/ivar.rs` and nothing below it
//!   can check it.
//! - **`--json` and the human surface reporting the same facts**, because they
//!   are meant to be one value rendered twice.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/integration.rs"]
mod common;

use common::{declare_repos, hall_root, ivar, seeded_repo};
use predicates::prelude::*;

#[test]
fn a_clean_sync_exits_zero_and_materialises_the_hall() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");
    declare_repos(&root, &[("api", &origin, "main")]);

    ivar()
        .current_dir(&root)
        .arg("sync")
        .assert()
        .success()
        .code(0);

    assert!(root.join(".ivar/repos/api/.bare/HEAD").is_file());
    assert!(root.join(".ivar/repos/api/main/README.md").is_file());
    assert!(root.join("CLAUDE.md").is_file());
}

/// The warning channel's whole point, at the process boundary: the run went
/// through, something needs attention, and the exit code says so without
/// claiming the command failed.
#[test]
fn a_repo_that_cannot_be_cloned_exits_one_not_two() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    declare_repos(&root, &[("ghost", &root.join("no-such-origin"), "main")]);

    ivar()
        .current_dir(&root)
        .arg("sync")
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("repo ghost"));
}

/// The manifest declaring a branch the remote does not have is a common
/// first-run mistake (`main` versus `master`). git's own refusal names neither
/// the manifest nor the branch that does exist.
#[test]
fn a_branch_the_repo_does_not_have_names_the_one_it_does() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "master");
    declare_repos(&root, &[("api", &origin, "main")]);

    ivar()
        .current_dir(&root)
        .arg("sync")
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("master"));
}

#[test]
fn syncing_outside_a_hall_exits_two_and_points_at_init() {
    let (_guard, root) = hall_root();

    ivar()
        .current_dir(&root)
        .arg("sync")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("ivar init"));
}

#[test]
fn json_and_human_surfaces_carry_the_same_facts() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");
    declare_repos(&root, &[("api", &origin, "main")]);

    let json_output = ivar()
        .current_dir(&root)
        .args(["sync", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&json_output).expect("valid json");
    assert_eq!(value["root"], root.as_str());
    let entries = value["entries"].as_array().expect("entries is an array");
    assert!(
        entries
            .iter()
            .any(|e| e["surface"] == "repo api" && e["label"] == "bare clone"),
        "expected a bare-clone entry in {entries:?}"
    );

    // A second sync, on a hall that is now current, renders the same facts for
    // a human: the root, and every surface the JSON listed.
    let human = String::from_utf8(
        ivar()
            .current_dir(&root)
            .arg("sync")
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .expect("utf8 output");
    assert!(human.contains(root.as_str()));
    assert!(human.contains("repo api"));
    assert!(human.contains("claude-code"));
}

/// `git pull && ivar sync` is the daily command. If the second run rewrote
/// anything, every run would leave a spurious modification behind.
#[test]
fn a_second_sync_leaves_every_managed_file_byte_identical() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");
    declare_repos(&root, &[("api", &origin, "main")]);
    ivar().current_dir(&root).arg("sync").assert().success();

    let before = [
        std::fs::read(root.join("CLAUDE.md")).unwrap(),
        std::fs::read(root.join(".gitignore")).unwrap(),
        std::fs::read(root.join("ivar.json")).unwrap(),
    ];

    ivar().current_dir(&root).arg("sync").assert().success();

    let after = [
        std::fs::read(root.join("CLAUDE.md")).unwrap(),
        std::fs::read(root.join(".gitignore")).unwrap(),
        std::fs::read(root.join("ivar.json")).unwrap(),
    ];
    assert_eq!(before, after);
}
