use camino::Utf8Path;

use crate::domain::provider::Provider;
use crate::providers::ManagedArtifact;

/// Embedded plain JavaScript pre-tool hook for OMP.
///
/// OMP discovers `.omp/hooks/pre/*.js` and loads each one **in-process** as
/// an ES module whose default export is a factory receiving the hook API
/// (`docs/hooks.md`, `docs/skills/authoring-hooks.md`, measured against
/// omp/18.1.8). A handler registered on `tool_call` blocks execution by
/// returning `{ block: true, reason }`; `reason` becomes the tool error the
/// model sees. The hook is therefore a module, never a stdin/stdout filter.
///
/// The decision itself stays in `ivar guard`, so all three providers share
/// one policy: the hook shells out and translates a non-zero exit into the
/// structured verdict. `guard` writes the denial reason to stdout and exits
/// non-zero (`src/bin/ivar.rs`), which is what the `catch` reads. On allow
/// its stdout is the repository instructions for the touched paths; the hook
/// returns them as `{ additionalContext }`, which omp adds to the tool call.
/// `agent` (`ctx.agent.id`) keys delivery per subagent.
pub const OMP_HOOK: &str = r#"// ivar pre-tool guard hook for OMP
// Materialised by `ivar sync`. Do not edit.

import { execFileSync } from "node:child_process";

export default function ivarGuard(pi) {
  pi.on("tool_call", async (event, ctx) => {
    const payload = JSON.stringify({
      tool: event.toolName,
      args: event.input ?? {},
      cwd: ctx?.cwd ?? process.cwd(),
      agent: ctx?.agent?.id ?? "main",
    });

    let out;
    try {
      out = execFileSync("ivar", ["guard", "--provider", "omp"], {
        input: payload,
        encoding: "utf-8",
        stdio: ["pipe", "pipe", "pipe"],
      });
    } catch (err) {
      const stdout = (err.stdout || "").toString().trim();
      const stderr = (err.stderr || "").toString().trim();
      const reason = stdout || stderr || "Write blocked by ivar guard policy";
      return { block: true, reason };
    }
    if (out && out.trim()) {
      return { additionalContext: out };
    }
  });
}
"#;

pub(crate) fn managed_artifacts() -> Vec<ManagedArtifact> {
    vec![ManagedArtifact {
        relative_path: Utf8Path::new(Provider::OMP_HOOKS_DIR).join("pre/ivar.js"),
        contents: OMP_HOOK,
    }]
}

pub const OMP_CAPTURE_HOOK: &str = r#"// ivar memory capture hook for OMP
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
    const child = spawn("ivar", ["memory", "capture", "--provider", "omp"], {
      stdio: ["pipe", "ignore", "ignore"],
      detached: true,
    });
    child.on("error", () => {});
    child.stdin.on("error", () => {});
    child.stdin.end(JSON.stringify(clip(payload)));
    child.unref();
  } catch {}
}

export default function ivarMemory(pi) {
  const session = (ctx) => ctx?.sessionManager?.getSessionId?.() ?? "";
  const cwd = (ctx) => ctx?.cwd ?? process.cwd();
  pi.on("session_start", async (_event, ctx) => {
    try { send({ event: "started", session: session(ctx), cwd: cwd(ctx) }); } catch {}
  });
  pi.on("before_agent_start", async (event, ctx) => {
    try { send({ event: "prompted", session: session(ctx), cwd: cwd(ctx), text: event?.prompt ?? "" }); } catch {}
  });
  pi.on("tool_result", async (event, ctx) => {
    try {
      send({
        event: "tool",
        session: session(ctx),
        cwd: cwd(ctx),
        tool: event?.toolName,
        args: event?.input ?? {},
        failed: event?.isError === true,
        result: typeof event?.content === "string" ? event.content : JSON.stringify(clip(event?.content ?? "")),
      });
    } catch {}
  });
  pi.on("session_shutdown", async (_event, ctx) => {
    try { send({ event: "ended", session: session(ctx), cwd: cwd(ctx) }); } catch {}
  });
}
"#;

pub(crate) fn capture_artifacts() -> Vec<ManagedArtifact> {
    vec![ManagedArtifact {
        relative_path: Utf8Path::new(Provider::OMP_HOOKS_DIR).join("pre/ivar-memory.js"),
        contents: OMP_CAPTURE_HOOK,
    }]
}
