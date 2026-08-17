//! OpenCode guard materialisation: the generated TypeScript plugin.
//!
//! See the module doc in `mod.rs` for the guard's contract — every branch
//! ends in a refusal, never an allow by omission.

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::infra::fs;

use super::OPENCODE_GUARD_PLUGIN;

/// Materialise the OpenCode guard plugin into `view_dir`.
///
/// OpenCode's plugin loader discovers everything under `.opencode/plugins/`
/// on its own, so unlike Claude Code there is no registration step.
pub(super) fn materialise(
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
pub(crate) fn js_string_literal(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

/// Render the OpenCode guard plugin, with `hall_root`, `feature` and
/// `session_id` baked in. Built from placeholder tokens and `.replace`,
/// rather than `format!`, because the template's TypeScript is dense with
/// literal `{`/`}` that `format!` would otherwise require doubling
/// throughout — a much easier place to introduce a silent mismatch than the
/// three tokens replaced here.
pub(crate) fn render_opencode_guard_plugin(
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
export const ivarExecutionGuard = async () => ({
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
});
"#;

    TEMPLATE
        .replace("__FEATURE_NAME__", feature.as_str())
        .replace("__SESSION_NAME__", session_id.as_str())
        .replace("__HALL_JSON__", &js_string_literal(hall_root.as_str()))
        .replace("__FEATURE_JSON__", &js_string_literal(feature.as_str()))
        .replace("__SESSION_JSON__", &js_string_literal(session_id.as_str()))
}
