#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::common::{hall_root, ivar};

const PATH: &str = ".ivar/scripts/deploy.sh";
const CLEAN: &str =
    "#!/usr/bin/env bash\n# description: Deploy.\n# usage: deploy.sh\n# env: TOKEN, URL\necho ok\n";

fn hall_with(content: &str) -> (tempfile::TempDir, camino::Utf8PathBuf) {
    let (guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    write(&root, content);
    (guard, root)
}

fn write(root: &camino::Utf8Path, content: &str) {
    let dir = root.join(".ivar/scripts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("deploy.sh"), content).unwrap();
}

fn run(root: &camino::Utf8Path, json: bool, args: &[&str]) -> (i32, String, String) {
    let mut cmd = ivar();
    cmd.current_dir(root);
    if json {
        cmd.arg("--json");
    }
    let output = cmd.args(args).output().unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

fn parsed(stdout: &str) -> serde_json::Value {
    serde_json::from_str(stdout).expect("valid json")
}

#[test]
fn approve_then_check_reports_approved_and_exits_zero() {
    let (_guard, root) = hall_with(CLEAN);

    let (code, out, _) = run(&root, false, &["script", "approve", PATH]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Approved .ivar/scripts/deploy.sh"), "{out}");

    let (code, out, _) = run(&root, false, &["script", "check", PATH]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out, "approved: .ivar/scripts/deploy.sh\nenv: TOKEN, URL\n");
}

#[test]
fn check_json_reports_approved_with_env_and_a_null_reason() {
    let (_guard, root) = hall_with(CLEAN);
    run(&root, false, &["script", "approve", PATH]);

    let (code, out, _) = run(&root, true, &["script", "check", PATH]);

    assert_eq!(code, 0);
    let value = parsed(&out);
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["path"], PATH);
    assert_eq!(value["approval"], "approved");
    assert_eq!(value["reason"], serde_json::Value::Null);
    assert_eq!(value["env"], serde_json::json!(["TOKEN", "URL"]));
    assert_eq!(value["sha256"].as_str().unwrap().len(), 64);
}

#[test]
fn an_unapproved_script_is_unreviewed_with_exit_code_three() {
    let (_guard, root) = hall_with(CLEAN);

    let (code, out, _) = run(&root, false, &["script", "check", PATH]);
    assert_eq!(code, 3, "{out}");
    assert_eq!(
        out,
        "unreviewed: .ivar/scripts/deploy.sh (no approval)\nenv: TOKEN, URL\n"
    );

    let (code, out, _) = run(&root, true, &["script", "check", PATH]);
    assert_eq!(code, 3);
    let value = parsed(&out);
    assert_eq!(value["approval"], "unreviewed");
    assert_eq!(value["reason"], "no approval");
}

#[test]
fn editing_an_approved_script_makes_it_unreviewed_because_it_changed() {
    let (_guard, root) = hall_with(CLEAN);
    run(&root, false, &["script", "approve", PATH]);
    write(&root, &format!("{CLEAN}echo more\n"));

    let (code, out, _) = run(&root, false, &["script", "check", PATH]);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("(changed since approval)"), "{out}");

    let (code, out, _) = run(&root, true, &["script", "check", PATH]);
    assert_eq!(code, 3);
    assert_eq!(parsed(&out)["reason"], "changed since approval");

    run(&root, false, &["script", "approve", PATH]);
    assert_eq!(run(&root, false, &["script", "check", PATH]).0, 0);
}

#[test]
fn approve_json_reports_path_hash_and_time() {
    let (_guard, root) = hall_with(CLEAN);

    let (code, out, _) = run(&root, true, &["script", "approve", PATH]);

    assert_eq!(code, 0, "{out}");
    let value = parsed(&out);
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["path"], PATH);
    assert_eq!(value["sha256"].as_str().unwrap().len(), 64);
    assert!(value["approved_at"].as_str().unwrap().ends_with('Z'));
}

#[test]
fn approve_refuses_a_secret_prints_line_and_pattern_and_records_nothing() {
    let secret = "ghp_0123456789abcdefghijklmnopqrstuvwxyzAB";
    let (_guard, root) = hall_with(&format!("{CLEAN}export GH={secret}\n"));

    let (code, out, err) = run(&root, false, &["script", "approve", PATH]);
    assert_eq!(code, 2, "{out}{err}");
    assert!(err.contains("line 6"), "{err}");
    assert!(err.contains("github_token"), "{err}");
    assert!(!err.contains(secret) && !out.contains(secret));

    let (code, out, _) = run(&root, true, &["script", "approve", PATH]);
    assert_eq!(code, 2);
    let value = parsed(&out);
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["code"], "script.secret_found");
    assert_eq!(
        value["details"]["findings"],
        serde_json::json!([{"line": 6, "pattern": "github_token"}])
    );

    assert_eq!(run(&root, false, &["script", "check", PATH]).0, 3);
}

#[test]
fn check_of_a_missing_script_is_a_failure_with_exit_code_two() {
    let (_guard, root) = hall_with(CLEAN);

    let (code, out, _) = run(&root, true, &["script", "check", ".ivar/scripts/nope.sh"]);

    assert_eq!(code, 2);
    assert_eq!(parsed(&out)["code"], "script.not_found");
}

#[test]
fn list_shows_the_approval_and_works_from_a_subdirectory_with_a_relative_path() {
    let (_guard, root) = hall_with(CLEAN);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    let sub = root.join("sub");

    let (code, _, _) = run(
        &sub,
        false,
        &["script", "approve", "../.ivar/scripts/deploy.sh"],
    );
    assert_eq!(code, 0);

    let (_, out, _) = run(&root, true, &["script", "list"]);
    assert_eq!(parsed(&out)["scripts"][0]["approval"], "approved");
    let (_, out, _) = run(&root, false, &["script", "list"]);
    assert!(out.contains("approved"), "{out}");
}

#[test]
fn doctor_warns_about_a_secret_and_about_an_env_file_reference() {
    let (_guard, root) = hall_with(&format!(
        "{CLEAN}export GH=ghp_0123456789abcdefghijklmnopqrstuvwxyzAB\nsource .env\n"
    ));

    let (_, out, _) = run(&root, true, &["doctor"]);

    let value = parsed(&out);
    let findings = value["findings"].as_array().unwrap();
    let find = |code: &str| findings.iter().find(|f| f["code"] == code).cloned();
    let secret = find("script.secret_found").unwrap_or_else(|| panic!("{value}"));
    assert!(
        secret["what"]
            .as_str()
            .unwrap()
            .contains("line 6 (github_token)")
    );
    let file = find("script.secret_file").unwrap_or_else(|| panic!("{value}"));
    assert!(file["what"].as_str().unwrap().contains("line 7 (`.env`)"));
}
