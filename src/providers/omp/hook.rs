use camino::Utf8PathBuf;

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
/// non-zero (`src/bin/ivar.rs`), which is what the `catch` reads.
pub const OMP_HOOK: &str = r#"// ivar pre-tool guard hook for OMP
// Materialised by `ivar sync`. Do not edit.

import { execFileSync } from "node:child_process";

export default function ivarGuard(pi) {
  pi.on("tool_call", async (event, ctx) => {
    const payload = JSON.stringify({
      tool: event.toolName,
      args: event.input ?? {},
      cwd: ctx?.cwd ?? process.cwd(),
    });

    try {
      execFileSync("ivar", ["guard", "--provider", "omp"], {
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
  });
}
"#;

pub(crate) fn managed_artifacts() -> Vec<ManagedArtifact> {
    vec![ManagedArtifact {
        relative_path: Utf8PathBuf::from(".omp/hooks/pre/ivar.js"),
        contents: OMP_HOOK,
    }]
}
