use camino::Utf8Path;

use crate::domain::provider::Provider;
use crate::providers::ManagedArtifact;

/// The embedded plugin source: a single file OpenCode loads from
/// `.opencode/plugins/`. Idempotent — written only when bytes on disk differ.
///
/// Hook signatures (from `opencode.ai/docs/plugins`):
/// - `shell.env(input, output)` — `input.cwd` is the working directory;
///   `output.env` is a mutable object; set keys on it to inject env vars.
/// - `tool.execute.before(input, output)` — `input.tool` is the tool name;
///   `output.args` are the arguments. Throw to block execution.
/// - `tool.execute.before(input, output)` also carries `input.sessionID`
///   (the agent key; subagents get their own) and `input.callID`. The payload
///   `cwd` is empty in practice, so the plugin falls back to `process.cwd()`.
///   On allow, `ivar guard` prints the repository instructions; they are kept
///   per `callID`.
/// - `tool.execute.after(input, output)` — same `input.callID`;
///   `output.output` is the tool result text the model sees, so the kept
///   instructions are appended to it inside `<system-reminder>`.
/// - Module shape: opencode 1.18 loads a named export of an async plugin
///   function (`export const IvarPlugin = async () => ({ …hooks })`); it
///   ignores a default-exported hooks object, which is why the earlier
///   `export default { … }` plugin never ran.
pub const OPENCODE_PLUGIN: &str = r#"// ivar session plugin for OpenCode
// Materialised by `ivar sync`. Do not edit.

// Repository instructions `ivar guard` printed in `tool.execute.before`,
// kept per call until `tool.execute.after` appends them to the tool output.
const contexts = new Map();

export const IvarPlugin = async () => ({
  "shell.env": async (input, output) => {
    const { execSync } = await import("child_process");
    const result = execSync(
      `ivar session env --json --cwd ${JSON.stringify(input.cwd)}`,
      { encoding: "utf-8" }
    );
    const env = JSON.parse(result);
    for (const [key, value] of Object.entries(env)) {
      output.env[key] = value;
    }
  },

  "tool.execute.before": async (input, output) => {
    const { execSync } = await import("child_process");
    const payload = JSON.stringify({
      tool: input.tool,
      args: output.args,
      cwd: input.cwd || output.cwd || process.cwd(),
      agent: input.sessionID,
    });
    const context = execSync("ivar guard --provider opencode", {
      input: payload,
      encoding: "utf-8",
      stdio: ["pipe", "pipe", "pipe"],
    });
    if (input.callID && context && context.trim()) {
      contexts.set(input.callID, context);
    }
  },

  "tool.execute.after": async (input, output) => {
    const context = contexts.get(input.callID);
    if (context === undefined) {
      return;
    }
    contexts.delete(input.callID);
    if (typeof output.output === "string") {
      output.output += `\n\n<system-reminder>\n${context}\n</system-reminder>`;
    }
  },
});
"#;

pub(crate) fn managed_artifacts() -> Vec<ManagedArtifact> {
    vec![ManagedArtifact {
        relative_path: Utf8Path::new(Provider::OPENCODE_PLUGINS_DIR).join("ivar.js"),
        contents: OPENCODE_PLUGIN,
    }]
}
