//! Contract of `ivar plan lint`: exit `0` on a clean packet, exit `1` on a
//! packet with problems, and one warning with its own code per problem.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::common::{hall_root, ivar};
use camino::Utf8Path;

const VALID: &str = "### Task 1: Thing\n\
**Files:**\n\
- Create: `src/new.rs`\n\
**Readers:**\n\
- `git grep -n thing`\n\
\x20 ```\n\
\x20 no readers outside the declared files\n\
\x20 ```\n\
**Interfaces:**\n\
- Consumes: none\n\
- Produces: none\n\
- [ ] **Step 1: Write the failing test**\n";

fn hall_with_packet(packet: &str) -> (tempfile::TempDir, camino::Utf8PathBuf) {
    let (guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "checkout"])
        .assert()
        .success();
    ivar()
        .current_dir(&root)
        .args(["plan", "create", "checkout"])
        .assert()
        .success();
    let tasks = root.join(".ivar/features/checkout/tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(tasks.join("01-thing.md"), packet).unwrap();
    (guard, root)
}

fn lint_json(root: &Utf8Path, expected_exit: i32) -> serde_json::Value {
    let output = ivar()
        .current_dir(root)
        .args(["--json", "plan", "lint", "checkout"])
        .assert()
        .code(expected_exit)
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("valid json")
}

#[test]
fn a_valid_packet_exits_zero() {
    let (_guard, root) = hall_with_packet(VALID);

    let value = lint_json(&root, 0);

    assert_eq!(value["ok"], true);
    assert_eq!(value["problems"], 0);
    assert_eq!(value["packets"][0], "01-thing.md");
    assert!(value.get("warnings").is_none());
}

#[test]
fn an_invalid_packet_exits_one_with_a_code_per_problem() {
    let packet = VALID.replace(
        "  ```\n  no readers outside the declared files\n  ```\n",
        "",
    ) + "- [ ] **Step 3: Implement** TODO\n";
    let (_guard, root) = hall_with_packet(&packet);

    let value = lint_json(&root, 1);

    assert_eq!(value["problems"], 2);
    let codes: Vec<&str> = value["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        ["plan.lint.readers_without_output", "plan.lint.placeholder"]
    );
}

#[test]
fn the_human_surface_names_each_problem_on_stderr() {
    let (_guard, root) = hall_with_packet(&(VALID.to_owned() + "- [ ] **Step 2: Run** <NNN>\n"));

    let assert = ivar()
        .current_dir(&root)
        .args(["plan", "lint", "checkout"])
        .assert()
        .code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();

    assert!(
        stderr.contains("01-thing.md:13") && stderr.contains("plan.lint.number_placeholder"),
        "{stderr}"
    );
}

#[test]
fn a_feature_with_no_packets_is_refused() {
    let (_guard, root) = hall_with_packet(VALID);
    std::fs::remove_file(root.join(".ivar/features/checkout/tasks/01-thing.md")).unwrap();

    let value: serde_json::Value = serde_json::from_slice(
        &ivar()
            .current_dir(&root)
            .args(["--json", "plan", "lint", "checkout"])
            .assert()
            .code(2)
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();

    assert_eq!(value["code"], "plan.no_packets");
}
