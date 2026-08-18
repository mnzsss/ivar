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

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::feature::{ExecutionBoard, Feature, WriteContract};
use crate::domain::name::FeatureName;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::store::layout::Layout;

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

    // The hook forwards whatever the executor's tool call reported, which is
    // an absolute path (Claude and OpenCode both resolve tool file paths to
    // the canonical location). A contract is written in the shape of a session
    // view dir — `<repo>/<path>` — so an absolute path must be relativised
    // before the glob can see that shape. A worktree path carries the branch
    // segment a contract never names, so it is rewritten to `<repo>/<path>`
    // first (the same normalisation `launch`'s audit applies); anything under
    // the hall, including a view dir, is then made relative to the hall root.
    let contract = WriteContract::new(workstream.write_contract.clone());
    let resolved = ctx.resolve(&path);
    let allowed = contract.allows(&normalise_path(&layout, &feature_name, &resolved));

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

/// Put `path` in the shape a write contract is written in — `<repo>/<path>`,
/// relative to the hall root — regardless of the absolute location the hook
/// forwarded.
///
/// An executor works in a session view dir, where each repo is a symlink at
/// the top level, so a contract names a file `<repo>/<path>`. The hook,
/// however, forwards the absolute canonical path, which can be either the
/// path through the view dir or the symlink's worktree target:
///
/// - `<hall>/.ivar/features/<f>/sessions/<id>/<repo>/<path>` — relativising
///   against the hall root keeps the `<repo>/<path>` tail the contract sees.
/// - `<hall>/.ivar/repos/<repo>/<branch>/<path>` — the worktree target. The
///   branch segment is not something a contract ever names, so it is dropped
///   and the path is rewritten to `<repo>/<path>`, the same normalisation
///   `launch`'s post-run audit applies. Without this, a relative glob
///   anchored with `ends_with` matches nothing at all.
///
/// `path` is already resolved to an absolute path. The default is to leave it
/// unchanged: an unknown layout, an absent feature record, or a path outside
/// the hall all fall through to the contract's own (deny-by-default) answer.
fn normalise_path(layout: &Layout, feature: &FeatureName, path: &Utf8Path) -> Utf8PathBuf {
    let root = layout.root();
    if let Some(feature_record) = read_feature_record(layout, feature) {
        for repo in feature_record.promotions.keys() {
            let worktree = layout.repo_worktree(repo, &feature_record.branch);
            if let Ok(relative) = path.strip_prefix(&worktree)
                && !relative.as_str().is_empty()
            {
                return Utf8PathBuf::from(repo.as_str()).join(relative);
            }
        }
    }
    path.strip_prefix(root)
        .map(Utf8PathBuf::from)
        .unwrap_or_else(|_| path.to_path_buf())
}

/// Read the feature record, or `None` when it is absent or unreadable — the
/// record is only used to map worktree paths back to `<repo>/<path>`, and a
/// view-dir path still matches without it.
fn read_feature_record(layout: &Layout, feature: &FeatureName) -> Option<Feature> {
    Feature::read(layout, feature).ok().flatten()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/guard_check.rs"]
mod tests;
