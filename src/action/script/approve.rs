//! `ivar script approve <path>` — a human vouches for a script's exact content.
//!
//! The content is searched with the capture sanitizer's secret patterns first.
//! A match refuses the approval and records nothing, so an approved script
//! never carries a secret into the repository or the agent's context.
//! Otherwise the path and the content's SHA-256 are recorded in `memory.db`,
//! replacing any earlier approval of that path. The content is never stored.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::{Ctx, discover_hall};
use crate::domain::sanitize::find_secrets;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::infra::{fs, hash};
use crate::store::memory::approvals;
use crate::store::memory::db::MemoryDb;

/// Most findings the refusal sentence lists; `details` carries them all.
const LISTED_FINDINGS: usize = 10;

/// What `ivar script approve` recorded.
#[derive(Debug, Clone, Serialize)]
pub struct ApproveOutcome {
    /// The path relative to the hall root.
    pub path: String,
    /// The SHA-256 of the approved content.
    pub sha256: String,
    /// When it was approved, RFC 3339.
    pub approved_at: String,
}

impl WriteHuman for ApproveOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Approved {} (sha256 {}).", self.path, self.sha256)
    }
}

/// Approve the current content of the script at `path`.
///
/// # Errors
///
/// `script.not_found` or `script.outside_hall` for a path that is not a file
/// in the hall; `script.unreadable` when the file is not UTF-8 text;
/// `script.secret_found` when a secret pattern matches (nothing is recorded);
/// `script.approvals_unavailable` when `memory.db` cannot be written.
pub fn approve(ctx: &Ctx, path: &Utf8PathBuf) -> Outcome<ApproveOutcome> {
    let layout = discover_hall(ctx)?;
    let target = super::resolve(ctx, &layout, path)?;
    let bytes = fs::read_bytes(&target.abs)?
        .ok_or_else(|| Failure::blocked("script.not_found", format!("`{path}` is not a file")))?;
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Err(Failure::blocked(
            "script.unreadable",
            format!("`{path}` is not UTF-8 text, so it cannot be reviewed"),
        ));
    };
    let findings = find_secrets(text);
    if !findings.is_empty() {
        return Err(refusal(&target.rel, &findings));
    }
    let sha256 = hash::bytes(&bytes);
    let approved_at = rfc3339_now();

    crate::action::graph::graph_db_path(&layout)?;
    let _lock = fs::lock_exclusive(&layout.memory_lock())?;
    let db = MemoryDb::open(layout.memory_db().as_std_path()).map_err(super::store_failure)?;
    approvals::record(db.conn(), &target.rel, &sha256, &approved_at)
        .map_err(super::store_failure)?;
    Ok(Report::new(ApproveOutcome {
        path: target.rel,
        sha256,
        approved_at,
    }))
}

fn refusal(rel: &str, findings: &[crate::domain::sanitize::SecretFinding]) -> Failure {
    let mut listed: Vec<String> = findings
        .iter()
        .take(LISTED_FINDINGS)
        .map(|f| format!("line {}: {}", f.line, f.pattern))
        .collect();
    if findings.len() > LISTED_FINDINGS {
        listed.push(format!("and {} more", findings.len() - LISTED_FINDINGS));
    }
    let details: Vec<_> = findings
        .iter()
        .map(|f| serde_json::json!({ "line": f.line, "pattern": f.pattern }))
        .collect();
    Failure::blocked(
        "script.secret_found",
        format!(
            "refusing to approve `{rel}`: it matches a secret pattern ({}). Nothing was recorded.",
            listed.join("; ")
        ),
    )
    .details(serde_json::json!({ "path": rel, "findings": details }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/script/approve.rs"]
mod tests;
