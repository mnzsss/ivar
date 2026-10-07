#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::infra::fs;
use crate::test_support::utf8_temp_dir;

#[test]
fn materialise_preserves_user_permissions_and_sandbox() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");
    fs::write_text(
        &path,
        r#"{
  "permissions": {
    "allow": ["Bash(npm run *)"],
    "deny": ["Read(./.env)"]
  },
  "sandbox": {
    "image": "node:20"
  }
}"#,
    )
    .unwrap();

    let change = materialise_settings(&path).unwrap();
    assert_eq!(change, Change::Updated);

    let doc: serde_json::Value =
        serde_json::from_str(&fs::read_text(&path).unwrap().unwrap()).unwrap();
    assert!(doc.get("env").is_none(), "settings carry no env: {doc}");
    // Claude Code's schema: each entry is a matcher + a `hooks` array.
    for event in ["SessionStart", "PreToolUse"] {
        let entry = &doc["hooks"][event][0];
        assert!(
            entry["matcher"].is_string(),
            "{event} entry needs a matcher"
        );
        assert!(
            entry["hooks"][0]["command"].is_string(),
            "{event} entry needs a hooks array"
        );
    }
    // The user's keys survive byte-for-byte in shape.
    assert_eq!(doc["permissions"]["allow"][0], "Bash(npm run *)");
    assert_eq!(doc["permissions"]["deny"][0], "Read(./.env)");
    assert_eq!(doc["sandbox"]["image"], "node:20");
}

#[test]
fn materialise_is_idempotent() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");

    let first = materialise_settings(&path).unwrap();
    assert_eq!(first, Change::Created);

    let second = materialise_settings(&path).unwrap();
    assert_eq!(second, Change::Unchanged);
}

#[test]
fn remove_settings_deletes_file_when_only_ivar_keys() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");
    materialise_settings(&path).unwrap();

    let change = remove_settings(&path).unwrap();
    assert_eq!(change, Change::Removed);
    assert!(!path.exists());
}

#[test]
fn remove_settings_preserves_user_keys() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");
    fs::write_text(
        &path,
        r#"{
  "permissions": { "allow": ["Bash(npm run *)"] },
  "env": { "IVAR_HALL": "/tmp/acme" },
  "hooks": { "SessionStart": [] }
}"#,
    )
    .unwrap();

    let change = remove_settings(&path).unwrap();
    assert_eq!(change, Change::Removed);

    let doc: serde_json::Value =
        serde_json::from_str(&fs::read_text(&path).unwrap().unwrap()).unwrap();
    assert_eq!(doc["permissions"]["allow"][0], "Bash(npm run *)");
    assert!(doc.get("env").is_none());
    assert!(doc.get("hooks").is_none());
}

fn read_doc(path: &Utf8Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_text(path).unwrap().unwrap()).unwrap()
}

#[test]
fn materialise_writes_no_ivar_hall() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");

    materialise_settings(&path).unwrap();

    let doc = read_doc(&path);
    assert!(doc.pointer("/env/IVAR_HALL").is_none(), "{doc}");
    assert!(doc.get("env").is_none(), "{doc}");
}

#[test]
fn materialise_keeps_user_env_and_strips_legacy_ivar_hall() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");
    fs::write_text(&path, r#"{ "env": { "FOO": "1", "IVAR_HALL": "acme" } }"#).unwrap();

    assert_eq!(materialise_settings(&path).unwrap(), Change::Updated);

    assert_eq!(read_doc(&path)["env"], serde_json::json!({ "FOO": "1" }));
}

#[test]
fn materialise_drops_env_left_empty_by_legacy_ivar_hall() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");
    fs::write_text(&path, r#"{ "env": { "IVAR_HALL": "acme" } }"#).unwrap();

    materialise_settings(&path).unwrap();

    assert!(read_doc(&path).get("env").is_none());
}

#[test]
fn remove_settings_strips_legacy_ivar_hall_and_keeps_user_env() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");
    fs::write_text(&path, r#"{ "env": { "FOO": "1", "IVAR_HALL": "acme" } }"#).unwrap();

    assert_eq!(remove_settings(&path).unwrap(), Change::Removed);

    assert_eq!(read_doc(&path)["env"], serde_json::json!({ "FOO": "1" }));
}

#[test]
fn remove_settings_on_absent_file_is_unchanged() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");

    assert_eq!(remove_settings(&path).unwrap(), Change::Unchanged);
}

#[test]
fn a_non_object_file_is_refused() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");
    fs::write_text(&path, r#""just a string""#).unwrap();

    let result = materialise_settings(&path);
    assert!(result.is_err());
}

#[test]
fn materialise_turns_off_harness_attribution() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");
    fs::write_text(&path, r#"{ "attribution": { "commit": "x", "pr": "y" } }"#).unwrap();

    materialise_settings(&path).unwrap();

    let doc: serde_json::Value =
        serde_json::from_str(&fs::read_text(&path).unwrap().unwrap()).unwrap();
    assert_eq!(
        doc["attribution"],
        serde_json::json!({ "commit": "", "pr": "" })
    );
}

#[test]
fn remove_settings_drops_attribution_and_keeps_user_keys() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");
    fs::write_text(
        &path,
        r#"{ "permissions": { "allow": [] }, "attribution": { "commit": "", "pr": "" } }"#,
    )
    .unwrap();

    assert_eq!(remove_settings(&path).unwrap(), Change::Removed);

    let doc: serde_json::Value =
        serde_json::from_str(&fs::read_text(&path).unwrap().unwrap()).unwrap();
    assert!(doc.get("attribution").is_none());
    assert!(doc.get("permissions").is_some());
}

#[test]
fn pre_tool_use_runs_the_guard_then_five_instruction_slices() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");
    materialise_settings(&path).unwrap();

    let doc = read_doc(&path);
    let entries = doc["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(
        entries.len(),
        1,
        "one entry, so every command runs for the same call: {doc}"
    );
    assert_eq!(entries[0]["matcher"], "");
    let commands: Vec<&str> = entries[0]["hooks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hook| {
            assert_eq!(hook["type"], "command", "{hook}");
            hook["command"].as_str().unwrap()
        })
        .collect();
    assert_eq!(
        commands,
        [
            "ivar guard --provider claude-code",
            "ivar guard --provider claude-code --slice 1",
            "ivar guard --provider claude-code --slice 2",
            "ivar guard --provider claude-code --slice 3",
            "ivar guard --provider claude-code --slice 4",
            "ivar guard --provider claude-code --slice 5",
        ]
    );
}

#[test]
fn an_existing_single_guard_entry_is_rewritten() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");
    fs::write_text(
        &path,
        r#"{ "hooks": { "PreToolUse": [ { "matcher": "", "hooks": [ { "type": "command", "command": "ivar guard --provider claude-code" } ] } ] } }"#,
    )
    .unwrap();

    assert_eq!(materialise_settings(&path).unwrap(), Change::Updated);
    assert_eq!(
        read_doc(&path)["hooks"]["PreToolUse"][0]["hooks"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
}
