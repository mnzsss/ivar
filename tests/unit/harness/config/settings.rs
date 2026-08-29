#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::name::HallName;
use crate::infra::fs;
use crate::test_support::utf8_temp_dir;

fn hall() -> HallName {
    HallName::new("acme").unwrap()
}

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

    let change = materialise_settings(&path, &hall()).unwrap();
    assert_eq!(change, Change::Updated);

    let doc: serde_json::Value =
        serde_json::from_str(&fs::read_text(&path).unwrap().unwrap()).unwrap();
    // ivar's keys are present.
    assert_eq!(doc["env"]["IVAR_HALL"], serde_json::json!("acme"));
    assert!(doc["hooks"]["SessionStart"].is_array());
    assert!(doc["hooks"]["PreToolUse"].is_array());
    // The user's keys survive byte-for-byte in shape.
    assert_eq!(doc["permissions"]["allow"][0], "Bash(npm run *)");
    assert_eq!(doc["permissions"]["deny"][0], "Read(./.env)");
    assert_eq!(doc["sandbox"]["image"], "node:20");
}

#[test]
fn materialise_is_idempotent() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");

    let first = materialise_settings(&path, &hall()).unwrap();
    assert_eq!(first, Change::Created);

    let second = materialise_settings(&path, &hall()).unwrap();
    assert_eq!(second, Change::Unchanged);
}

#[test]
fn remove_settings_deletes_file_when_only_ivar_keys() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("settings.json");
    materialise_settings(&path, &hall()).unwrap();

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
  "env": { "IVAR_HALL": "acme" },
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

    let result = materialise_settings(&path, &hall());
    assert!(result.is_err());
}
