#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Executes the embedded `.opencode/plugins/ivar.js` in `node` with a fake
//! `ivar` first on `PATH`, so the tests observe what OpenCode's `shell.env`
//! hook does when `ivar session env` succeeds or fails.

use std::os::unix::fs::PermissionsExt;

use serde_json::{Value, json};

use crate::domain::provider::Provider;
use crate::providers;
use crate::test_support::utf8_temp_dir;

const HARNESS: &str = r#"
import { pathToFileURL } from "node:url";

const [, , pluginPath] = process.argv;
const { IvarPlugin } = await import(pathToFileURL(pluginPath).href);
const hooks = await IvarPlugin();
const output = { env: {} };
let threw = null;
try {
  await hooks["shell.env"]({ cwd: "/somewhere/in/the/hall" }, output);
} catch (error) {
  threw = String(error?.message ?? error);
}
process.stdout.write(JSON.stringify({ threw, env: output.env }));
"#;

const NOT_IN_SESSION: &str = r#"{"ok":false,"kind":"blocked","code":"session.not_in_session","what":"not inside an active session view directory"}"#;

/// Runs the plugin's `shell.env` hook with an `ivar` that prints `stdout`,
/// writes `stderr` and exits with `status`.
fn run_shell_env(stdout: &str, stderr: &str, status: i32) -> Value {
    let artifacts = providers::managed_artifacts(Provider::OpenCode);
    let plugin = artifacts
        .iter()
        .find(|a| a.relative_path.as_str() == ".opencode/plugins/ivar.js")
        .expect("OpenCode declares the plugin artifact");

    let (_guard, root) = utf8_temp_dir();
    let plugin_path = root.join("ivar.js");
    std::fs::write(&plugin_path, plugin.contents).unwrap();
    let harness = root.join("harness.mjs");
    std::fs::write(&harness, HARNESS).unwrap();

    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let ivar = bin.join("ivar");
    std::fs::write(
        &ivar,
        format!("#!/bin/sh\nprintf '%s' '{stdout}'\nprintf '%s' '{stderr}' >&2\nexit {status}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&ivar, std::fs::Permissions::from_mode(0o755)).unwrap();

    let path = format!(
        "{bin}:{}",
        std::env::var("PATH").expect("PATH is set for the test process")
    );
    let output = std::process::Command::new("node")
        .arg(&harness)
        .arg(&plugin_path)
        .env("PATH", path)
        .output()
        .unwrap_or_else(|e| panic!("node is required to test the embedded OpenCode plugin: {e}"));
    assert!(
        output.status.success(),
        "plugin harness failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn shell_env_leaves_the_shell_alone_outside_a_session() {
    let out = run_shell_env(NOT_IN_SESSION, "", 2);

    assert_eq!(
        out["threw"],
        Value::Null,
        "a shell outside a session must still run"
    );
    assert_eq!(out["env"], json!({}));
}

#[test]
fn shell_env_injects_the_session_environment_inside_a_session() {
    let out = run_shell_env(
        r#"{"ok":true,"IVAR_SESSION_ID":"s1","IVAR_FEATURE":"demo"}"#,
        "",
        0,
    );

    assert_eq!(out["threw"], Value::Null);
    assert_eq!(out["env"]["IVAR_SESSION_ID"], "s1");
    assert_eq!(out["env"]["IVAR_FEATURE"], "demo");
}

#[test]
fn shell_env_still_surfaces_any_other_ivar_failure() {
    let out = run_shell_env(
        r#"{"ok":false,"kind":"failed","code":"store.unreadable","what":"state.json is unreadable"}"#,
        "",
        1,
    );

    assert_ne!(
        out["threw"],
        Value::Null,
        "only session.not_in_session is expected outside a session"
    );
    assert_eq!(out["env"], json!({}));
}

#[test]
fn shell_env_still_surfaces_a_failure_without_json_output() {
    let out = run_shell_env("not json", "boom", 1);

    assert!(
        out["threw"]
            .as_str()
            .is_some_and(|m| m.starts_with("Command failed")),
        "the original execSync error must surface, got {}",
        out["threw"]
    );
    assert_eq!(out["env"], json!({}));
}
