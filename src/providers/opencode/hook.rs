use camino::Utf8Path;

use crate::domain::provider::Provider;
use crate::providers::ManagedArtifact;

/// The embedded plugin source: a single file OpenCode loads from
/// `.opencode/plugins/`. Idempotent — written only when bytes on disk differ.
///
/// Hook signatures (from `opencode.ai/docs/plugins`):
/// - `shell.env(input, output)` — `input.cwd` is the working directory;
///   `output.env` is a mutable object; set keys on it to inject env vars.
///   Outside a session (`session.not_in_session`) it leaves the env alone;
///   any other failure still throws.
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
    let result;
    try {
      result = execSync(
        `ivar session env --json --cwd ${JSON.stringify(input.cwd)}`,
        { encoding: "utf-8" }
      );
    } catch (error) {
      // Throwing here fails every shell command, and most run outside a session.
      let code;
      try {
        code = JSON.parse(error.stdout).code;
      } catch {}
      if (code === "session.not_in_session") {
        return;
      }
      throw error;
    }
    const env = JSON.parse(result);
    for (const [key, value] of Object.entries(env)) {
      if (key.startsWith("IVAR_")) {
        output.env[key] = value;
      }
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

pub const OPENCODE_CAPTURE_PLUGIN: &str = r#"// ivar memory capture plugin for OpenCode
// Materialised by `ivar sync`. Do not edit.

import { spawn } from "node:child_process";

// Bounds the work done inside the host process; `ivar` cuts every body to 4096 bytes anyway.
const MAX_STRING = 16384;
const MAX_ENTRIES = 32;
const MAX_DEPTH = 4;

function clip(value, depth = 0) {
  if (typeof value === "string") {
    const head = value.slice(0, MAX_STRING);
    return /[\ud800-\udbff]$/.test(head) ? head.slice(0, -1) : head;
  }
  if (value === null || typeof value !== "object") return value;
  if (depth >= MAX_DEPTH) return "";
  if (Array.isArray(value)) return value.slice(0, MAX_ENTRIES).map((item) => clip(item, depth + 1));
  const clipped = {};
  for (const key of Object.keys(value).slice(0, MAX_ENTRIES)) clipped[key] = clip(value[key], depth + 1);
  return clipped;
}

// ponytail: one detached process per event; a long-lived socket if the per-event spawn cost shows up in latency.
function send(payload) {
  try {
    const child = spawn("ivar", ["memory", "capture", "--provider", "opencode"], {
      stdio: ["pipe", "ignore", "ignore"],
      detached: true,
    });
    child.on("error", () => {});
    child.stdin.on("error", () => {});
    child.stdin.end(JSON.stringify(clip(payload)));
    child.unref();
  } catch {}
}

export const IvarMemoryPlugin = async () => ({
  event: async ({ event }) => {
    try {
      if (event?.type === "session.created") {
        send({ event: "started", session: event.properties?.info?.id ?? "", cwd: event.properties?.info?.directory ?? process.cwd() });
      }
    } catch {}
  },
  "chat.message": async (input, output) => {
    try {
      const text = (output?.parts ?? []).filter((part) => part.type === "text").map((part) => part.text).join("\n");
      send({ event: "prompted", session: input?.sessionID ?? "", cwd: process.cwd(), text });
    } catch {}
  },
  "tool.execute.after": async (input, output) => {
    try {
      send({
        event: "tool",
        session: input?.sessionID ?? "",
        cwd: process.cwd(),
        tool: input?.tool,
        args: input?.args ?? {},
        failed: false,
        result: typeof output?.output === "string" ? output.output : "",
      });
    } catch {}
  },
});
"#;

pub(crate) fn capture_artifacts() -> Vec<ManagedArtifact> {
    vec![ManagedArtifact {
        relative_path: Utf8Path::new(Provider::OPENCODE_PLUGINS_DIR).join("ivar-memory.js"),
        contents: OPENCODE_CAPTURE_PLUGIN,
    }]
}
