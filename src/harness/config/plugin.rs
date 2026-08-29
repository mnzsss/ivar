//! The OpenCode plugin: embedded JavaScript source materialised by `sync`
//! into `<hall>/.opencode/plugins/ivar.js`.
//!
//! The plugin hooks two events:
//! - `shell.env` — injects session environment variables (`IVAR_HALL`,
//!   `IVAR_SESSION_ID`, `IVAR_SESSION_PATH`, `IVAR_PROVIDER`) into the
//!   shell that OpenCode launches.
//! - `tool.execute.before` — runs `ivar guard` to enforce the write-guard
//!   contract before any tool call.

use camino::Utf8Path;

use crate::infra::{fs, json};

use super::{Change, Error};

/// The embedded plugin source: a single file OpenCode loads from
/// `.opencode/plugins/`. Idempotent — `materialise_plugin` writes it only
/// when the bytes on disk differ.
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

/// Materialise the ivar plugin at `path`.
///
/// Created when absent, [`Change::Unchanged`] when the bytes on disk already
/// match. The parent directory is created if needed.
pub fn materialise_plugin(path: &Utf8Path) -> Result<Change, Error> {
    let existing = fs::read_text(path).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source: json::Error::Fs(source),
    })?;

    if existing.as_deref() == Some(OPENCODE_PLUGIN) {
        return Ok(Change::Unchanged);
    }

    if let Some(parent) = path.parent() {
        fs::ensure_dir(parent).map_err(|source| Error::Mcp {
            path: path.to_path_buf(),
            source: json::Error::Fs(source),
        })?;
    }
    fs::write_text(path, OPENCODE_PLUGIN).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source: json::Error::Fs(source),
    })?;
    Ok(Change::Created)
}

/// Remove the plugin file at `path`. Absent file is [`Change::Unchanged`].
pub fn remove_plugin(path: &Utf8Path) -> Result<Change, Error> {
    if !fs::exists(path).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source: json::Error::Fs(source),
    })? {
        return Ok(Change::Unchanged);
    }
    fs::remove_file(path).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source: json::Error::Fs(source),
    })?;
    Ok(Change::Removed)
}

#[cfg(test)]
#[path = "../../../tests/unit/harness/config/plugin.rs"]
mod tests;
