use camino::Utf8PathBuf;

use crate::providers::ManagedArtifact;

/// The embedded plugin source: a single file OpenCode loads from
/// `.opencode/plugins/`. Idempotent — written only when bytes on disk differ.
///
/// Hook signatures (from `opencode.ai/docs/plugins`):
/// - `shell.env(input, output)` — `input.cwd` is the working directory;
///   `output.env` is a mutable object; set keys on it to inject env vars.
/// - `tool.execute.before(input, output)` — `input.tool` is the tool name;
///   `output.args` are the arguments. Throw to block execution.
pub const OPENCODE_PLUGIN: &str = r#"// ivar session plugin for OpenCode
// Materialised by `ivar sync`. Do not edit.

export default {
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
      cwd: input.cwd || output.cwd || "",
    });
    execSync("ivar guard --provider opencode", {
      input: payload,
      encoding: "utf-8",
      stdio: ["pipe", "pipe", "pipe"],
    });
  },
};
"#;

pub(crate) fn managed_artifacts() -> Vec<ManagedArtifact> {
    vec![ManagedArtifact {
        relative_path: Utf8PathBuf::from(".opencode/plugins/ivar.js"),
        contents: OPENCODE_PLUGIN,
    }]
}
