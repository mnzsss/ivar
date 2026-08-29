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
pub const OPENCODE_PLUGIN: &str = r#"// ivar session plugin for OpenCode
// Materialised by `ivar sync`. Do not edit.

export default {
  "shell.env": async (ctx) => {
    const { execSync } = await import("child_process");
    const env = JSON.parse(execSync("ivar session env", { encoding: "utf-8" }));
    return { ...ctx.env, ...env };
  },

  "tool.execute.before": async (ctx) => {
    const { execSync } = await import("child_process");
    try {
      execSync("ivar guard --provider opencode", {
        encoding: "utf-8",
        stdio: "pipe",
      });
    } catch (e) {
      return { ...ctx, abort: true, error: e.message };
    }
    return ctx;
  },
};
"#;

/// Materialise the ivar plugin at `path`.
///
/// Created when absent, [`Change::Unchanged`] when the bytes on disk already
/// match. The parent directory is created if needed.
pub fn materialise_plugin(path: &Utf8Path) -> Result<Change, Error> {
    if let Some(existing) = fs::read_text(path).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source: json::Error::Fs(source),
    })? {
        if existing == OPENCODE_PLUGIN {
            return Ok(Change::Unchanged);
        }
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
