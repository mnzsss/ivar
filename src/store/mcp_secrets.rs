//! Durable local storage for MCP OAuth client secrets.
//!
//! Stored in `.ivar/secrets/mcp.env` with owner-only (`0600`) permissions on Unix.
//! Values are private, never derived with `Serialize`, and never exposed in `Debug`.

use std::collections::BTreeMap;

use camino::Utf8Path;

use crate::error::Failure;
use crate::infra::fs;
use crate::store::layout::Layout;

/// What [`McpSecrets::set_and_write`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The key was not present and was created.
    Created,
    /// The key was present with a different value and was updated.
    Updated,
    /// The key was already present with the exact same value.
    Unchanged,
}

/// Durable, private MCP credential store.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct McpSecrets {
    entries: BTreeMap<String, String>,
}

impl std::fmt::Debug for McpSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpSecrets")
            .field("keys", &self.entries.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl McpSecrets {
    /// Read and parse `.ivar/secrets/mcp.env`. Returns an empty store if the file is absent.
    pub fn read(layout: &Layout) -> Result<Self, Failure> {
        let path = layout.mcp_secrets_env();
        match fs::read_text(&path) {
            Ok(Some(text)) => Self::parse(&text, &path),
            Ok(None) => Ok(Self::default()),
            Err(err) => Err(err.into()),
        }
    }

    /// Retrieve a secret by environment variable name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries.get(name).map(|s| s.as_str())
    }

    /// Whether the store contains a value for `name`.
    #[must_use]
    pub fn contains_key(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Update or insert a secret and atomically write the updated file to `.ivar/secrets/mcp.env`.
    pub fn set_and_write(layout: &Layout, name: &str, value: &str) -> Result<Change, Failure> {
        if !is_valid_key(name) {
            return Err(Failure::failed(
                "store.mcp_secrets_invalid_key",
                format!("invalid environment variable name `{name}`"),
            )
            .expected("ASCII identifier (letters, digits, underscores, not starting with digit)")
            .actual(format!("invalid key `{name}`")));
        }

        let mut secrets = Self::read(layout)?;
        let change = match secrets.entries.get(name) {
            Some(existing) if existing == value => Change::Unchanged,
            Some(_) => Change::Updated,
            None => Change::Created,
        };

        if change != Change::Unchanged {
            secrets.entries.insert(name.to_owned(), value.to_owned());
            let rendered = secrets.render();
            let path = layout.mcp_secrets_env();
            fs::write_sensitive_atomic(&path, rendered.as_bytes())?;
        }

        Ok(change)
    }

    /// Parse the `.env` text format safely without shell evaluation.
    ///
    /// Malformed lines, duplicate keys, or invalid syntax fail closed naming the path
    /// and line number, without including any raw secret material.
    pub fn parse(text: &str, path: &Utf8Path) -> Result<Self, Failure> {
        let mut entries = BTreeMap::new();

        for (idx, line) in text.lines().enumerate() {
            let line_no = idx + 1;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let Some((raw_key, raw_val)) = trimmed.split_once('=') else {
                return Err(Failure::failed(
                    "store.mcp_secrets_malformed",
                    format!("{path}:{line_no}: malformed line (missing `=`)"),
                )
                .expected("KEY=VALUE")
                .actual("line without `=`"));
            };

            let key = raw_key.trim();
            if !is_valid_key(key) {
                return Err(Failure::failed(
                    "store.mcp_secrets_invalid_key",
                    format!("{path}:{line_no}: invalid environment variable name"),
                )
                .expected(
                    "ASCII identifier (letters, digits, underscores, not starting with digit)",
                )
                .actual("invalid key name"));
            }

            if entries.contains_key(key) {
                return Err(Failure::failed(
                    "store.mcp_secrets_duplicate_key",
                    format!("{path}:{line_no}: duplicate key `{key}`"),
                )
                .expected("unique key")
                .actual(format!("duplicate key `{key}`")));
            }

            let val_str = raw_val.trim();
            let val = if val_str.starts_with('"') {
                if !val_str.ends_with('"') || val_str.len() < 2 {
                    return Err(Failure::failed(
                        "store.mcp_secrets_invalid_value",
                        format!("{path}:{line_no}: unterminated quoted string"),
                    )
                    .expected("terminated double-quoted string")
                    .actual("unterminated quote"));
                }
                serde_json::from_str::<String>(val_str).map_err(|_| {
                    Failure::failed(
                        "store.mcp_secrets_invalid_value",
                        format!("{path}:{line_no}: invalid escaped string value"),
                    )
                    .expected("valid JSON-escaped string")
                    .actual("invalid escape sequence")
                })?
            } else {
                val_str.to_owned()
            };

            entries.insert(key.to_owned(), val);
        }

        Ok(Self { entries })
    }

    /// Deterministically render entries into `.env` format, sorted by key, ending with one newline.
    #[must_use]
    pub fn render(&self) -> String {
        let mut rendered = String::new();
        for (key, val) in &self.entries {
            let escaped = serde_json::to_string(val).unwrap_or_else(|_| format!("\"{val}\""));
            rendered.push_str(key);
            rendered.push('=');
            rendered.push_str(&escaped);
            rendered.push('\n');
        }
        rendered
    }
}

fn is_valid_key(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap_or('\0');
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
#[path = "../../tests/unit/store/mcp_secrets.rs"]
mod tests;
