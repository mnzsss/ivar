#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::common::{hall_root, ivar};

fn put(root: &camino::Utf8Path, folder: &str, file: &str, content: &str) {
    let dir = root.join(".ivar").join(folder);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(file), content).unwrap();
}

fn json(root: &camino::Utf8Path, args: &[&str]) -> serde_json::Value {
    let output = ivar()
        .current_dir(root)
        .arg("--json")
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("valid json")
}

#[test]
fn script_list_json_lists_both_folders_with_unknown_approval() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    put(
        &root,
        "scripts",
        "deploy.sh",
        "#!/usr/bin/env bash\n# description: Deploy.\n# usage: deploy.sh <env>\n# env: TOKEN, URL\n",
    );
    put(
        &root,
        "scripts-local",
        "mine.py",
        "# description: Mine.\n# usage: mine.py\n",
    );
    put(&root, "scripts", "broken.sh", "echo no header\n");

    let value = json(&root, &["script", "list"]);

    assert_eq!(value["ok"], true, "{value}");
    let scripts = value["scripts"].as_array().unwrap();
    assert_eq!(scripts.len(), 2, "{value}");
    assert_eq!(scripts[0]["name"], "mine");
    assert_eq!(scripts[0]["scope"], "personal");
    assert_eq!(scripts[0]["env"], serde_json::json!([]));
    assert_eq!(scripts[1]["name"], "deploy");
    assert_eq!(scripts[1]["scope"], "team");
    assert_eq!(scripts[1]["description"], "Deploy.");
    assert_eq!(scripts[1]["usage"], "deploy.sh <env>");
    assert_eq!(scripts[1]["env"], serde_json::json!(["TOKEN", "URL"]));
    assert!(scripts.iter().all(|s| s["approval"] == "unknown"));
    assert_eq!(value["invalid"].as_array().unwrap().len(), 1);
}

#[test]
fn script_list_in_a_fresh_hall_is_empty_and_human_output_says_so() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    assert_eq!(
        json(&root, &["script", "list"])["scripts"],
        serde_json::json!([])
    );
    ivar()
        .current_dir(&root)
        .args(["script", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("No scripts"));
}

#[test]
fn doctor_warns_about_a_malformed_script_without_failing() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    put(&root, "scripts", "broken.sh", "echo no header\n");
    put(&root, "scripts", "notes.txt", "hello\n");

    let value = json(&root, &["doctor"]);

    let codes: Vec<&str> = value["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"script.missing_description"), "{value}");
    assert!(codes.contains(&"script.unsupported_extension"), "{value}");
}

#[test]
fn init_gitignore_keeps_scripts_committable_and_scripts_local_ignored() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap()
    };
    if !git(&["rev-parse", "--is-inside-work-tree"])
        .status
        .success()
    {
        git(&["init", "-q"]);
    }
    put(&root, "scripts", "a.sh", "#!/bin/bash\n");
    put(&root, "scripts-local", "b.sh", "#!/bin/bash\n");

    assert!(
        !git(&["check-ignore", "-q", ".ivar/scripts/a.sh"])
            .status
            .success()
    );
    assert!(
        git(&["check-ignore", "-q", ".ivar/scripts-local/b.sh"])
            .status
            .success()
    );
}
