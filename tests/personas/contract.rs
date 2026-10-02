//! The CLI contract: exit codes and `ok` match the outcome, a feature session
//! infers its feature, and committed settings carry no clone-specific paths.

use camino::Utf8Path;

use super::support::{commit_slice, home, isolated_ivar, one_repo_hall, run_ok};
use crate::common::hall_root;

fn ivar_at(root: &Utf8Path) -> assert_cmd::Command {
    let mut cmd = isolated_ivar(&home(root));
    cmd.current_dir(root);
    cmd
}

#[test]
fn contract_plan_status_accepts_a_feature_name() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["feature", "create", "checkout"]);
    run_ok(&root, &["plan", "create", "checkout"]);

    let status = run_ok(&root, &["plan", "status", "checkout"]);

    assert_eq!(status["feature"], "checkout", "{status}");
}

#[test]
fn contract_a_feature_session_infers_its_feature() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["feature", "create", "checkout"]);
    run_ok(&root, &["plan", "create", "checkout"]);

    for args in [
        &["feature", "status", "--json"][..],
        &["plan", "status", "--json"],
    ] {
        ivar_at(&root)
            .env("IVAR_FEATURE", "checkout")
            .args(args)
            .assert()
            .success()
            .stdout(predicates::str::contains(r#":"checkout""#));
    }
    ivar_at(&root)
        .env("IVAR_FEATURE", "checkout")
        .args(["review", "comment", "list", "--json"])
        .assert()
        .success();
}

fn relay_failure(cmd: &mut assert_cmd::Command) -> serde_json::Value {
    let failed = cmd
        .args(["session", "relay", "--provider", "codex", "--json"])
        .assert()
        .failure();
    serde_json::from_slice(&failed.get_output().stdout).unwrap()
}

#[test]
fn contract_session_relay_infers_its_feature() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);

    let body = relay_failure(ivar_at(&root).env("IVAR_FEATURE", "ghost"));

    assert_ne!(body["code"], "feature.missing_argument", "{body}");
    assert!(body.to_string().contains("ghost"), "{body}");
}

#[test]
fn contract_session_relay_without_a_feature_is_a_missing_argument() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);

    let body = relay_failure(&mut ivar_at(&root));

    assert_eq!(body["code"], "feature.missing_argument", "{body}");
}

#[test]
fn contract_a_destructive_verb_never_infers_its_feature() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["feature", "create", "checkout"]);

    ivar_at(&root)
        .env("IVAR_FEATURE", "checkout")
        .args(["feature", "delete", "--json"])
        .assert()
        .code(2);

    assert!(root.join(".ivar/features/checkout/feature.json").exists());
}

#[test]
fn contract_settings_carry_no_clone_specific_paths() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);

    let raw = std::fs::read_to_string(root.join(".claude/settings.json")).unwrap();
    let settings: serde_json::Value = serde_json::from_str(&raw).unwrap();

    assert!(settings.pointer("/env/IVAR_HALL").is_none(), "{settings}");
    assert!(!raw.contains(root.as_str()), "{raw}");
}

#[test]
fn contract_a_clean_integrate_exits_zero() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);
    run_ok(&root, &["feature", "create", "proto"]);
    run_ok(&root, &["feature", "promote", "proto", "app"]);
    run_ok(&root, &["feature", "create", "login", "--parent", "proto"]);
    run_ok(&root, &["feature", "promote", "login", "app"]);
    commit_slice(&root, "app", "login", "login.txt");
    run_ok(&root, &["plan", "create", "login", "plan"]);
    run_ok(&root, &["plan", "approve", "login", "plan"]);

    let integrated = ivar_at(&root)
        .args(["feature", "integrate", "login", "--json"])
        .assert()
        .success();

    let body: serde_json::Value = serde_json::from_slice(&integrated.get_output().stdout).unwrap();
    assert_eq!(body["closed_integrated"], true, "{body}");
    assert!(body.get("warnings").is_none(), "{body}");
}

#[test]
fn contract_a_json_failure_prints_once_on_stdout() {
    let (_guard, root) = hall_root();
    one_repo_hall(&root);

    let failed = ivar_at(&root)
        .args(["feature", "status", "nope", "--json"])
        .assert()
        .code(2);

    let output = failed.get_output();
    let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["ok"], false, "{body}");
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
