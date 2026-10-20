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

#[test]
#[cfg(target_os = "linux")]
#[expect(clippy::print_stderr, reason = "a Landlock-less kernel skips visibly")]
fn the_host_sandbox_lets_a_command_write_only_inside_its_roots() {
    let (_guard, root) = hall_root();
    let inside = root.join("inside");
    let outside = root.join("outside");
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let output = ivar()
        .current_dir(&root)
        .args([
            "--json",
            "agent",
            "sandbox",
            "--write",
            inside.as_str(),
            "--",
            "sh",
            "-c",
        ])
        .arg(format!(
            "echo ok > {inside}/kept; echo no > {outside}/leaked; true"
        ))
        .output()
        .unwrap();
    if String::from_utf8_lossy(&output.stdout).contains("agent.landlock_unavailable") {
        eprintln!("skipped: this kernel has no Landlock");
        return;
    }
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        std::fs::read_to_string(inside.join("kept")).unwrap(),
        "ok\n"
    );
    assert!(!outside.join("leaked").exists());
}
