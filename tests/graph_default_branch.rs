#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use support::common;
use support::graph::TestHall;

#[test]
fn e2e_graph_indexes_and_checks_the_declared_default_branch_worktree() {
    let hall = TestHall::new();
    hall.commit_base_on("core", "trunk", &[("src/lib.rs", "pub fn trunk_fn() {}\n")]);

    hall.graph_command(hall.root(), &["index", "--json"]);
    assert_eq!(
        hall.graph_command(hall.root(), &["find", "trunk_fn", "--json"])["matches"][0]["name"],
        "trunk_fn"
    );

    let trunk = hall.root().join(".ivar/repos/core/trunk");
    hall.write(
        &trunk,
        "src/lib.rs",
        "pub fn trunk_fn() {}\npub fn later_fn() {}\n",
    );
    hall.commit(&trunk, "advance trunk");

    let output = common::ivar()
        .current_dir(hall.root())
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("graph.repo_stale"), "{stdout}");
}
