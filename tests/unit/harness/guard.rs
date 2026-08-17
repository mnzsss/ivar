//! Unit tests for `crate::harness::guard`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::infra::fs;
use crate::test_support::utf8_temp_dir;
use serde_json::Value;

fn feature() -> FeatureName {
    FeatureName::new("checkout").unwrap()
}

fn session() -> SessionId {
    SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap()
}

// -- Claude Code: the hook file -----------------------------------------

#[test]
fn claude_hook_is_written_executable() {
    let (_dir, view_dir) = utf8_temp_dir();
    let (_hall_dir, hall_root) = utf8_temp_dir();

    let hook_path = materialise(
        Provider::ClaudeCode,
        &view_dir,
        &hall_root,
        &feature(),
        &session(),
    )
    .unwrap();

    assert_eq!(
        hook_path,
        view_dir
            .join(".claude")
            .join("hooks")
            .join(CLAUDE_GUARD_SCRIPT)
    );
    assert!(fs::is_file(&hook_path).unwrap());

    let mode = fs::stat(&hook_path).unwrap().unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755, "hook must be mode 0755");

    let contents = fs::read_text(&hook_path).unwrap().unwrap();
    assert!(contents.starts_with("#!/usr/bin/env bash"));
    assert!(contents.contains("generated, do not edit"));
    assert!(contents.contains("ivar feature execute guard-check"));
    assert!(contents.contains(feature().as_str()));
    assert!(contents.contains(session().as_str()));
}

// -- Claude Code: settings.json merge ------------------------------------

#[test]
fn claude_settings_registration_merges_rather_than_clobbers() {
    let (_dir, view_dir) = utf8_temp_dir();
    let (_hall_dir, hall_root) = utf8_temp_dir();

    let config_dir = view_dir.join(".claude");
    fs::ensure_dir(&config_dir).unwrap();
    fs::write_text(
        &config_dir.join("settings.json"),
        r#"{
  "permissions": { "allow": ["Bash(ls)"] },
  "hooks": {
"PreToolUse": [
  { "matcher": "Bash", "hooks": [{ "type": "command", "command": "echo hi" }] }
]
  }
}"#,
    )
    .unwrap();

    materialise(
        Provider::ClaudeCode,
        &view_dir,
        &hall_root,
        &feature(),
        &session(),
    )
    .unwrap();

    let settings: Value = serde_json::from_str(
        &fs::read_text(&config_dir.join("settings.json"))
            .unwrap()
            .unwrap(),
    )
    .unwrap();

    // The unrelated top-level setting survives untouched.
    assert_eq!(
        settings["permissions"]["allow"][0].as_str(),
        Some("Bash(ls)"),
        "an unrelated existing setting must survive the merge"
    );

    let entries = settings["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "merge, not clobber: both entries present");

    assert!(
        entries
            .iter()
            .any(|entry| entry["matcher"] == "Bash" && entry["hooks"][0]["command"] == "echo hi"),
        "the pre-existing Bash hook must survive: {entries:?}"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry["matcher"] == WRITE_TOOL_MATCHER
                && entry["hooks"][0]["command"]
                    .as_str()
                    .is_some_and(|command| command.contains(CLAUDE_GUARD_SCRIPT))),
        "the generated guard must be registered: {entries:?}"
    );
}

#[test]
fn rematerialising_the_claude_guard_does_not_duplicate_the_registration() {
    let (_dir, view_dir) = utf8_temp_dir();
    let (_hall_dir, hall_root) = utf8_temp_dir();

    materialise(
        Provider::ClaudeCode,
        &view_dir,
        &hall_root,
        &feature(),
        &session(),
    )
    .unwrap();
    materialise(
        Provider::ClaudeCode,
        &view_dir,
        &hall_root,
        &feature(),
        &session(),
    )
    .unwrap();

    let settings: Value = serde_json::from_str(
        &fs::read_text(&view_dir.join(".claude").join("settings.json"))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let entries = settings["hooks"]["PreToolUse"].as_array().unwrap();
    let matching: Vec<_> = entries
        .iter()
        .filter(|entry| {
            entry["hooks"][0]["command"]
                .as_str()
                .is_some_and(|command| command.contains(CLAUDE_GUARD_SCRIPT))
        })
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "re-materialising must replace, not accumulate: {entries:?}"
    );
}

// -- Quoting --------------------------------------------------------------

#[test]
fn a_hall_path_with_a_space_and_a_quote_is_safely_single_quoted_in_the_claude_script() {
    let tricky = FeatureName::new("checkout").unwrap();
    let session_id = session();
    let hall = Utf8PathBuf::from("/tmp/my hall's dir");

    let script = render_claude_guard_script(&hall, &tricky, &session_id);

    // The naive, broken interpolation would end the shell string early at
    // the embedded quote. The safe form re-opens the quote around a
    // backslash-escaped literal quote.
    assert!(
        script.contains(r"HALL_PATH='/tmp/my hall'\''s dir'"),
        "hall path must be single-quote-escaped: {script}"
    );
    assert!(
        !script.contains("HALL_PATH='/tmp/my hall's dir'"),
        "an unescaped embedded quote must never appear in the generated script"
    );
}

#[test]
fn a_hall_path_with_a_quote_and_space_is_safely_json_escaped_in_the_opencode_plugin() {
    let hall = Utf8PathBuf::from("/tmp/my hall's dir");

    let plugin = render_opencode_guard_plugin(&hall, &feature(), &session());

    assert!(
        plugin.contains(r#"const hallPath = "/tmp/my hall's dir";"#),
        "hall path must be a valid, escaped JS string literal: {plugin}"
    );
}

// -- OpenCode: the plugin file --------------------------------------------

#[test]
fn opencode_plugin_is_written_with_the_hall_feature_and_session_baked_in() {
    let (_dir, view_dir) = utf8_temp_dir();
    let (_hall_dir, hall_root) = utf8_temp_dir();

    let plugin_path = materialise(
        Provider::OpenCode,
        &view_dir,
        &hall_root,
        &feature(),
        &session(),
    )
    .unwrap();

    assert_eq!(
        plugin_path,
        view_dir
            .join(".opencode")
            .join("plugins")
            .join(OPENCODE_GUARD_PLUGIN)
    );
    let contents = fs::read_text(&plugin_path).unwrap().unwrap();
    assert!(contents.contains("generated, do not edit"));
    assert!(contents.contains("tool.execute.before"));
    assert!(contents.contains("guard-check"));
    assert!(contents.contains(&format!(
        "const hallPath = {};",
        js_string_literal(hall_root.as_str())
    )));
    assert!(contents.contains(&js_string_literal(feature().as_str())));
    assert!(contents.contains(&js_string_literal(session().as_str())));

    // OpenCode's loader calls the export and takes the hook table from what
    // it returns. An export that *is* the table is refused with "Plugin
    // export is not a function", and a guard refused at load time arbitrates
    // nothing at all — silently, for the whole run.
    assert!(
        contents.contains("export const ivarExecutionGuard = async () => ("),
        "the guard must export a function, was:\n{contents}"
    );
    assert!(
        !contents.contains("export default {"),
        "the guard exports the hook table itself, which the loader refuses"
    );
    assert!(
        !contents.contains("hooks: {"),
        "the hooks are returned by the function, not nested under a `hooks` key"
    );

    // Not executable: it is loaded by OpenCode's plugin loader, never run
    // directly the way the Claude Code hook is.
    let mode = fs::stat(&plugin_path)
        .unwrap()
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o111,
        0,
        "the plugin file has no reason to be executable"
    );
}

// -- Settings merge: malformed existing content is leniently replaced ----

#[test]
fn a_corrupt_existing_settings_file_is_replaced_not_fatal() {
    let (_dir, view_dir) = utf8_temp_dir();
    let (_hall_dir, hall_root) = utf8_temp_dir();

    let config_dir = view_dir.join(".claude");
    fs::ensure_dir(&config_dir).unwrap();
    fs::write_text(&config_dir.join("settings.json"), "not json at all").unwrap();

    // Must still succeed — a session's guard is not held hostage by an
    // earlier malformed write.
    materialise(
        Provider::ClaudeCode,
        &view_dir,
        &hall_root,
        &feature(),
        &session(),
    )
    .unwrap();

    let settings: Value = serde_json::from_str(
        &fs::read_text(&config_dir.join("settings.json"))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(settings["hooks"]["PreToolUse"].is_array());
}

/// The OpenCode hook fires for *every* tool, and its first argument is the
/// call's identity — `{ tool, sessionID, callID }` — not its arguments. A
/// hook that ignored it and went straight for a path denied `bash` (which
/// has none) before a test could run, and sent `read` to `guard-check`,
/// which answers for the *write* contract: an executor could then neither
/// read the modules it was told to change nor prove its change worked, and
/// ran itself out of turns analysing the few files it could open. The
/// allowlist is what keeps the guard on the tools it is a guard for; the
/// tools it lets past are covered by the post-run audit in `tick`.
#[test]
fn the_opencode_plugin_arbitrates_only_direct_mutation_tools() {
    let (_hall_dir, hall_root) = utf8_temp_dir();

    let plugin = render_opencode_guard_plugin(&hall_root, &feature(), &session());

    assert!(
        plugin.contains(
            r#"const MUTATION_TOOLS = new Set(["write", "edit", "multiedit", "notebookedit"]);"#
        ),
        "the allowlist must name the mutation tools, lower-cased: {plugin}"
    );

    let gate = plugin
        .find("MUTATION_TOOLS.has")
        .expect("the hook must consult the allowlist");
    let extraction = plugin
        .find("args.filePath")
        .expect("the hook must still extract a path");
    assert!(
        gate < extraction,
        "a non-mutation tool must return before the pathless default-deny: {plugin}"
    );

    assert!(
        plugin.contains("input: { tool: string"),
        "the hook must read the documented input shape: {plugin}"
    );
    assert!(
        !plugin.contains("_input: unknown"),
        "the hook can no longer ignore which tool it was called for: {plugin}"
    );

    // Letting the other tools past narrows what the guard arbitrates; it
    // does not soften the answer for what it still does.
    assert!(
        plugin.contains("no path in the tool call — denying by default"),
        "a mutation without a path must still be denied: {plugin}"
    );
}
