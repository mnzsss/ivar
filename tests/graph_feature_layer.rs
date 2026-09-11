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
fn e2e_feature_session_reads_its_checkout_and_refreshes_dirty_edits() {
    let hall = TestHall::new();
    hall.commit_base(
        "core",
        &[
            (
                "src/lib.rs",
                "pub fn alpha_fn() -> i32 { 10 }\npub fn keep_fn() {}\n",
            ),
            ("src/old.rs", "pub fn old_fn() {}\n"),
        ],
    );
    hall.graph_command(hall.root(), &["index", "--repo", "core"]);

    let worktree = hall.promote("rename-feat", "core");
    hall.write(
        &worktree,
        "src/lib.rs",
        "pub fn beta_fn() -> i32 { 20 }\npub fn keep_fn() {}\n",
    );
    hall.write(&worktree, "src/new.rs", "pub fn new_fn() {}\n");
    std::fs::remove_file(worktree.join("src/old.rs")).unwrap();
    hall.commit(&worktree, "rename alpha and change file set");
    let view = hall.connect_view("rename-feat");

    let feature_find = hall.graph_command(&view, &["find", "beta_fn", "--json"]);
    assert_eq!(feature_find["matches"][0]["name"], "beta_fn");
    assert!(
        hall.graph_command(&view, &["find", "alpha_fn", "--json"])["matches"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        hall.graph_command(&view, &["find", "old_fn", "--json"])["matches"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        hall.graph_command(&view, &["find", "new_fn", "--json"])["matches"][0]["name"],
        "new_fn"
    );

    let explored = hall.graph_command(&view, &["explore", "beta_fn", "--json"]);
    assert!(
        explored["sources"][0]["content"]
            .as_str()
            .unwrap()
            .contains("pub fn beta_fn()")
    );

    let base_find = hall.graph_command(hall.root(), &["find", "alpha_fn", "--json"]);
    assert_eq!(base_find["matches"][0]["name"], "alpha_fn");

    hall.write(
        &worktree,
        "src/lib.rs",
        "pub fn zeta_fn() -> i32 { 20 }\npub fn keep_fn() {}\n",
    );
    let dirty_find = hall.graph_command(&view, &["find", "zeta_fn", "--json"]);
    assert_eq!(dirty_find["matches"][0]["name"], "zeta_fn");
    assert!(
        hall.graph_command(&view, &["find", "beta_fn", "--json"])["matches"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    hall.graph_command(
        hall.root(),
        &["clean", "--feature", "rename-feat", "--json"],
    );
    let stats = hall.graph_command(hall.root(), &["stats", "--json"]);
    assert!(
        stats["layers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|layer| layer["feature"] != "rename-feat")
    );
}
