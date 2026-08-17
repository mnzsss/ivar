//! `ivar feature execute replan <feature> --plan <path> --graph-json <path>`
//! — adopt a complete revised plan and graph into an existing execution
//! board.
//!
//! # What it does
//!
//! Reads the feature's [`ExecutionBoard`], resolves the revised plan and
//! graph the caller points at through the shared resolver
//! ([`super::graph`]) — the same resolution `prepare` applies, so the two
//! verbs cannot disagree about what a valid graph is — and replaces the
//! board's graph with the resolved candidate, merging execution state by
//! stable workstream id:
//!
//! - an **identical** definition (every authored field) retains its old
//!   status;
//! - a **changed** definition becomes `Paused`;
//! - a **new** definition becomes `Paused`;
//! - an **omitted** non-`Done` definition is removed;
//! - an omitted `Done` definition blocks unless `--allow-remove-completed`
//!   is supplied ([`ReplanInput::allow_remove_completed`]).
//!
//! The board's `plan_fingerprint` and graph become the resolved candidate,
//! the resolved plan is written to the feature's canonical `plan.md`, and a
//! `replan` journal entry names the changed, added, removed, and protected
//! workstreams. Removed workstreams remain visible through the journal
//! entries that mention them — the journal is never rewritten.
//!
//! # What it is not
//!
//! Replanning never rewrites or destroys journal history or sessions. It
//! adopts a complete graph: a workstream omitted from the revised graph is
//! gone from the board, but its history stays in the journal. A replanned
//! board is a fresh complete representation of the current resolved graph,
//! and every subsequent replan diffs against the immediately previous graph
//! — not the one the board was originally prepared with.
//!
//! # v1 scope
//!
//! Whole-workstream merge: any difference in the authored fields pauses the
//! whole workstream. No per-operation inbox granularity yet.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::feature::{ExecutionBoard, JournalEntry, WorkstreamDef, WorkstreamStatus};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::{fs, hash};

use super::super::discover_hall;
use super::graph;
use super::require_board;
use crate::action::Ctx;
use crate::store::feature;

/// What `ivar feature execute replan` needs.
#[derive(Debug, Clone)]
pub struct ReplanInput {
    /// The feature whose board is replanned.
    pub feature: String,
    /// Path to the revised `plan.md`. Resolved against the current directory.
    pub plan: String,
    /// Path to the revised execution graph JSON — the complete replacement
    /// graph the board adopts. Resolved against the current directory.
    pub graph_json: String,
    /// Whether omitted `Done` workstreams may be removed. Defaults to
    /// `false`: a completed workstream never disappears from the board
    /// without this explicit authorization.
    pub allow_remove_completed: bool,
}

/// What `ivar feature execute replan` did.
#[derive(Debug, Clone, Serialize)]
pub struct ReplanOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the board belongs to.
    pub feature: FeatureName,
    /// SHA-256 of the resolved plan — the board's new `plan_fingerprint`.
    pub fingerprint: String,
    /// The workstreams this replan changed, in board order.
    pub changed: Vec<String>,
    /// The workstreams this replan added.
    pub added: Vec<String>,
    /// The workstreams this replan removed (that were not protected).
    pub removed: Vec<String>,
    /// The workstreams this replan kept unchanged.
    pub retained: Vec<String>,
    /// The workstreams that were protected from removal — omitted `Done`
    /// workstreams when `allow_remove_completed` was not supplied. Empty
    /// once the replan succeeds.
    pub protected: Vec<String>,
    /// The board after the replan.
    pub board: ExecutionBoard,
    /// Where the board was written.
    pub board_path: Utf8PathBuf,
}

impl WriteHuman for ReplanOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Replanned `{}` to {} at {}",
            self.feature, self.fingerprint, self.board_path
        )?;
        fn list(w: &mut impl io::Write, name: &str, items: &[String]) -> io::Result<()> {
            if items.is_empty() {
                return Ok(());
            }
            writeln!(w, "  {name}: {}", items.join(", "))
        }
        list(w, "changed", &self.changed)?;
        list(w, "added", &self.added)?;
        list(w, "removed", &self.removed)?;
        list(w, "retained", &self.retained)?;
        if !self.protected.is_empty() {
            writeln!(
                w,
                "  protected from removal (completed): {}",
                self.protected.join(", ")
            )?;
        }
        Ok(())
    }
}

/// Fold the revised plan and graph at `input` into `input.feature`'s board.
///
/// Blocked when the feature has no board yet — replanning advances an
/// existing board; it does not create one. Every validation runs through the
/// shared resolver **before** anything is written: graph parsing, locked
/// contracts, dependencies, targeting, and plan-backed operations all refuse
/// a candidate that must not be adopted, leaving the persisted plan, board,
/// and journal byte-identical. An omitted `Done` workstream is refused with
/// `execute.completed_workstream_requires_authorization` unless
/// `allow_remove_completed` is supplied.
///
/// After validation and authorization succeed, the resolved canonical plan is
/// written to `plan.md`, the board's graph and fingerprint become the
/// resolved candidate, and the replan is journaled before the board is
/// persisted.
pub fn replan(ctx: &Ctx, input: ReplanInput) -> Outcome<ReplanOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;
    let plan_path = ctx.resolve(Utf8Path::new(&input.plan));
    let graph_path = ctx.resolve(Utf8Path::new(&input.graph_json));

    let board = require_board(&layout, &feature)?;
    let board_path = feature::board_path(&layout, &feature);

    // Replan persists a board: blocked once the whole child closes as
    // `integrated`.
    let feature_record =
        crate::domain::feature::Feature::read(&layout, &feature)?.ok_or_else(|| {
            Failure::blocked(
                "execute.feature_vanished",
                format!("feature `{feature}` has a board but no feature.json"),
            )
        })?;
    crate::action::feature::ensure_not_fully_integrated(&layout, &feature_record)?;

    // Resolve and validate the candidate before writing anything.
    let canonical_plan_path = layout.plan_dir(&feature).join("plan.md");
    let resolved = graph::resolve(
        &layout,
        &feature,
        &feature_record,
        None,
        &graph_path,
        &plan_path,
    )?;

    // Merge by stable workstream id. The decision about an omitted `Done`
    // workstream happens here, before any write.
    let (merged, merge) = merge(board, resolved.workstreams, input.allow_remove_completed)?;

    let mut board = merged;
    let fingerprint = hash::text(&resolved.plan_text);

    // Persist the resolved canonical plan, then the board: the fingerprint
    // must cover exactly what the board was derived from.
    fs::write_text(&canonical_plan_path, &resolved.plan_text)?;
    board.graph.plan_fingerprint = fingerprint.clone();
    board.push_journal(JournalEntry::new(
        "board",
        "replan",
        replan_message(&merge, &fingerprint),
    ));
    board.write(&layout, &feature)?;

    Ok(Report::new(ReplanOutcome {
        root: layout.root().to_path_buf(),
        feature,
        fingerprint,
        changed: merge.changed,
        added: merge.added,
        removed: merge.removed,
        retained: merge.retained,
        protected: merge.protected,
        board,
        board_path,
    }))
}

/// The merge classes for a replan.
struct Merge {
    changed: Vec<String>,
    added: Vec<String>,
    removed: Vec<String>,
    retained: Vec<String>,
    protected: Vec<String>,
}

/// Merge `candidate` into `board` by workstream id, preserving the board's
/// journal and sessions.
///
/// An omitted `Done` workstream blocks unless `allow_remove_completed` —
/// the completed-work removal guard — is supplied; the refusal names the
/// protected workstreams so the human sees the full scope. This runs before
/// any write, so a blocked merge leaves plan/board/journal byte-identical.
fn merge(
    mut board: ExecutionBoard,
    candidate: Vec<WorkstreamDef>,
    allow_remove_completed: bool,
) -> Result<(ExecutionBoard, Merge), Failure> {
    // Classify each candidate workstream by whether a definition with the
    // same id already exists and whether it is identical.
    let mut changed = Vec::new();
    let mut added = Vec::new();
    let mut retained = Vec::new();
    for candidate_workstream in &candidate {
        match board
            .graph
            .workstreams
            .iter()
            .find(|existing| existing.id == candidate_workstream.id)
        {
            Some(existing) if identical(existing, candidate_workstream) => {
                retained.push(candidate_workstream.id.clone());
            }
            Some(_) => {
                changed.push(candidate_workstream.id.clone());
            }
            None => {
                added.push(candidate_workstream.id.clone());
            }
        }
    }

    // Workstreams on the board with no counterpart in the candidate are
    // omitted. A `Done` omission is protected unless explicitly authorized.
    let mut omitted_done = Vec::new();
    let mut removed = Vec::new();
    for workstream in &board.graph.workstreams {
        if candidate
            .iter()
            .any(|candidate| candidate.id == workstream.id)
        {
            continue;
        }
        if workstream.status == WorkstreamStatus::Done && !allow_remove_completed {
            omitted_done.push(workstream.id.clone());
        } else {
            removed.push(workstream.id.clone());
        }
    }

    // An omitted `Done` workstream without authorization is a hard block:
    // a completed workstream must never disappear accidentally. Report every
    // merge class so the human can decide, but write nothing.
    if !omitted_done.is_empty() {
        return Err(completed_removal_blocked(&omitted_done, &removed));
    }

    // Rebuild the graph from the candidate, carrying each workstream's old
    // status when it is retained unchanged, and pausing changed and added
    // workstreams until their current revision is acknowledged.
    let mut new_workstreams = Vec::with_capacity(candidate.len());
    for mut candidate_workstream in candidate {
        let status = if retained.contains(&candidate_workstream.id) {
            board
                .graph
                .workstreams
                .iter()
                .find(|existing| existing.id == candidate_workstream.id)
                .map(|existing| existing.status)
                .unwrap_or(WorkstreamStatus::Paused)
        } else {
            WorkstreamStatus::Paused
        };
        candidate_workstream.status = status;
        new_workstreams.push(candidate_workstream);
    }
    board.graph.workstreams = new_workstreams;

    Ok((
        board,
        Merge {
            changed,
            added,
            removed,
            retained,
            protected: Vec::new(),
        },
    ))
}

/// Whether `existing` and `candidate` are the same authored definition —
/// identical on every field a replan can update. Execution status is not part
/// of the authored definition and is deliberately ignored here.
fn identical(existing: &WorkstreamDef, candidate: &WorkstreamDef) -> bool {
    existing.title == candidate.title
        && existing.operations == candidate.operations
        && existing.depends_on == candidate.depends_on
        && existing.write_contract == candidate.write_contract
        && existing.provider == candidate.provider
        && existing.model == candidate.model
        && existing.agent == candidate.agent
}

/// The journal message for a successful replan, naming every merge class.
fn replan_message(merge: &Merge, fingerprint: &str) -> String {
    let mut parts = vec![format!("Plan revised to fingerprint {fingerprint}")];
    let mut list = |name: &str, items: &[String]| {
        if items.is_empty() {
            return;
        }
        parts.push(format!("{name}: {}", items.join(", ")));
    };
    list("changed", &merge.changed);
    list("added", &merge.added);
    list("removed", &merge.removed);
    list("retained", &merge.retained);
    parts.join("; ")
}

/// The refusal for an omitted `Done` workstream when removal was not
/// authorized. Names every merge class so the human sees the full scope of
/// the replan before deciding.
fn completed_removal_blocked(protected: &[String], removed: &[String]) -> Failure {
    let noun = if protected.len() == 1 {
        "workstream"
    } else {
        "workstreams"
    };
    Failure::blocked(
        "execute.completed_workstream_requires_authorization",
        format!(
            "the revised graph omits completed {noun} {}; removing a completed workstream requires explicit authorization",
            protected.join(", ")
        ),
    )
    .expected("every `Done` workstream to stay on the board, or `--allow-remove-completed` to be supplied")
    .actual(format!(
        "omitted `Done` {noun}: {}; also removing: {}",
        protected.join(", "),
        if removed.is_empty() {
            "none".to_owned()
        } else {
            removed.join(", ")
        }
    ))
    .fix(FixAction::safe(
        "execute.allow_completed_removal",
        "Re-run replan with `--allow-remove-completed` to confirm removing the completed workstreams, or restore them to the revised graph.",
    ))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/replan.rs"]
mod tests;
