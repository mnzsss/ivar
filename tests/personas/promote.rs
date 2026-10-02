//! Promotion recovery: failed and interrupted promotions are explained and
//! retried, feature records keep every write, and doctor sees the drift.

use camino::Utf8Path;

use super::support::{home, isolated_ivar, one_repo_hall, run_ok};
use crate::common::{git, hall_root};

#[test]
fn promote_two_features_cannot_share_a_branch() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(
        &root,
        &["feature", "create", "login", "--branch", "feat/login"],
    );
    isolated_ivar(&home(&root))
        .current_dir(&root)
        .args(["feature", "create", "login-two", "--branch", "feat/login"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("login"));
}

fn write_setup(root: &Utf8Path, body: &str) {
    std::fs::create_dir_all(root.join(".ivar/setups")).unwrap();
    std::fs::write(
        root.join(".ivar/setups/app.sh"),
        format!("#!/usr/bin/env bash\n{body}\n"),
    )
    .unwrap();
}

fn promote_with_failing_setup(root: &Utf8Path, feature: &str) {
    let output = isolated_ivar(&home(root))
        .current_dir(root)
        .args(["feature", "promote", feature, "app", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("feature.setup_script_failed"), "{stdout}");
}

#[test]
fn promote_a_failed_promotion_is_retried_by_promoting_again() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    write_setup(
        &root,
        "[ -f \"$IVAR_HALL/.setup-ok\" ] || { echo 'pub get failed' >&2; exit 3; }",
    );
    run_ok(&root, &["feature", "create", "checkout"]);
    promote_with_failing_setup(&root, "checkout");

    let failed = run_ok(&root, &["feature", "status", "checkout"]);
    assert_eq!(failed["repos"][0]["state"], "failed");
    let reason = failed["repos"][0]["reason"].as_str().unwrap();
    assert!(
        reason.contains("exit 3") && reason.contains("pub get failed"),
        "{reason}"
    );
    assert_eq!(
        failed["repos"][0]["retry"],
        "ivar feature promote checkout app"
    );

    std::fs::write(root.join(".setup-ok"), "").unwrap();
    run_ok(&root, &["feature", "promote", "checkout", "app"]);
    let ready = run_ok(&root, &["feature", "status", "checkout"]);
    assert_eq!(ready["repos"][0]["state"], "ready");
    assert!(ready["repos"][0].get("reason").is_none(), "{ready}");
}

#[test]
fn promote_setup_output_goes_to_a_log_not_the_callers_stdout() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    write_setup(&root, "echo bootstrapped");
    run_ok(&root, &["feature", "create", "checkout"]);
    let promoted = run_ok(&root, &["feature", "promote", "checkout", "app"]);
    assert_eq!(promoted["setup_ran"], true, "{promoted}");
    let log = std::fs::read_to_string(root.join(".ivar/features/checkout/setup-app.log")).unwrap();
    assert!(log.contains("bootstrapped"), "{log}");
}

#[test]
fn promote_recreates_a_promoted_worktree_that_was_deleted() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["feature", "create", "checkout"]);
    run_ok(&root, &["feature", "promote", "checkout", "app"]);
    std::fs::remove_dir_all(root.join(".ivar/repos/app/checkout")).unwrap();

    let missing = run_ok(&root, &["feature", "status", "checkout"]);
    assert_eq!(missing["repos"][0]["worktree_present"], false);
    assert_eq!(
        missing["repos"][0]["retry"],
        "ivar feature promote checkout app"
    );

    run_ok(&root, &["feature", "promote", "checkout", "app"]);
    let back = run_ok(&root, &["feature", "status", "checkout"]);
    assert_eq!(back["repos"][0]["worktree_present"], true, "{back}");
}

#[cfg(target_os = "linux")]
#[test]
fn promote_tells_a_running_session_to_restart() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["feature", "create", "checkout"]);
    let session = run_ok(&root, &["session", "start", "checkout", "--detached"]);
    let view_dir = session["view_dir"].as_str().unwrap();
    let mut agent = std::process::Command::new("bash")
        .args(["-c", "exec -a claude sleep 30"])
        .current_dir(view_dir)
        .spawn()
        .unwrap();

    let promoted = run_ok(&root, &["feature", "promote", "checkout", "app"]);
    agent.kill().unwrap();
    let _ = agent.wait();

    let warnings = promoted["warnings"].to_string();
    assert!(warnings.contains("session.restart_required"), "{promoted}");
    assert!(
        warnings.contains("ivar session start checkout --resume"),
        "{promoted}"
    );
}

#[test]
fn sync_setup_output_goes_to_a_log_not_the_callers_stdout() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    write_setup(&root, "echo bootstrapped");
    run_ok(&root, &["sync"]);
    let log =
        std::fs::read_to_string(root.join(".ivar/repos/app/.bare/worktrees/main/ivar-setup.log"))
            .unwrap();
    assert!(log.contains("bootstrapped"), "{log}");
}

fn finding<'a>(doctor: &'a serde_json::Value, code: &str) -> &'a serde_json::Value {
    doctor["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["code"] == code)
        .unwrap_or_else(|| panic!("no `{code}` in {doctor}"))
}

#[test]
fn doctor_names_a_failed_promotion_with_the_retry_command() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    write_setup(&root, "exit 3");
    run_ok(&root, &["feature", "create", "checkout"]);
    promote_with_failing_setup(&root, "checkout");

    let doctor = run_ok(&root, &["doctor"]);
    let fix = finding(&doctor, "feature.promotion_failed")["fix"]
        .as_str()
        .unwrap();
    assert!(fix.contains("ivar feature promote checkout app"), "{fix}");
}

#[test]
fn doctor_names_a_missing_promotion_worktree_and_sync_prunes_its_registration() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["feature", "create", "checkout"]);
    run_ok(&root, &["feature", "promote", "checkout", "app"]);
    std::fs::remove_dir_all(root.join(".ivar/repos/app/checkout")).unwrap();

    let doctor = run_ok(&root, &["doctor"]);
    finding(&doctor, "feature.promotion_worktree_missing");

    run_ok(&root, &["sync"]);
    let bare = root.join(".ivar/repos/app/.bare");
    let list = std::process::Command::new("git")
        .args([
            "--git-dir",
            bare.as_str(),
            "worktree",
            "list",
            "--porcelain",
        ])
        .output()
        .unwrap();
    let list = String::from_utf8_lossy(&list.stdout);
    assert!(!list.contains("prunable"), "{list}");
}

#[test]
fn doctor_names_two_features_on_one_branch() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(
        &root,
        &["feature", "create", "login", "--branch", "feat/login"],
    );
    let record = std::fs::read_to_string(root.join(".ivar/features/login/feature.json")).unwrap();
    std::fs::create_dir_all(root.join(".ivar/features/login-two")).unwrap();
    std::fs::write(
        root.join(".ivar/features/login-two/feature.json"),
        record.replace("\"name\": \"login\"", "\"name\": \"login-two\""),
    )
    .unwrap();

    let doctor = run_ok(&root, &["doctor"]);
    let what = finding(&doctor, "feature.branch_shared")["what"]
        .as_str()
        .unwrap();
    assert!(what.contains("feat/login"), "{what}");
}

#[test]
fn doctor_names_stale_integrate_staging() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["feature", "create", "checkout"]);
    run_ok(&root, &["feature", "promote", "checkout", "app"]);
    let candidate = root.join(".ivar/features/checkout/integration/app/candidate");
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

    let doctor = run_ok(&root, &["doctor"]);
    let fix = finding(&doctor, "integrate.staging_stale")["fix"]
        .as_str()
        .unwrap();
    assert!(fix.contains("worktree remove"), "{fix}");
}
