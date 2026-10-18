#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::common::{hall_root, ivar};

fn json(root: &camino::Utf8Path, args: &[&str], code: i32) -> serde_json::Value {
    let output = ivar()
        .current_dir(root)
        .arg("--json")
        .args(args)
        .assert()
        .code(code)
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("valid json")
}

#[test]
fn disable_then_enable_round_trips_through_json() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    json(&root, &["skill", "create", "demo", "--description", "d"], 0);
    json(&root, &["skill", "sync"], 0);

    let off = json(&root, &["skill", "disable", "demo"], 0);
    assert_eq!(off["enabled"], false);
    assert_eq!(off["changed"], true);
    assert!(!root.join(".claude/skills/demo").exists());
    json(&root, &["sync"], 0);
    json(&root, &["skill", "sync"], 0);
    assert!(!root.join(".claude/skills/demo").exists());

    let on = json(&root, &["skill", "enable", "demo"], 0);
    assert_eq!(on["enabled"], true);
    assert!(root.join(".claude/skills/demo").exists());
}

#[test]
fn toggling_an_unknown_skill_exits_two() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    let value = json(&root, &["skill", "disable", "ghost"], 2);

    assert_eq!(value["code"], "skill.not_found");
}
