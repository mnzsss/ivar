//! Claude Code guard materialisation: the generated hook script and its
//! `settings.json` registration.
//!
//! See the module doc in `mod.rs` for the guard's contract — every branch
//! ends in a refusal, never an allow by omission.

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::{Map, Value};

use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::infra::fs;

use super::CLAUDE_GUARD_SCRIPT;

/// Tool names whose calls carry a path the guard must arbitrate.
pub(crate) const WRITE_TOOL_MATCHER: &str = "Write|Edit|MultiEdit|NotebookEdit";

/// Materialise the Claude Code guard hook into `view_dir` and register it as
/// a `PreToolUse` hook in the session's `settings.json`.
pub(super) fn materialise(
    view_dir: &Utf8Path,
    hall_root: &Utf8Path,
    feature: &FeatureName,
    session_id: &SessionId,
) -> Result<Utf8PathBuf, Failure> {
    let hooks_dir = view_dir
        .join(Provider::ClaudeCode.config_dir())
        .join("hooks");
    fs::ensure_dir(&hooks_dir)?;

    let hook_path = hooks_dir.join(CLAUDE_GUARD_SCRIPT);
    let script = render_claude_guard_script(hall_root, feature, session_id);
    fs::write_text(&hook_path, &script)?;
    // The hook is invoked directly by Claude Code, not through `bash script`
    // in `settings.json`'s command — wait, it *is* invoked through `bash
    // "$path"` (see `hook_command`), so execute bits are not strictly load
    // bearing for Claude Code's own invocation, but are set anyway: the file
    // is a shebang script that should be runnable on its own for debugging,
    // and a generated artefact that looks executable but is not would be a
    // confusing thing to hand a human investigating a denial.
    fs::chmod(&hook_path, 0o755)?;

    register_claude_hook(view_dir, CLAUDE_GUARD_SCRIPT, WRITE_TOOL_MATCHER)?;

    Ok(hook_path)
}

/// Single-quote `value` for safe interpolation into generated shell text.
/// The one escape a single-quoted shell string needs: close the quote,
/// emit a literal quote via backslash-escape *outside* any quoting, reopen
/// the quote. This is the only interpolation discipline the Claude script
/// needs, because every value it embeds is assigned to a shell variable
/// inside single quotes, never spliced into a larger command unquoted.
fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Render the Claude Code guard hook script, with `hall_root`, `feature` and
/// `session_id` baked in.
///
/// The Claude Code hook contract: the tool call arrives as JSON on stdin,
/// never as an argument; exiting `2` blocks the call and feeds stderr back to
/// the model; any other non-zero exit is a hook *error* and does not block —
/// which is exactly why every failure path below ends in `exit 2`, deliberately,
/// rather than an unhandled crash that Claude Code would treat as "hook broke,
/// carry on".
pub(crate) fn render_claude_guard_script(
    hall_root: &Utf8Path,
    feature: &FeatureName,
    session_id: &SessionId,
) -> String {
    let hall_quoted = shell_single_quote(hall_root.as_str());
    let feature_quoted = shell_single_quote(feature.as_str());
    let session_quoted = shell_single_quote(session_id.as_str());

    format!(
        r#"#!/usr/bin/env bash
# ivar execution guard — generated, do not edit.
# Regenerated on every session materialisation, for feature "{feature_name}",
# session "{session_name}". The default is deny: a tool call this hook cannot
# pull a path out of, a `guard-check` it cannot even run, a non-zero exit, or
# any answer other than an explicit `allowed: true` are all refused — never
# allowed by omission.
set -uo pipefail

INPUT="$(cat)"

if command -v jq >/dev/null 2>&1; then
  file_path="$(printf '%s' "$INPUT" | jq -r '.tool_input.file_path // .tool_input.notebook_path // .tool_input.path // ""' 2>/dev/null)"
else
  file_path="$(printf '%s' "$INPUT" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
fi

if [ -z "$file_path" ]; then
  printf 'ivar execution guard: no path in the tool call — denying by default\n' >&2
  exit 2
fi

HALL_PATH={hall_quoted}
FEATURE={feature_quoted}
SESSION_ID={session_quoted}

RESULT="$(cd "$HALL_PATH" 2>/dev/null && ivar feature execute guard-check --feature "$FEATURE" --session "$SESSION_ID" --path "$file_path" --json 2>&1)"
STATUS=$?

ALLOWED="false"
if [ "$STATUS" -eq 0 ]; then
  if command -v jq >/dev/null 2>&1; then
    ALLOWED="$(printf '%s' "$RESULT" | jq -r 'if .allowed == true then "true" else "false" end' 2>/dev/null)"
  else
    case "$RESULT" in
      *'"allowed":true'*) ALLOWED="true" ;;
    esac
  fi
fi

if [ "$ALLOWED" != "true" ]; then
  printf 'ivar denied write to %s: %s\n' "$file_path" "$RESULT" >&2
  exit 2
fi

exit 0
"#,
        feature_name = feature.as_str(),
        session_name = session_id.as_str(),
        hall_quoted = hall_quoted,
        feature_quoted = feature_quoted,
        session_quoted = session_quoted,
    )
}

/// The command Claude Code runs for the generated hook. `CLAUDE_PROJECT_DIR`
/// is expanded by Claude Code itself to the directory it was launched in —
/// the view dir — so the registration keeps working if the view dir is ever
/// moved, without this module needing to know its final location.
fn claude_hook_command(script: &str) -> String {
    format!(
        "bash \"$CLAUDE_PROJECT_DIR/{}/hooks/{script}\"",
        Provider::ClaudeCode.config_dir()
    )
}

/// Register `script` as a `PreToolUse` hook in `<view_dir>/.claude/settings.json`,
/// merging with whatever is already there — see the module doc's "Merge,
/// never clobber" section.
fn register_claude_hook(view_dir: &Utf8Path, script: &str, matcher: &str) -> Result<(), Failure> {
    let path = view_dir
        .join(Provider::ClaudeCode.config_dir())
        .join("settings.json");
    let existing = fs::read_text(&path)?;
    let command = claude_hook_command(script);
    let rendered = merge_claude_settings(existing.as_deref(), script, matcher, &command)?;
    fs::write_text(&path, &rendered)?;
    Ok(())
}

/// Fold a `PreToolUse` registration for `script` into `existing` settings
/// JSON text, replacing only a previous registration of the same script.
///
/// `existing` missing, unparsable, or not a JSON object are all treated as
/// "start from `{}`" — see the module doc for why that is a deliberate
/// leniency, not an oversight.
fn merge_claude_settings(
    existing: Option<&str>,
    script: &str,
    matcher: &str,
    command: &str,
) -> Result<String, Failure> {
    let mut root: Map<String, Value> = existing
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default();

    let mut hooks: Map<String, Value> = match root.remove("hooks") {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };

    let pre_tool_use: Vec<Value> = match hooks.remove("PreToolUse") {
        Some(Value::Array(entries)) => entries,
        _ => Vec::new(),
    };

    let mut kept: Vec<Value> = pre_tool_use
        .into_iter()
        .filter(|entry| !claude_entry_targets_script(entry, script))
        .collect();

    kept.push(serde_json::json!({
        "matcher": matcher,
        "hooks": [{ "type": "command", "command": command }],
    }));

    hooks.insert("PreToolUse".to_owned(), Value::Array(kept));
    root.insert("hooks".to_owned(), Value::Object(hooks));

    serde_json::to_string_pretty(&Value::Object(root)).map_err(|source| {
        Failure::failed(
            "harness.guard_settings_render_failed",
            format!("could not render settings.json: {source}"),
        )
    })
}

/// Does this `PreToolUse` entry already point at `script`?
fn claude_entry_targets_script(entry: &Value, script: &str) -> bool {
    let Some(hooks) = entry.get("hooks").and_then(Value::as_array) else {
        return false;
    };
    hooks.iter().any(|hook| {
        hook.get("command")
            .and_then(Value::as_str)
            .is_some_and(|command| command.contains(script))
    })
}
