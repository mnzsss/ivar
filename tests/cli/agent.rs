#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::common::{hall_root, ivar};

#[test]
fn status_lists_agents_as_json() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(
        root.join("agents/nightly.md"),
        "---\nschedule: \"@daily\"\ntool: claude\n---\nGo.\n",
    )
    .unwrap();

    let output = ivar()
        .current_dir(&root)
        .args(["--json", "agent", "status"])
        .env("XDG_CONFIG_HOME", root.join("xdg").as_str())
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["agents"][0]["name"], "nightly");
    assert_eq!(value["agents"][0]["on_calendar"], "*-*-* 00:00:00");
    assert_eq!(value["agents"][0]["sync"]["state"], "not_installed");
}

#[test]
fn an_invalid_agent_fails_run_with_exit_code_and_field() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(
        root.join("agents/bad.md"),
        "---\nschedule: \"@reboot\"\ntool: claude\n---\n",
    )
    .unwrap();
    let output = ivar()
        .current_dir(&root)
        .args(["--json", "agent", "run", "bad"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["code"], "agent.invalid");
    assert!(value["what"].as_str().unwrap().contains("`schedule`"));
}
