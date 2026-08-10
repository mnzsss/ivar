//! Per-session execution guard materialisation: the artefact that arbitrates
//! every write an executor makes against its workstream's write contract.
//!
//! [`materialise`] writes one of two things into a session's view dir,
//! depending on [`Provider`]:
//!
//! - Claude Code: `<view_dir>/.claude/hooks/ivar-execution-guard.sh`,
//!   registered as a `PreToolUse` hook in `<view_dir>/.claude/settings.json`.
//! - OpenCode: `<view_dir>/.opencode/plugins/ivar-execution-guard.ts`,
//!   intercepting `tool.execute.before`.
//!
//! Both shell back into `ivar feature execute guard-check`
//! ([`crate::action::execute::guard_check`], read but not modified here) with
//! the feature, the session id, and the attempted path, and refuse the write
//! unless that command answers with an explicit `allowed: true`.
//!
//! # The default is deny
//!
//! Every branch a generated artefact can take — a tool call it cannot pull a
//! path out of, a `guard-check` invocation that cannot even run, a non-zero
//! exit, output that fails to parse, an `allowed` field that is present but
//! not `true` — ends in a refusal. Nothing here ever allows by omission; see
//! the safeguard in the plan and `guard_check`'s own module doc, which states
//! the same rule for the command side of this pair.
//!
//! One consequence of that rule is easy to miss by reading `guard_check.rs`
//! only for its exit behaviour: **every** answer it computes, including a
//! denial, returns cleanly — an unknown session, a path outside the
//! contract, and a missing board all render as `Ok(Report { allowed: false,
//! .. })`, not an `Err`. The process exit code is therefore `0` for an
//! allowed *and* a denied answer alike, and only a genuinely missing
//! argument (a caller bug, not a workstream's business) makes it non-zero.
//! A generated script that only checked `$?` would allow every write
//! `guard-check` managed to compute an answer for, denying solely on a
//! crash — the opposite of this module's contract. Both generated artefacts
//! therefore always inspect the `allowed` field of `guard-check`'s `--json`
//! output, and treat a non-zero exit as one more reason that field is
//! unavailable, not as the primary signal.
//!
//! # Why the hall path is baked in, not discovered
//!
//! [`crate::action::discover_hall`] finds a hall by walking up from the
//! current directory looking for `ivar.json`. That works when the process's
//! cwd is still inside the hall tree — but a generated hook or plugin runs
//! as a child of the executor process, whose shell cwd can have drifted
//! anywhere in the course of a session (the agent's own commands can `cd`
//! wherever they like). Relying on walk-up discovery would make the guard's
//! reliability hostage to wherever the agent happened to leave its shell,
//! which is exactly backwards for the one thing in a session that must not
//! be foolable. Both generated artefacts instead carry the hall root
//! resolved once, at materialisation time, as an absolute path baked into
//! the script/plugin text — a Claude Code script `cd`s into it before
//! shelling out; the OpenCode plugin passes it as the child process's `cwd`
//! directly. Neither `ivar` command line has a `--hall`/`--cwd` flag to pass
//! this any other way — see `cli::root::ExecuteGuardCheckArgs`, which has
//! only `--feature`, `--session` and `--path`.
//!
//! # Merge, never clobber
//!
//! `<view_dir>/.claude/settings.json` is not this module's file alone — a
//! user, or the harness itself, may have written permissions or other hooks
//! into it. [`register_claude_hook`] reads whatever is there, replaces only
//! the `PreToolUse` entry that already targets this generated script (so
//! re-materialising a session, which happens on every `session connect`, is
//! idempotent rather than accumulating duplicates), and leaves every other
//! key untouched. A file that exists but fails to parse as a JSON object is
//! treated the same as no file at all: there is nothing coherent to merge
//! into, and a session's guard must still materialise rather than be held
//! hostage by an earlier malformed write. OpenCode needs no equivalent
//! registration step — its plugin loader discovers everything under
//! `.opencode/plugins/` on its own.
//!
//! # Rejected: reusing `Harness` from `super`
//!
//! The generated artefact only needs to know which dotdir and which template
//! to use, which is exactly [`Provider::config_dir`]. Keying this module off
//! `Provider` instead of the sibling [`super::Harness`] enum keeps it usable
//! wherever a `Provider` is already at hand (as it is on `SessionState`)
//! without a conversion, and keeps this module's compile-time surface
//! independent of the process-spawning concerns `Harness` owns.

use camino::{Utf8Path, Utf8PathBuf};
use serde_json::{Map, Value};

use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::infra::fs;

/// Filename of the generated Claude Code guard hook, under
/// `<view_dir>/.claude/hooks/`.
pub const CLAUDE_GUARD_SCRIPT: &str = "ivar-execution-guard.sh";

/// Filename of the generated OpenCode guard plugin, under
/// `<view_dir>/.opencode/plugins/`.
pub const OPENCODE_GUARD_PLUGIN: &str = "ivar-execution-guard.ts";

/// Tool names whose calls carry a path the guard must arbitrate.
const WRITE_TOOL_MATCHER: &str = "Write|Edit|MultiEdit|NotebookEdit";

/// Materialise the execution guard for `provider` into `view_dir`, so that
/// every write the executor's harness attempts is arbitrated against
/// `feature`/`session_id`'s workstream on the board.
///
/// `hall_root` is the absolute hall path baked into the generated artefact —
/// see the module doc for why it cannot instead be discovered at guard-check
/// time. Returns the path to the artefact written (the hook script for
/// Claude Code, the plugin file for OpenCode), matching what the two
/// TypeScript predecessors this ports return.
pub fn materialise(
    provider: Provider,
    view_dir: &Utf8Path,
    hall_root: &Utf8Path,
    feature: &FeatureName,
    session_id: &SessionId,
) -> Result<Utf8PathBuf, Failure> {
    match provider {
        Provider::ClaudeCode => materialise_claude_guard(view_dir, hall_root, feature, session_id),
        Provider::OpenCode => materialise_opencode_guard(view_dir, hall_root, feature, session_id),
    }
}

// -- Claude Code --------------------------------------------------------

fn materialise_claude_guard(
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
fn render_claude_guard_script(
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

// -- OpenCode -------------------------------------------------------------

fn materialise_opencode_guard(
    view_dir: &Utf8Path,
    hall_root: &Utf8Path,
    feature: &FeatureName,
    session_id: &SessionId,
) -> Result<Utf8PathBuf, Failure> {
    let plugins_dir = view_dir
        .join(Provider::OpenCode.config_dir())
        .join("plugins");
    fs::ensure_dir(&plugins_dir)?;

    let plugin_path = plugins_dir.join(OPENCODE_GUARD_PLUGIN);
    let plugin = render_opencode_guard_plugin(hall_root, feature, session_id);
    fs::write_text(&plugin_path, &plugin)?;

    Ok(plugin_path)
}

/// Render a JS string literal for `value`, safe to splice into the generated
/// TypeScript plugin verbatim. JSON string-literal syntax and JS
/// double-quoted string-literal syntax agree on every escape this needs
/// (quotes, backslashes, control characters), so `serde_json`'s own escaping
/// is reused rather than hand-rolling a second one.
fn js_string_literal(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

/// Render the OpenCode guard plugin, with `hall_root`, `feature` and
/// `session_id` baked in. Built from placeholder tokens and `.replace`,
/// rather than `format!`, because the template's TypeScript is dense with
/// literal `{`/`}` that `format!` would otherwise require doubling
/// throughout — a much easier place to introduce a silent mismatch than the
/// three tokens replaced here.
fn render_opencode_guard_plugin(
    hall_root: &Utf8Path,
    feature: &FeatureName,
    session_id: &SessionId,
) -> String {
    const TEMPLATE: &str = r#"/**
 * ivar execution guard — generated, do not edit.
 * Regenerated on every session materialisation, for feature "__FEATURE_NAME__",
 * session "__SESSION_NAME__". The default is deny: a tool call this hook
 * cannot pull a path out of, a `guard-check` it cannot even run, a non-zero
 * exit, or any answer other than an explicit `allowed: true` are all
 * refused — never allowed by omission.
 */
export default {
  name: 'ivar-execution-guard',
  hooks: {
    'tool.execute.before': async (
      _input: unknown,
      output: { args?: Record<string, unknown> },
    ) => {
      const args = output?.args ?? {};
      const filePath =
        (typeof args.filePath === 'string' && args.filePath) ||
        (typeof args.file_path === 'string' && args.file_path) ||
        (typeof args.path === 'string' && args.path) ||
        undefined;

      if (!filePath) {
        throw new Error('ivar execution guard: no path in the tool call — denying by default');
      }

      const hallPath = __HALL_JSON__;
      const feature = __FEATURE_JSON__;
      const sessionId = __SESSION_JSON__;

      let exitCode = 1;
      let stdout = '';
      let stderr = '';
      try {
        const proc = Bun.spawn(
          [
            'ivar',
            'feature',
            'execute',
            'guard-check',
            '--feature',
            feature,
            '--session',
            sessionId,
            '--path',
            filePath,
            '--json',
          ],
          { cwd: hallPath, stdout: 'pipe', stderr: 'pipe' },
        );
        exitCode = await proc.exited;
        stdout = await new Response(proc.stdout).text();
        stderr = await new Response(proc.stderr).text();
      } catch (error) {
        stderr = error instanceof Error ? error.message : String(error);
      }

      let allowed = false;
      if (exitCode === 0) {
        try {
          allowed = JSON.parse(stdout.trim()).allowed === true;
        } catch {
          allowed = false;
        }
      }

      if (!allowed) {
        throw new Error(`ivar denied write to ${filePath}: ${stderr.trim() || stdout.trim()}`);
      }
    },
  },
};
"#;

    TEMPLATE
        .replace("__FEATURE_NAME__", feature.as_str())
        .replace("__SESSION_NAME__", session_id.as_str())
        .replace("__HALL_JSON__", &js_string_literal(hall_root.as_str()))
        .replace("__FEATURE_JSON__", &js_string_literal(feature.as_str()))
        .replace("__SESSION_JSON__", &js_string_literal(session_id.as_str()))
}

#[cfg(test)]
#[path = "../../tests/unit/harness/guard.rs"]
mod tests;
