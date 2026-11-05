//! Values for agents' `secrets`, stored in `.ivar/secrets/agents.env`.
//!
//! Same format and keys-only `Debug` as [`McpSecrets`]; the file must be
//! owner-only before it is read.

use crate::error::Failure;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::mcp_secrets::{Change, McpSecrets, render_value};

#[derive(Clone, Default)]
pub struct AgentSecrets(McpSecrets);

impl std::fmt::Debug for AgentSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("AgentSecrets").field(&self.0).finish()
    }
}

impl AgentSecrets {
    /// Read `.ivar/secrets/agents.env`, empty when absent.
    ///
    /// # Errors
    ///
    /// `agent.secrets_mode` when the file grants any group or other bit, or a
    /// read or parse failure.
    pub fn read(layout: &Layout) -> Result<Self, Failure> {
        let path = layout.agent_secrets_env();
        #[cfg(unix)]
        if let Some(mode) = fs::unix_mode(&path)?.map(|m| m & 0o777)
            && mode & 0o077 != 0
        {
            return Err(Failure::blocked(
                "agent.secrets_mode",
                format!(
                    "{path} has mode {mode:04o}; secrets must be owner-only. Run `chmod 600 {path}`."
                ),
            ));
        }
        match fs::read_text(&path) {
            Ok(Some(text)) => McpSecrets::parse(&text, &path).map(Self),
            Ok(None) => Ok(Self::default()),
            Err(err) => Err(err.into()),
        }
    }

    /// Create an empty owner-only `agents.env` when it is absent.
    ///
    /// # Errors
    ///
    /// Returns [`Failure`] if the file cannot be inspected or written.
    pub fn ensure_file(layout: &Layout) -> Result<(), Failure> {
        let path = layout.agent_secrets_env();
        if fs::read_text(&path)?.is_none() {
            fs::write_sensitive_atomic(&path, b"")?;
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name)
    }

    /// Insert or replace `name` in `.ivar/secrets/agents.env`, owner-only,
    /// keeping every other line (comments, blank lines, key order) as it was.
    ///
    /// # Errors
    ///
    /// `agent.secrets_mode` when the file is not owner-only, a parse failure
    /// when the edited file would not read back (nothing is written), or a
    /// read or write failure.
    pub fn set_and_write(layout: &Layout, name: &str, value: &str) -> Result<Change, Failure> {
        let change = match Self::read(layout)?.get(name) {
            Some(existing) if existing == value => return Ok(Change::Unchanged),
            Some(_) => Change::Updated,
            None => Change::Created,
        };
        let path = layout.agent_secrets_env();
        let text = fs::read_text(&path)?.unwrap_or_default();
        let line = format!("{name}={}", render_value(value));
        let mut lines: Vec<&str> = text.lines().collect();
        let existing = lines
            .iter_mut()
            .find(|l| l.split_once('=').is_some_and(|(key, _)| key.trim() == name));
        match existing {
            Some(existing) => *existing = &line,
            None => lines.push(&line),
        }
        let mut updated = lines.join("\n");
        updated.push('\n');
        McpSecrets::parse(&updated, &path)?;
        fs::write_sensitive_atomic(&path, updated.as_bytes())?;
        Ok(change)
    }

    /// The named secrets, in the order asked.
    ///
    /// # Errors
    ///
    /// `agent.secret_missing` naming the first absent key.
    pub fn select(&self, names: &[String]) -> Result<Vec<(String, String)>, Failure> {
        names
            .iter()
            .map(|name| {
                self.0
                    .get(name)
                    .map(|value| (name.clone(), value.to_owned()))
                    .ok_or_else(|| {
                        Failure::blocked(
                            "agent.secret_missing",
                            format!("secret `{name}` is not set in .ivar/secrets/agents.env"),
                        )
                    })
            })
            .collect()
    }
}

/// Render `K=V` lines for `docker run --env-file`.
///
/// # Errors
///
/// `agent.secret_multiline` naming the key, because `--env-file` has no
/// quoting and a newline would split the value.
pub fn render_env_file(pairs: &[(String, String)]) -> Result<String, Failure> {
    pairs
        .iter()
        .map(|(key, value)| {
            if value.contains(['\n', '\r']) {
                return Err(Failure::blocked(
                    "agent.secret_multiline",
                    format!("secret `{key}` spans several lines, which an env file cannot carry"),
                ));
            }
            Ok(format!("{key}={value}\n"))
        })
        .collect()
}

#[cfg(test)]
#[path = "../../tests/unit/store/agent_secrets.rs"]
mod tests;
