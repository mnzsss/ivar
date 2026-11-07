//! Execution approval of a script: a human vouched for its exact content.
//!
//! An approval is the pair (hall-relative path, sha256 of the file's bytes).
//! A script is **approved** while its current hash equals the recorded one and
//! **unreviewed** otherwise — never approved, or edited since. Nothing here
//! stores or reads: [`crate::store::memory::approvals`] keeps the pairs, and
//! the actions compare.
//!
//! The module also holds the lexical check `ivar doctor` runs for scripts that
//! reference a secret file. It is deliberately simple: it reads tokens, not
//! shell or Python, so a read built at run time slips through (see
//! `docs/reference/limitations-memory.md`).

use serde::Serialize;

/// Why a script is unreviewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnreviewedReason {
    /// An approval exists for the path, for other content.
    ChangedSinceApproval,
    /// No approval was ever recorded for the path.
    NoApproval,
}

impl UnreviewedReason {
    /// The sentence `ivar script check` prints and reports.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChangedSinceApproval => "changed since approval",
            Self::NoApproval => "no approval",
        }
    }
}

/// Whether a script's current content is approved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalState {
    /// The recorded hash equals the current one.
    Approved,
    /// The content is not the approved one, or nothing was approved.
    Unreviewed(UnreviewedReason),
}

impl ApprovalState {
    /// `approved` or `unreviewed`: the word `ivar script list` and `check` use.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Unreviewed(_) => "unreviewed",
        }
    }

    /// Whether the state is [`ApprovalState::Approved`].
    #[must_use]
    pub fn is_approved(self) -> bool {
        matches!(self, Self::Approved)
    }

    /// The reason, when unreviewed.
    #[must_use]
    pub fn reason(self) -> Option<UnreviewedReason> {
        match self {
            Self::Approved => None,
            Self::Unreviewed(reason) => Some(reason),
        }
    }
}

/// The state of content hashing to `current`, given the hash recorded for its
/// path, if any.
#[must_use]
pub fn state(recorded: Option<&str>, current: &str) -> ApprovalState {
    match recorded {
        Some(recorded) if recorded == current => ApprovalState::Approved,
        Some(_) => ApprovalState::Unreviewed(UnreviewedReason::ChangedSinceApproval),
        None => ApprovalState::Unreviewed(UnreviewedReason::NoApproval),
    }
}

/// A line of a script that names a secret file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretFileReference {
    /// The 1-based line.
    pub line: usize,
    /// The file name as written (its last path component).
    pub file: String,
}

/// Every line that names a secret file: `.env`, `.env.<anything>`, `.envrc`,
/// or any `<name>.env` such as `agents.env`.
///
/// Lexical: each line is cut into path-like words and the last path component
/// of each is compared. `.env.example`, `.env.sample` and `.env.template` are
/// not secrets and are skipped. A path assembled at run time is not seen.
#[must_use]
pub fn secret_file_references(source: &str) -> Vec<SecretFileReference> {
    let mut found = Vec::new();
    for (index, line) in source.lines().enumerate() {
        for word in line.split(|c: char| !is_path_char(c)) {
            let file = word.rsplit('/').next().unwrap_or(word);
            if is_secret_file(file) {
                found.push(SecretFileReference {
                    line: index + 1,
                    file: file.to_owned(),
                });
            }
        }
    }
    found
}

fn is_path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '~' | '$' | '{' | '}')
}

fn is_secret_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if [".example", ".sample", ".template"]
        .iter()
        .any(|safe| lower.ends_with(safe))
    {
        return false;
    }
    lower == ".env" || lower.starts_with(".env.") || lower == ".envrc" || lower.ends_with(".env")
}

#[cfg(test)]
#[path = "../../tests/unit/domain/script_approval.rs"]
mod tests;
