//! Shared authored-graph parsing and complete plan/graph resolution.
//!
//! # Why this is its own module
//!
//! `prepare` and `replan` both accept an execution graph — a plain JSON file
//! of workstreams with `id`/`title`/`operations`/`depends_on`/
//! `write_contract` and optional `provider`/`model`/`agent` targeting — and
//! both must apply exactly the same checks before the graph may become a
//! board's authoritative graph: parse the authored JSON, refuse a write
//! contract reaching a locked promotion, refuse unknown or cyclic
//! dependencies, resolve targeting defaults, and refuse a graph whose
//! operations the plan does not back. Copy-pasting any of that into a second
//! module would let the two forks drift on what counts as a workstream, a
//! target, or a backed operation. One parser, one resolver, two callers.
//!
//! The resolver's result is the synchronized pair `prepare` and `replan`
//! persist: the workstreams with targeting resolved and providers pinned,
//! and the plan text with the same targeting written back into its
//! `## Operations` blocks — so the plan and the board cannot disagree about
//! who runs what, and `tick` never re-decides it.
//!
//! Lifecycle policy stays in the callers: `prepare` remains responsible for
//! one-shot board creation, `replan` for merging execution state into an
//! existing board. This module only answers "what is the resolved graph?".

use camino::Utf8Path;
use serde::Deserialize;

use crate::domain::feature::{Feature, WorkstreamDef, WorkstreamStatus};
use crate::domain::name::FeatureName;
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::infra::{fs, json};
use crate::store::layout::Layout;

use super::targeting;

/// The shape of the graph JSON `--graph-json` points at: workstreams as
/// authored, with no execution state. `status` is added when a board is
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

/// The synchronized result of complete graph resolution: the workstreams
/// every one of which carries an explicit `provider`, and the plan text with
/// the same targeting written into its `## Operations` blocks.
pub(crate) struct ResolvedGraph {
    /// The plan text with resolved targeting lines written back in.
    pub(crate) plan_text: String,
    /// The resolved workstreams, with `provider` pinned.
    pub(crate) workstreams: Vec<WorkstreamDef>,
}

/// Resolve the authored graph at `graph_path` against the plan at
/// `plan_path`, for `feature_record`, and return the pair `prepare` and
/// `replan` persist.
///
/// Blocked when the graph file is missing or unparseable, a workstream's
/// write contract reaches a locked promotion, a dependency names a
/// workstream that is not on the graph or the graph has a dependency cycle,
/// a workstream's provider cannot be resolved (no explicit target and no
/// caller session), the graph and the plan disagree about targeting, or the
/// plan does not document an operation a workstream claims (see
/// [`require_plan_backs_the_graph`]).
///
/// Targeting is resolved **before** the caller computes the plan fingerprint:
/// the resolved `provider`/`model`/`agent` lines are written into the plan
/// text, the fingerprint covers that persisted form, and the board is created
/// from the same resolved workstreams.
///
/// The resolver never writes anything: the contract check and the
/// plan-backing check run on the resolved text before the caller persists it,
/// so a blocked resolution leaves `plan.md` byte untouched.
pub(crate) fn resolve(
    layout: &Layout,
    feature: &FeatureName,
    feature_record: &Feature,
    session: Option<&str>,
    graph_path: &Utf8Path,
    plan_path: &Utf8Path,
) -> Result<ResolvedGraph, Failure> {
    let authored = read_workstreams(graph_path)?;
    // The contract check runs on the *authored* graph, before the resolved
    // plan is persisted: targeting never touches `write_contract`, so the
    // answer is the same either way, and a blocked resolution must not leave
    // a rewritten `plan.md` behind.
    crate::action::feature::ensure_contracts_avoid_locked_promotions(
        layout,
        feature_record,
        &authored,
    )?;
    validate_dependencies(&authored)?;

    let plan_text = fs::read_text(plan_path)?.ok_or_else(|| plan_missing(feature, plan_path))?;
    let resolved = targeting::resolve(layout, feature, session, &plan_text, authored)?;
    // Validate before persisting: `Blocked` means nothing was mutated, so a
    // plan that does not back the graph must be refused before `plan.md` is
    // rewritten with the resolved targeting.
    require_plan_backs_the_graph(&resolved.plan_text, &resolved.workstreams)?;

    Ok(ResolvedGraph {
        plan_text: resolved.plan_text,
        workstreams: resolved.workstreams,
    })
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

/// Refuse a graph whose dependencies can never be satisfied: an id that is
/// not a workstream on the graph, or a cycle of dependencies. Either makes
/// the board un-tickable — nothing on a broken edge ever becomes ready — and
/// a replan that adopted it would strand the journal behind an unsatisfiable
/// graph.
fn validate_dependencies(workstreams: &[WorkstreamDef]) -> Result<(), Failure> {
    // Unknown ids first — a cycle error would be a lie about a graph that is
    // also broken in a more basic way.
    for workstream in workstreams {
        for dependency in &workstream.depends_on {
            if !workstreams.iter().any(|other| other.id == *dependency) {
                return Err(unknown_dependency(workstream, dependency));
            }
        }
    }

    // Depth-first walk over the dependency edges, tracking which workstreams
    // are still on the current path. A dependency edge onto a workstream
    // already on the path is a cycle.
    fn visit<'a>(
        workstream: &'a WorkstreamDef,
        workstreams: &'a [WorkstreamDef],
        on_path: &mut Vec<&'a str>,
    ) -> Result<(), Failure> {
        on_path.push(workstream.id.as_str());
        for dependency in &workstream.depends_on {
            if on_path.contains(&dependency.as_str()) {
                return Err(cycle(workstream, dependency));
            }
            // Unknown ids were refused above; `continue` keeps the walk total
            // if this ever happens anyway.
            let Some(dependent) = workstreams.iter().find(|ws| &ws.id == dependency) else {
                continue;
            };
            visit(dependent, workstreams, on_path)?;
        }
        on_path.pop();
        Ok(())
    }

    let mut on_path = Vec::new();
    for workstream in workstreams {
        visit(workstream, workstreams, &mut on_path)?;
    }
    Ok(())
}

/// The refusal for a `depends_on` id that names no workstream on the graph.
fn unknown_dependency(workstream: &WorkstreamDef, dependency: &str) -> Failure {
    Failure::blocked(
        "execute.dependency_unknown",
        format!(
            "workstream `{}` depends on `{dependency}`, which is not a workstream on the graph",
            workstream.id
        ),
    )
    .expected("every `depends_on` id to name a workstream in the same graph")
    .actual(format!("no workstream has id `{dependency}`"))
    .fix(FixAction::safe(
        "execute.fix_dependency",
        format!("Name an existing workstream id, or add a workstream with id `{dependency}`."),
    ))
}

/// The refusal for a dependency edge that closes a cycle.
fn cycle(workstream: &WorkstreamDef, dependency: &str) -> Failure {
    Failure::blocked(
        "execute.dependency_cycle",
        format!(
            "workstream `{}` depends on `{dependency}`, which depends back on it — a cycle no board can ever satisfy",
            workstream.id
        ),
    )
    .expected("an acyclic dependency graph")
    .actual("at least two workstreams depend on each other, directly or transitively")
    .fix(FixAction::safe(
        "execute.break_cycle",
        "Remove or reorder the circular `depends_on` edges so every dependency can finish before its dependents start.",
    ))
}

/// Refuse a graph whose workstreams the plan does not back.
///
/// The check is [`super::prompt::render`] itself, run over every workstream
/// and its output thrown away, because the question is exactly "will `tick`
/// be able to hand this workstream a prompt?" — and any re-implementation of
/// it here would be a second opinion free to drift from the first.
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

/// The "no plan.md to resolve against" refusal, shared by every path that
/// needs the plan's text — `prepare` reads the feature's own plan, `replan`
/// the revised plan the caller points at.
fn plan_missing(feature: &FeatureName, path: &Utf8Path) -> Failure {
    Failure::blocked(
        "execute.plan_missing",
        format!("the plan for `{feature}` does not exist at `{path}`"),
    )
    .expected("a plan.md to resolve the graph against")
    .actual("no such file")
    .fix(FixAction::safe(
        "execute.provide_plan",
        format!("Write the plan under `plans/{feature}/plan.md`, or point --plan at the revised plan.md."),
    ))
}
