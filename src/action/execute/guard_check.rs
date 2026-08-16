//! `ivar feature execute guard-check --session <id> --path <path>` — check
//! whether a path is allowed by the write contract of the workstream that
//! owns the given provider session.
//!
//! # What it does
//!
//! Reads the execution board for `input.feature`, looks up `session` in the
//! `sessions` map to find the owning workstream, and checks whether
//! `path` is covered by that workstream's [`WriteContract`].
//!
//! The default is DENY: unknown session, missing board, unreadable board — all
//! refuse. A path inside the contract passes; outside is refused naming the
//! workstream.

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::feature::{ExecutionBoard, WriteContract};
use crate::domain::name::FeatureName;
use crate::error::{Failure, Outcome, Report, WriteHuman};

use super::super::discover_hall;
use super::find_workstream;

/// What `ivar feature execute guard-check` needs.
#[derive(Debug, Clone)]
pub struct GuardCheckInput {
    /// The feature whose board holds the session→workstream link.
    pub feature: Option<String>,
    /// Provider session id to look up in the board's `sessions` map.
    pub session: Option<String>,
    /// Path to check against the workstream's write contract.
    pub path: Option<String>,
}

/// What `ivar feature execute guard-check` did.
#[derive(Debug, Clone, Serialize)]
pub struct GuardCheckOutcome {
    /// Whether the path is allowed.
    pub allowed: bool,
    /// The workstream that owns the session, when known.
    pub workstream: Option<String>,
    /// Where the board was read from.
    pub board_path: Utf8PathBuf,
}

impl WriteHuman for GuardCheckOutcome {
    fn write_human(&self, w: &mut impl std::io::Write) -> std::io::Result<()> {
        let status = if self.allowed { "allowed" } else { "denied" };
        writeln!(
            w,
            "guard-check: {} ({})",
            status,
            self.workstream.as_deref().unwrap_or("unknown session")
        )
    }
}

/// Check whether `input.path` is allowed by the write contract of the
/// workstream that owns `input.session`.
///
/// Blocked when the board is missing, any required argument is absent, or the
/// path falls outside the workstream's write contract. The default is DENY.
pub fn guard_check(ctx: &Ctx, input: GuardCheckInput) -> Outcome<GuardCheckOutcome> {
    // Validate arguments before touching the hall — a missing argument is a
    // caller error, not a hall problem.
    let feature_name = require_feature(&input)?;
    let path = require_path(&input)?;
    let session = require_session(&input)?;

    let layout = discover_hall(ctx)?;

    let board = match ExecutionBoard::read(&layout, &feature_name) {
        Ok(Some(b)) => b,
        Ok(None) => {
            return Ok(Report::new(GuardCheckOutcome {
                allowed: false,
                workstream: None,
                board_path: crate::store::feature::board_path(&layout, &feature_name),
            }));
        }
        Err(e) => {
            return Err(e);
        }
    };

    // Look up the session → workstream link.
    let workstream_id = match board.sessions.get(session) {
        Some(id) => id.clone(),
        None => {
            // Unknown session — never allowed by omission.
            return Ok(Report::new(GuardCheckOutcome {
                allowed: false,
                workstream: None,
                board_path: crate::store::feature::board_path(&layout, &feature_name),
            }));
        }
    };

    // Find the workstream definition to get its write contract.
    let workstream = match find_workstream(&board, &feature_name, &workstream_id) {
        Ok(ws) => ws,
        Err(_) => {
            // Session references a workstream not found on the board — deny.
            return Ok(Report::new(GuardCheckOutcome {
                allowed: false,
                workstream: Some(workstream_id),
                board_path: crate::store::feature::board_path(&layout, &feature_name),
            }));
        }
    };

    let contract = WriteContract::new(workstream.write_contract.clone());
    let resolved = ctx.resolve(&path);

    let allowed = contract.allows(&resolved);

    Ok(Report::new(GuardCheckOutcome {
        allowed,
        workstream: Some(workstream_id),
        board_path: crate::store::feature::board_path(&layout, &feature_name),
    }))
}

fn require_feature(input: &GuardCheckInput) -> Result<FeatureName, Failure> {
    let feature = input.feature.as_deref().ok_or_else(|| {
        Failure::blocked(
            "execute.guard_check.missing_feature",
            "--feature is required".to_owned(),
        )
    })?;
    Ok(FeatureName::new(feature)?)
}

fn require_session(input: &GuardCheckInput) -> Result<&str, Failure> {
    input.session.as_deref().ok_or_else(|| {
        Failure::blocked(
            "execute.guard_check.missing_session",
            "--session is required".to_owned(),
        )
    })
}

fn require_path(input: &GuardCheckInput) -> Result<Utf8PathBuf, Failure> {
    input.path.as_deref().map(Utf8PathBuf::from).ok_or_else(|| {
        Failure::blocked(
            "execute.guard_check.missing_path",
            "--path is required".to_owned(),
        )
    })
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/guard_check.rs"]
mod tests;
