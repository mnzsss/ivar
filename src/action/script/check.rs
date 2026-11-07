//! `ivar script check <path>` — is the script's current content approved?
//!
//! Read-only: it never creates or migrates `memory.db`. The agent runs it
//! before running a script and asks the human when the answer is
//! `unreviewed`.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::{Ctx, discover_hall};
use crate::domain::script::parse;
use crate::domain::script_approval::state;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::infra::{fs, hash};
use crate::store::memory::approvals;

/// What `ivar script check` found.
#[derive(Debug, Clone, Serialize)]
pub struct CheckOutcome {
    /// The path relative to the hall root.
    pub path: String,
    /// `approved` or `unreviewed`.
    pub approval: &'static str,
    /// Why it is unreviewed: `changed since approval` or `no approval`.
    /// Null when approved.
    pub reason: Option<&'static str>,
    /// The environment variables the script's header declares. Empty when the
    /// header names none or is not valid.
    pub env: Vec<String>,
    /// The SHA-256 of the current content.
    pub sha256: String,
}

impl CheckOutcome {
    /// Whether the current content is approved.
    #[must_use]
    pub fn is_approved(&self) -> bool {
        self.reason.is_none()
    }
}

impl WriteHuman for CheckOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        match self.reason {
            None => writeln!(w, "approved: {}", self.path)?,
            Some(reason) => writeln!(w, "unreviewed: {} ({reason})", self.path)?,
        }
        if self.env.is_empty() {
            writeln!(w, "env: none")
        } else {
            writeln!(w, "env: {}", self.env.join(", "))
        }
    }
}

/// Report whether the script at `path` is approved.
///
/// # Errors
///
/// `script.not_found` or `script.outside_hall` for a path that is not a file
/// in the hall; `script.approvals_unavailable` when `memory.db` cannot be read.
pub fn check(ctx: &Ctx, path: &Utf8PathBuf) -> Outcome<CheckOutcome> {
    let layout = discover_hall(ctx)?;
    let target = super::resolve(ctx, &layout, path)?;
    let bytes = fs::read_bytes(&target.abs)?
        .ok_or_else(|| Failure::blocked("script.not_found", format!("`{path}` is not a file")))?;
    let sha256 = hash::bytes(&bytes);
    let recorded =
        approvals::read_all(layout.memory_db().as_std_path()).map_err(super::store_failure)?;
    let found = state(recorded.get(&target.rel).map(String::as_str), &sha256);
    let env = std::str::from_utf8(&bytes)
        .ok()
        .zip(target.abs.file_name())
        .and_then(|(source, name)| parse(name, source).ok())
        .map(|script| script.env)
        .unwrap_or_default();
    Ok(Report::new(CheckOutcome {
        path: target.rel,
        approval: found.as_str(),
        reason: found
            .reason()
            .map(crate::domain::script_approval::UnreviewedReason::as_str),
        env,
        sha256,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/script/check.rs"]
mod tests;
