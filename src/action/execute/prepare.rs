//! `ivar feature execute <feature> --graph-json <path>` — prepare a
//! feature's execution board.
//!
//! # What it does
//!
//! Reads the feature's plan (`plans/<feature>/plan.md`) and the execution
//! graph the plan derives — a plain JSON file of workstreams, each with
//! `id`, `title`, `operations`, `depends_on` and `write_contract` — and
//! writes an [`ExecutionBoard`] at
//! `features/<feature>/execution/board.json` (schema v1, `Policy::Local`).
//!
//! The graph file carries no execution state: `prepare` stamps the board's
//! status `Pending`, every workstream's status `Waiting`, and fingerprints
//! `plan.md` into the graph so a plan change voids the board (the same
//! content the Execution Graph approval gate fingerprints). The board's
//! journal opens with a `prepared` entry.
//!
//! Graph parsing, targeting resolution, and every validation — locked
//! contracts, dependencies, and plan-backed operations — live in the shared
//! resolver ([`super::graph`]); `prepare` owns only the one-shot board
//! creation this verb is responsible for, and delegates the rest.
//!
//! Preparing is a one-shot: a feature that already has a board is refused,
//! because re-writing it would destroy the journal. Delete `board.json`
//! deliberately to re-prepare from a fresh graph.
//!
//! # v1 scope
//!
//! Graph + status + journal only. No inboxes, no blockers, no handoffs, no
//! tick/reply — nothing here advances the board once it exists.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::feature::{ExecutionBoard, ExecutionGraph, ExecutionStatus, JournalEntry};
use crate::domain::name::FeatureName;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::{fs, hash};
use crate::store::feature;
use crate::store::layout::Layout;

use super::super::discover_hall;
use super::graph;
use crate::action::Ctx;

/// Parse the graph JSON at `path` — re-exported for the prompt test module,
/// which reaches the parser through `prepare`; the implementation lives in
/// the shared resolver ([`super::graph`]). Unused by `prepare`'s own code.
#[allow(unused_imports)]
pub(crate) use super::graph::read_workstreams;

/// What `ivar feature execute` needs.
#[derive(Debug, Clone)]
pub struct PrepareInput {
    /// The feature to prepare an execution board for.
    pub feature: String,
    /// Path to the execution graph JSON — workstreams with
    /// `id`/`title`/`operations`/`depends_on`/`write_contract`. Resolved
    /// against the current directory.
    pub graph_json: String,
    /// The current Ivar session whose provider supplies defaults for
    /// untargeted workstreams. `None` is fine when the graph targets every
    /// workstream explicitly.
    pub session: Option<String>,
}

/// What `ivar feature execute` did.
#[derive(Debug, Clone, Serialize)]
pub struct PrepareOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the board belongs to.
    pub feature: FeatureName,
    /// The board that was prepared.
    pub board: ExecutionBoard,
    /// Where the board was written.
    pub board_path: Utf8PathBuf,
}

impl WriteHuman for PrepareOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let workstreams = self.board.graph.workstreams.len();
        let noun = if workstreams == 1 {
            "workstream"
        } else {
            "workstreams"
        };
        writeln!(
            w,
            "Prepared execution board for `{}` ({workstreams} {noun}, {}) at {}",
            self.feature, self.board.status, self.board_path
        )
    }
}

/// Prepare an execution board for `input.feature`.
///
/// Blocked when the feature does not exist, the feature's plan has not been
/// written, the graph file is missing or unparseable, a board already exists
/// — an existing board carries a journal that overwriting would destroy —
/// the child has already closed as `integrated`, or the shared resolver
/// refuses the graph (locked promotions, unknown or cyclic dependencies, an
/// unresolvable provider, targeting conflicts, or operations the plan does
/// not document — see [`graph::resolve`]).
///
/// Targeting is resolved **before** the plan fingerprint is computed: the
/// resolved `provider`/`model`/`agent` lines are written into `plan.md`, the
/// fingerprint covers that persisted form, and the board is created from the
/// same resolved workstreams — so the plan and the board cannot disagree
/// about who runs what, and `tick` never re-decides it.
pub fn prepare(ctx: &Ctx, input: PrepareInput) -> Outcome<PrepareOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;
    let graph_path = ctx.resolve(Utf8Path::new(&input.graph_json));

    require_feature(&layout, &feature)?;
    require_no_board(&layout, &feature)?;

    // Preparing persists a fresh board: blocked once the whole child closes
    // as `integrated`.
    let feature_record =
        crate::domain::feature::Feature::read(&layout, &feature)?.ok_or_else(|| {
            Failure::blocked(
                "execute.feature_vanished",
                format!("feature `{feature}` has a directory but no feature.json"),
            )
        })?;
    crate::action::feature::ensure_not_fully_integrated(&layout, &feature_record)?;

    // Resolve and validate before writing anything: graph parsing, locked
    // contracts, dependencies, targeting, and plan-backed operations all run
    // here, and a `Blocked` resolution means nothing was mutated.
    let plan_path = layout.plan_dir(&feature).join("plan.md");
    let resolved = graph::resolve(
        &layout,
        &feature,
        &feature_record,
        input.session.as_deref(),
        &graph_path,
        &plan_path,
    )?;

    // Persist the resolved targeting before fingerprinting: the fingerprint
    // must cover exactly what the board was derived from.
    fs::write_text(&plan_path, &resolved.plan_text)?;
    let plan_fingerprint = hash::file(&plan_path)?;

    let mut board = ExecutionBoard::new(ExecutionGraph {
        plan_fingerprint,
        workstreams: resolved.workstreams,
    });
    // A freshly prepared board awaits human approval before any workstream
    // can tick.
    board.set_status(ExecutionStatus::AwaitingApproval);
    board.push_journal(JournalEntry::new(
        "board",
        "prepared",
        format!("Execution board prepared from {}", graph_path),
    ));

    board.write(&layout, &feature)?;

    let board_path = feature::board_path(&layout, &feature);
    Ok(Report::new(PrepareOutcome {
        root: layout.root().to_path_buf(),
        feature,
        board_path,
        board,
    }))
}

/// Block when the feature does not exist — execution boards belong to
/// features.
fn require_feature(layout: &Layout, feature: &FeatureName) -> Result<(), Failure> {
    if fs::is_dir(&layout.feature_dir(feature))? {
        return Ok(());
    }
    Err(Failure::blocked(
        "execute.feature_not_found",
        format!("feature `{feature}` does not exist"),
    )
    .expected("an existing feature to prepare an execution board for")
    .actual(format!("`{feature}` has no feature directory"))
    .fix(FixAction::safe(
        "feature.create_first",
        format!("Create the feature first with `ivar feature create {feature}`."),
    )))
}

/// Block when the feature already has a board. Re-preparing would overwrite
/// the journal, so it takes a deliberate deletion instead.
fn require_no_board(layout: &Layout, feature: &FeatureName) -> Result<(), Failure> {
    if ExecutionBoard::read(layout, feature)?.is_none() {
        return Ok(());
    }
    let path = feature::board_path(layout, feature);
    Err(Failure::blocked(
        "execute.board_already_exists",
        format!("`{path}` already holds an execution board for `{feature}`"),
    )
    .expected("a feature with no execution board yet")
    .actual("board.json already exists — re-preparing would destroy its journal")
    .fix(FixAction::safe(
        "execute.delete_board",
        format!("Delete `{path}` deliberately, then prepare again from a fresh graph."),
    )))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/prepare.rs"]
mod tests;
