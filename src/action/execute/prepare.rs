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
use serde::{Deserialize, Serialize};

use crate::domain::feature::{
    ExecutionBoard, ExecutionGraph, ExecutionStatus, JournalEntry, WorkstreamDef, WorkstreamStatus,
};
use crate::domain::name::FeatureName;
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::{fs, hash, json};
use crate::store::feature;
use crate::store::layout::Layout;

use super::super::discover_hall;
use super::targeting;
use crate::action::Ctx;

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

/// The shape of the graph JSON `--graph-json` points at: workstreams as
/// authored, with no execution state. `status` is added when the board is
/// prepared, and `plan_fingerprint` is derived from `plan.md`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphFile {
    workstreams: Vec<GraphWorkstream>,
}

/// One workstream as authored in the graph JSON.
///
/// `provider`, `model` and `agent` are all optional and default to `None` on
/// a missing key — a graph authored before they existed carries only the
/// original five fields and must keep parsing unchanged. `#[serde(
/// deny_unknown_fields)]` stays: an unrecognised key is still refused, only a
/// *known* absent key is tolerated.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphWorkstream {
    id: String,
    title: String,
    operations: Vec<String>,
    depends_on: Vec<String>,
    write_contract: Vec<String>,
    /// The provider to run this workstream on. Parses through
    /// [`Provider`]'s own `Deserialize`, so an id outside the closed set
    /// (`claude-code`, `opencode`) is refused with a message naming the
    /// valid options — never silently coerced to `None`.
    #[serde(default)]
    provider: Option<Provider>,
    /// The model to run this workstream with, e.g. `claude --model` or
    /// `opencode -m`. Distinct from `agent` — see [`WorkstreamDef::model`].
    #[serde(default)]
    model: Option<String>,
    /// The agent to run this workstream with, e.g. `claude --agent` or
    /// `opencode --agent`. Distinct from `model` — see
    /// [`WorkstreamDef::agent`].
    #[serde(default)]
    agent: Option<String>,
}

impl From<GraphWorkstream> for WorkstreamDef {
    fn from(workstream: GraphWorkstream) -> Self {
        Self {
            id: workstream.id,
            title: workstream.title,
            operations: workstream.operations,
            depends_on: workstream.depends_on,
            write_contract: workstream.write_contract,
            status: WorkstreamStatus::Waiting,
            provider: workstream.provider,
            model: workstream.model,
            agent: workstream.agent,
        }
    }
}

/// Prepare an execution board for `input.feature`.
///
/// Blocked when the feature does not exist, the feature's plan has not been
/// written, the graph file is missing or unparseable, a board already exists
/// — an existing board carries a journal that overwriting would destroy —
/// the child has already closed as `integrated`, a workstream's write
/// contract reaches a locked promotion, a workstream's provider cannot be
/// resolved (no explicit target and no caller session), the graph and the
/// plan disagree about targeting, or the plan does not document an operation
/// a workstream claims (see [`require_plan_backs_the_graph`]).
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
    // as `integrated`, and the graph's contracts must not reach a locked
    // promotion.
    let feature_record =
        crate::domain::feature::Feature::read(&layout, &feature)?.ok_or_else(|| {
            Failure::blocked(
                "execute.feature_vanished",
                format!("feature `{feature}` has a directory but no feature.json"),
            )
        })?;
    crate::action::feature::ensure_not_fully_integrated(&layout, &feature_record)?;

    let authored = read_workstreams(&graph_path)?;
    // The contract check runs on the *authored* graph, before the resolved
    // plan is persisted: targeting never touches `write_contract`, so the
    // answer is the same either way, and a blocked prepare must not leave a
    // rewritten `plan.md` behind.
    crate::action::feature::ensure_contracts_avoid_locked_promotions(
        &layout,
        &feature_record,
        &authored,
    )?;

    let plan_path = layout.plan_dir(&feature).join("plan.md");
    let plan_text = fs::read_text(&plan_path)?.ok_or_else(|| plan_missing(&feature))?;
    let resolved = targeting::resolve(
        &layout,
        &feature,
        input.session.as_deref(),
        &plan_text,
        authored,
    )?;
    // Persist the resolved targeting before fingerprinting: the fingerprint
    // must cover exactly what the board was derived from.
    fs::write_text(&plan_path, &resolved.plan_text)?;
    let plan_fingerprint = hash::file(&plan_path)?;
    require_plan_backs_the_graph(&resolved.plan_text, &resolved.workstreams)?;

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

/// Refuse a graph whose workstreams the plan does not back.
///
/// The check is [`super::prompt::render`] itself, run over every workstream
/// and its output thrown away, because the question is exactly "will `tick`
/// be able to hand this workstream a prompt?" — and any re-implementation of
/// it here would be a second opinion free to drift from the first.
///
/// It belongs at `prepare` rather than only at `tick`. Both refuse the same
/// plan, but `tick` refuses it *after* a human has approved the graph, after
/// the smart fetch, with the board already live — and the plan gate upstream
/// is closed by then, so the fix is a replan. Here the board does not exist
/// yet, nothing has been approved, and the answer is to edit `plan.md`.
///
/// `plan_text` is the already-resolved text (targeting lines included), not a
/// re-read from disk: the caller persists it first and fingerprints the
/// persisted form, so the check must run against the same content the board
/// is derived from.
fn require_plan_backs_the_graph(
    plan_text: &str,
    workstreams: &[WorkstreamDef],
) -> Result<(), Failure> {
    for workstream in workstreams {
        super::prompt::render(plan_text, workstream, &[])?;
    }
    Ok(())
}

/// The "no plan.md under the feature's plan directory" refusal, shared by
/// every path that needs the plan's text.
fn plan_missing(feature: &FeatureName) -> Failure {
    Failure::blocked(
        "execute.plan_missing",
        format!("the plan for `{feature}` does not exist"),
    )
    .expected("the feature's plan to have been written")
    .actual("no plan.md under the feature's plan directory")
    .fix(FixAction::safe(
        "plan.create_first",
        format!("Scaffold the plan first: `ivar plan create {feature}`."),
    ))
}

/// Parse the graph JSON at `path` into the graph's workstreams. A missing
/// file is blocked; unparseable JSON fails with the path and parse position
/// from `infra::json`.
pub(crate) fn read_workstreams(path: &Utf8Path) -> Result<Vec<WorkstreamDef>, Failure> {
    let file: GraphFile = json::read(path)?.ok_or_else(|| {
        Failure::blocked("execute.graph_missing", format!("`{path}` does not exist"))
            .expected("an execution graph JSON file at the given path")
            .actual("no such file")
            .fix(FixAction::safe(
                "execute.provide_graph",
                "Point --graph-json at a file describing the plan's workstreams.",
            ))
    })?;
    Ok(file
        .workstreams
        .into_iter()
        .map(WorkstreamDef::from)
        .collect())
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/prepare.rs"]
mod tests;
