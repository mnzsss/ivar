//! Deliver refuses exactly what its preview lists, and says what `--land` pushes.

use camino::Utf8Path;
use predicates::prelude::*;

use super::support::{commit_slice, home, isolated_ivar, one_repo_hall, run_ok};
use crate::common::hall_root;

fn feature_with_a_slice(root: &Utf8Path) {
    one_repo_hall(root);
    run_ok(root, &["feature", "create", "ship"]);
    run_ok(root, &["feature", "promote", "ship", "app"]);
    commit_slice(root, "app", "ship", "slice.md");
}

fn approve_plan(root: &Utf8Path) {
    run_ok(root, &["plan", "create", "ship"]);
    for gate in ["requirements", "analysis", "plan"] {
        run_ok(root, &["plan", "approve", "ship", gate]);
    }
}

#[test]
fn deliver_refuses_exactly_what_its_preview_lists_as_blocked() {
    let (_guard, root) = hall_root();
    feature_with_a_slice(&root);
    approve_plan(&root);
    std::fs::write(root.join(".ivar/repos/app/ship/wip.md"), "wip\n").unwrap();

    let preview = run_ok(&root, &["feature", "deliver", "ship", "--preview"]);

    let blockers = preview["blockers"].as_array().expect("blockers listed");
    assert!(
        blockers
            .iter()
            .any(|b| b.as_str().unwrap().contains("uncommitted")),
        "the dirty worktree is a blocker: {blockers:?}"
    );
    assert!(
        preview["preview"]["repos"][0]["pending"][0]
            .as_str()
            .unwrap()
            .contains("not pushed"),
        "unpushed work is what deliver delivers, not a blocker: {preview}"
    );
    let fingerprint = preview["preview"]["fingerprint"].as_str().unwrap();
    isolated_ivar(&home(&root))
        .current_dir(&root)
        .args(["feature", "deliver", "ship", "--fingerprint", fingerprint])
        .assert()
        .failure()
        .stderr(predicate::str::contains("uncommitted"));
    let origin = root.parent().unwrap().join("origins/app");
    let branches = std::process::Command::new("git")
        .args(["branch", "--list", "ship"])
        .current_dir(&origin)
        .output()
        .unwrap();
    assert!(
        branches.stdout.is_empty(),
        "a refused delivery pushed nothing"
    );
}

#[test]
fn deliver_lists_a_missing_plan_and_says_to_create_one() {
    let (_guard, root) = hall_root();
    feature_with_a_slice(&root);

    let preview = run_ok(&root, &["feature", "deliver", "ship", "--preview"]);
    let blockers = preview["blockers"].as_array().expect("blockers listed");
    assert!(
        blockers
            .iter()
            .any(|b| b.as_str().unwrap().contains("plan")),
        "the unapproved plan gate is listed: {blockers:?}"
    );

    let fingerprint = preview["preview"]["fingerprint"].as_str().unwrap();
    isolated_ivar(&home(&root))
        .current_dir(&root)
        .args(["feature", "deliver", "ship", "--fingerprint", fingerprint])
        .assert()
        .failure()
        .stderr(predicate::str::contains("ivar plan create ship"));
}

#[test]
fn deliver_land_says_it_pushes_the_default_branch() {
    let (_guard, root) = hall_root();
    feature_with_a_slice(&root);

    isolated_ivar(&home(&root))
        .current_dir(&root)
        .args(["feature", "deliver", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("then push each default"));
    isolated_ivar(&home(&root))
        .current_dir(&root)
        .args(["feature", "deliver", "ship", "--land", "--preview"])
        .assert()
        .success()
        .stdout(predicate::str::contains("then push main to"));
}
