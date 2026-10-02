//! Integrate recovery: a failed integrate is clean and retryable, and
//! integrations into one parent are serialized.

use camino::Utf8Path;

use super::support::{home, isolated_ivar, one_repo_hall, run_ok};
use crate::common::{git, hall_root};

fn child_touching(root: &Utf8Path, child: &str, file: &str) {
    run_ok(root, &["feature", "create", child, "--parent", "proto"]);
    run_ok(root, &["feature", "promote", child, "app"]);
    let worktree = root.join(format!(".ivar/repos/app/{child}"));
    std::fs::write(worktree.join(file), format!("{child}\n")).unwrap();
    git(&worktree, &["add", "."]);
    git(&worktree, &["commit", "-m", child]);
    run_ok(root, &["plan", "create", child, "plan"]);
    run_ok(root, &["plan", "approve", child, "plan"]);
}

fn proto_with_children(root: &Utf8Path) {
    one_repo_hall(root);
    run_ok(root, &["feature", "create", "proto"]);
    run_ok(root, &["feature", "promote", "proto", "app"]);
    child_touching(root, "login", "routes.txt");
    child_touching(root, "profile", "routes.txt");
}

#[test]
fn integrate_a_conflicted_child_can_be_retried_after_it_is_fixed() {
    let (_guard, root) = hall_root();
    proto_with_children(&root);
    run_ok(&root, &["feature", "integrate", "login"]);

    let conflict = isolated_ivar(&home(&root))
        .current_dir(&root)
        .args(["feature", "integrate", "profile", "--json"])
        .assert()
        .failure();
    let body: serde_json::Value = serde_json::from_slice(&conflict.get_output().stdout).unwrap();
    assert_eq!(body["ok"], false, "{body}");
    assert!(
        !root
            .join(".ivar/features/profile/integration/app/candidate")
            .exists(),
        "a failed integrate leaves no staging behind"
    );

    let worktree = root.join(".ivar/repos/app/profile");
    git(&worktree, &["mv", "routes.txt", "profile-routes.txt"]);
    git(&worktree, &["commit", "-m", "own routes file"]);
    let retried = run_ok(&root, &["feature", "integrate", "profile"]);
    assert_eq!(retried["closed_integrated"], true, "{retried}");
}

#[test]
fn doctor_leaves_staging_alone_while_its_parent_integrates() {
    let (_guard, root) = hall_root();
    proto_with_children(&root);
    let candidate = root.join(".ivar/features/login/integration/app/candidate");
    let bare = root.join(".ivar/repos/app/.bare");
    git(
        &root,
        &[
            "--git-dir",
            bare.as_str(),
            "worktree",
            "add",
            "--detach",
            candidate.as_str(),
            "main",
        ],
    );
    let stale = |doctor: &serde_json::Value| {
        doctor["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "integrate.staging_stale")
    };

    let lock = std::fs::File::create(root.join(".ivar/features/proto/integrate.lock")).unwrap();
    lock.lock().unwrap();
    assert!(
        !stale(&run_ok(&root, &["doctor"])),
        "a running integrate owns its staging"
    );
    drop(lock);
    assert!(stale(&run_ok(&root, &["doctor"])));
}
