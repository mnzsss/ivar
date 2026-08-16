//! Resolve execution targeting for a prepared board.
//!
//! # What it does
//!
//! `prepare` hands this module the plan's text and the graph's workstreams,
//! plus the caller's session id. It merges the three sources of targeting —
//! the graph JSON, the plan's `## Operations` blocks, and (for providers the
//! first two do not pin) the caller session's `state.json` — and produces the
//! two artifacts `prepare` persists:
//!
//! - `workstreams`, with `provider` filled in for **every** workstream, and
//! - `plan_text`, the plan with the resolved `provider`/`model`/`agent`
//!   lines written back into each workstream's block.
//!
//! # The merge rules
//!
//! For each of `provider`, `model`, `agent`: when the graph and the plan both
//! carry a value they must agree — a drift is refused with
//! `execute.targeting_conflict` naming the workstream, the field and both
//! values. When only one carries a value, it wins. `model` and `agent` may
//! stay unset (the provider's default); `provider` must resolve to a value,
//! falling back to the caller session's provider when neither graph nor plan
//! pins one.
//!
//! The caller session is only read when at least one workstream actually
//! needs its provider — a fully-targeted graph is executable outside any
//! session. When a provider is needed and no readable caller session is
//! supplied, `prepare` refuses with `execute.provider_context_missing`: it
//! never falls back silently to the hall default, because the hall default is
//! a choice `tick` should not make for a workstream nobody targeted.

use crate::domain::feature::WorkstreamDef;
use crate::domain::name::FeatureName;
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::store::layout::Layout;

use super::super::session::lookup;
use super::plan_ops::{self, ResolvedTarget};

/// The synchronized result of targeting resolution: workstreams every one of
/// which carries an explicit `provider`, and the plan text with the same
/// targeting written into its `## Operations` blocks.
pub(crate) struct ResolvedExecution {
    /// The workstreams, with `provider` resolved.
    pub(crate) workstreams: Vec<WorkstreamDef>,
    /// The plan text with resolved targeting lines written back in.
    pub(crate) plan_text: String,
}

/// Resolve every workstream's targeting from the graph, the plan, and the
/// caller session (when needed), synchronizing the plan text to match.
pub(crate) fn resolve(
    layout: &Layout,
    feature: &FeatureName,
    session: Option<&str>,
    plan_text: &str,
    mut workstreams: Vec<WorkstreamDef>,
) -> Result<ResolvedExecution, Failure> {
    let plan_workstreams = plan_ops::operations_from_plan(plan_text)?;

    // Merge the graph and the plan per field, remembering which workstreams
    // still need a provider from the caller session.
    let mut merged = Vec::with_capacity(workstreams.len());
    let mut needs_caller = false;
    for workstream in &workstreams {
        let plan_entry = plan_workstreams
            .iter()
            .find(|entry| entry.id == workstream.id);
        let provider = merge_field(
            "provider",
            &workstream.id,
            workstream.provider,
            plan_entry.and_then(|entry| entry.provider),
        )?;
        let model = merge_field(
            "model",
            &workstream.id,
            workstream.model.clone(),
            plan_entry.and_then(|entry| entry.model.clone()),
        )?;
        let agent = merge_field(
            "agent",
            &workstream.id,
            workstream.agent.clone(),
            plan_entry.and_then(|entry| entry.agent.clone()),
        )?;
        needs_caller |= provider.is_none();
        merged.push((provider, model, agent));
    }

    // Resolve the caller session only when something needs its provider —
    // fully-targeted graphs stay executable outside any session.
    let caller = if needs_caller {
        Some(caller_provider(layout, feature, session)?)
    } else {
        None
    };

    let mut targets = Vec::with_capacity(workstreams.len());
    for (workstream, (provider, model, agent)) in workstreams.iter_mut().zip(merged) {
        let provider = match provider {
            Some(provider) => provider,
            None => caller.ok_or_else(provider_context_missing)?,
        };
        // The board carries the resolved selectors, not just the graph's
        // authored ones — a `model`/`agent` the plan pins is as binding as
        // one the graph does, so plan and board cannot disagree.
        workstream.provider = Some(provider);
        workstream.model = model.clone();
        workstream.agent = agent.clone();
        targets.push(ResolvedTarget {
            id: workstream.id.clone(),
            provider,
            model,
            agent,
        });
    }

    let plan_text = plan_ops::write_targets(plan_text, &targets)?;

    Ok(ResolvedExecution {
        workstreams,
        plan_text,
    })
}

/// Merge one targeting field from the graph and the plan. Both carrying
/// different values is a refusal — the two artifacts must not drift silently;
/// otherwise the value that is present wins.
fn merge_field<T>(
    field: &str,
    workstream: &str,
    graph: Option<T>,
    plan: Option<T>,
) -> Result<Option<T>, Failure>
where
    T: PartialEq + std::fmt::Display,
{
    match (graph, plan) {
        (Some(graph), Some(plan)) if graph != plan => Err(Failure::blocked(
            "execute.targeting_conflict",
            format!(
                "workstream `{workstream}` targets `{field}` as `{graph}` in the graph but `{plan}` in the plan"
            ),
        )
        .expected("the graph and the plan to name the same provider, model and agent")
        .actual(format!("graph `{graph}`; plan `{plan}`"))
        .fix(FixAction::safe(
            "execute.align_graph_and_plan",
            format!(
                "Set `{field}` identically in the graph JSON and the plan's `### {workstream}` block, then prepare again."
            ),
        ))),
        (Some(value), _) | (_, Some(value)) => Ok(Some(value)),
        (None, None) => Ok(None),
    }
}

/// The provider that launched the caller's session — the default for any
/// workstream neither the graph nor the plan targets.
///
/// Refused with `execute.provider_context_missing` when no session id was
/// supplied, and with `execute.session_state_missing` when the session exists
/// but its `state.json` is unreadable. Never a silent hall-default fallback.
fn caller_provider(
    layout: &Layout,
    feature: &FeatureName,
    session: Option<&str>,
) -> Result<Provider, Failure> {
    let Some(session) = session else {
        return Err(provider_context_missing());
    };
    let session = lookup::resolve(layout, Some(session), Some(feature.as_str()))?;
    let state = session.state.ok_or_else(|| {
        Failure::blocked(
            "execute.session_state_missing",
            format!("session `{}` has no readable state.json", session.id),
        )
        .expected("a session record containing its provider")
        .actual("state.json is missing or unreadable")
        .fix(FixAction::safe(
            "execute.session_start",
            "Start a session first with `ivar session start`, or pass a session id that has one.",
        ))
    })?;
    Ok(state.provider())
}

/// The shared "provider needed but no caller session supplied" refusal.
fn provider_context_missing() -> Failure {
    Failure::blocked(
        "execute.provider_context_missing",
        "one or more workstreams have no provider and no caller session was supplied",
    )
    .expected("an explicit provider per workstream, or `--session <id>`")
    .actual("provider and caller session are both absent")
    .fix(FixAction::safe(
        "execute.pass_session",
        "Run prepare from `/ivar-execute`, or pass the current IVAR_SESSION_ID with `--session`.",
    ))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/targeting.rs"]
mod tests;
