//! Hall scripts — the automations a team or one person keeps under
//! `<hall>/.ivar/scripts/` (committed) and `<hall>/.ivar/scripts-local/`
//! (personal, never committed).
//!
//! `ivar` only **reads** scripts: it lists them, validates their header and
//! announces them in the session brief. It never writes, moves, chmods or
//! deletes one.
//!
//! # Format
//!
//! - The file extension is `.sh` or `.py`, and the file stem is the script's
//!   name, in kebab-case (`deploy-staging.sh` is `deploy-staging`).
//! - A comment header sits right after the shebang (the shebang itself is
//!   optional: scripts run under an explicit interpreter, never directly):
//!
//!   ```text
//!   #!/usr/bin/env bash
//!   # description: Deploy a build to staging.
//!   # usage: deploy-staging.sh <build-id>
//!   # env: STAGING_TOKEN, STAGING_URL
//!   ```
//!
//!   `description` (one line) and `usage` are required; `env` is an optional
//!   comma-separated list of environment variable names the script reads.
//! - The header is the run of `#` comment lines at the top of the file. It ends
//!   at the first line that is not a comment.
//! - A Python script may carry a PEP 723 inline-metadata block
//!   (`# /// script` … `# ///`) in that same comment run. It is skipped while
//!   reading the header, and the script runs with `uv run --script`. Shell
//!   scripts run with `bash`.
//!
//! Everything here is pure: text in, types out. Reading the folders is
//! [`crate::store::script`].

use serde::Serialize;

/// Which folder defines a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptScope {
    /// `.ivar/scripts-local/` — personal, never committed.
    Personal,
    /// `.ivar/scripts/` — committed, shared with the team.
    Team,
}

impl ScriptScope {
    /// The word the brief and `ivar script list` print.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Personal => "personal",
            Self::Team => "team",
        }
    }
}

/// The interpreter a script's extension selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptKind {
    /// `.sh`, run with `bash <path>`.
    Shell,
    /// `.py`, run with `uv run --script <path>`.
    Python,
}

/// A script whose header satisfies the format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Script {
    /// The file stem, in kebab-case.
    pub name: String,
    /// The interpreter its extension selects.
    pub kind: ScriptKind,
    /// The one-line `# description:`.
    pub description: String,
    /// The `# usage:` line.
    pub usage: String,
    /// The names in `# env:`, in order. Empty when the header has none.
    pub env: Vec<String>,
}

/// Why a file under a scripts folder is not a valid script.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScriptProblem {
    /// The extension is neither `.sh` nor `.py`.
    #[error("unsupported extension: only `.sh` and `.py` scripts are supported")]
    UnsupportedExtension,
    /// The file stem is not kebab-case.
    #[error("the file stem `{0}` is not kebab-case (lowercase letters, digits and single dashes)")]
    BadName(String),
    /// The header has no non-empty `# description:` line.
    #[error("the header has no `# description:` line")]
    MissingDescription,
    /// The header has no non-empty `# usage:` line.
    #[error("the header has no `# usage:` line")]
    MissingUsage,
    /// The header repeats a field.
    #[error("the header repeats `# {0}:`")]
    DuplicateField(&'static str),
    /// `# env:` names something that is not an environment variable name.
    #[error("`# env:` lists `{0}`, which is not an environment variable name")]
    BadEnvName(String),
    /// The file could not be read as UTF-8 text.
    #[error("the file could not be read as text")]
    Unreadable,
}

impl ScriptProblem {
    /// A stable, machine-readable code for the problem.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedExtension => "script.unsupported_extension",
            Self::BadName(_) => "script.bad_name",
            Self::MissingDescription => "script.missing_description",
            Self::MissingUsage => "script.missing_usage",
            Self::DuplicateField(_) => "script.duplicate_field",
            Self::BadEnvName(_) => "script.bad_env_name",
            Self::Unreadable => "script.unreadable",
        }
    }
}

/// The interpreter and stem for a script file name, or why it has none.
///
/// # Errors
///
/// [`ScriptProblem::UnsupportedExtension`] for anything but `.sh` or `.py`;
/// [`ScriptProblem::BadName`] when the stem is not kebab-case.
pub fn classify(file_name: &str) -> Result<(String, ScriptKind), ScriptProblem> {
    let Some((stem, extension)) = file_name.rsplit_once('.') else {
        return Err(ScriptProblem::UnsupportedExtension);
    };
    let kind = match extension {
        "sh" => ScriptKind::Shell,
        "py" => ScriptKind::Python,
        _ => return Err(ScriptProblem::UnsupportedExtension),
    };
    if !is_kebab_case(stem) {
        return Err(ScriptProblem::BadName(stem.to_owned()));
    }
    Ok((stem.to_owned(), kind))
}

/// Parse a script from its file name and content.
///
/// # Errors
///
/// The first [`ScriptProblem`] the file name or header shows.
pub fn parse(file_name: &str, source: &str) -> Result<Script, ScriptProblem> {
    let (name, kind) = classify(file_name)?;
    let header = Header::read(source)?;
    Ok(Script {
        name,
        kind,
        description: header
            .description
            .ok_or(ScriptProblem::MissingDescription)?,
        usage: header.usage.ok_or(ScriptProblem::MissingUsage)?,
        env: header.env,
    })
}

#[derive(Debug, Default)]
struct Header {
    description: Option<String>,
    usage: Option<String>,
    env: Vec<String>,
}

impl Header {
    fn read(source: &str) -> Result<Self, ScriptProblem> {
        let mut lines = source.lines().peekable();
        if lines.peek().is_some_and(|line| line.starts_with("#!")) {
            lines.next();
        }
        let mut header = Self::default();
        let mut seen_env = false;
        let mut in_pep723 = false;
        for line in lines {
            let Some(comment) = line.strip_prefix('#') else {
                break;
            };
            let comment = comment.trim();
            if in_pep723 {
                in_pep723 = comment != "///";
                continue;
            }
            if comment == "/// script" {
                in_pep723 = true;
                continue;
            }
            if let Some(value) = field(comment, "description") {
                set_once(&mut header.description, "description", value)?;
            } else if let Some(value) = field(comment, "usage") {
                set_once(&mut header.usage, "usage", value)?;
            } else if let Some(value) = field(comment, "env") {
                if seen_env {
                    return Err(ScriptProblem::DuplicateField("env"));
                }
                seen_env = true;
                header.env = env_names(value)?;
            }
        }
        Ok(header)
    }
}

/// The value after `key:` on a comment line, trimmed.
fn field<'a>(comment: &'a str, key: &str) -> Option<&'a str> {
    comment
        .strip_prefix(key)
        .and_then(|rest| rest.strip_prefix(':'))
        .map(str::trim)
}

fn set_once(
    slot: &mut Option<String>,
    key: &'static str,
    value: &str,
) -> Result<(), ScriptProblem> {
    if slot.is_some() {
        return Err(ScriptProblem::DuplicateField(key));
    }
    if !value.is_empty() {
        *slot = Some(value.to_owned());
    }
    Ok(())
}

fn env_names(value: &str) -> Result<Vec<String>, ScriptProblem> {
    let mut names = Vec::new();
    for raw in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if !is_env_name(raw) {
            return Err(ScriptProblem::BadEnvName(raw.to_owned()));
        }
        names.push(raw.to_owned());
    }
    Ok(names)
}

fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_kebab_case(stem: &str) -> bool {
    !stem.is_empty()
        && stem.split('-').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

#[cfg(test)]
#[path = "../../tests/unit/domain/script.rs"]
mod tests;
