//! `--land` with pull-request metadata, rejected through the CLI.

use crate::common::{hall_root, ivar};
use crate::support::{approve_through_plan, setup_deliver_hall};
use predicates::prelude::*;

/// Land conflict: metadata cannot be used with --land.
#[test]
fn land_conflict() {
    let (_guard, root) = hall_root();
    setup_deliver_hall(&root);
    approve_through_plan(&root, "checkout");

    ivar()
        .current_dir(&root)
        .args([
            "feature",
            "deliver",
            "checkout",
            "--land",
            "--name",
            "feat",
            "--preview",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("cannot be used in land mode"));
}
