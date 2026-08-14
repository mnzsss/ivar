//! Parse a plan.md's `## Operations` section into per-workstream operation
//! lists and write contracts.
//!
//! # Why this is its own module
//!
//! `operations_from_plan` was originally private to
//! [`super::replan`], the only verb that needed it. The executor prompt
//! renderer ([`super::prompt`]) needs the same parse — which workstream owns
//! which operations, per the plan's own Operations section — to know what to
//! put in an agent's hands. Copy-pasting the parser into a second module
//! would let the two forks drift on what counts as a heading, a bullet, or
//! the write-contract marker, and the format has a sharp edge (the section
//! never ends once entered — see `replan`'s module doc) that is exactly the
//! kind of behaviour a fork silently loses. One parser, two callers.
//!
//! This module is a pure relocation: the parsing logic below is unchanged
//! from `replan.rs`, only made visible to the rest of the crate.

use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};

/// One workstream's Operations as authored in a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanWorkstream {
    /// The workstream's id — the subheading text under `Operations`.
    pub(crate) id: String,
    /// The operations, in order.
    pub(crate) operations: Vec<String>,
    /// The paths the workstream may touch.
    pub(crate) write_contract: Vec<String>,
    /// The provider named in the plan, if any — `None` means the plan does
    /// not pin one (a caller session default may supply it at prepare).
    pub(crate) provider: Option<Provider>,
    /// The model selector named in the plan, if any.
    pub(crate) model: Option<String>,
    /// The agent selector named in the plan, if any.
    pub(crate) agent: Option<String>,
}

/// Parse `text`'s Operations section. See [`super::replan`]'s module doc
/// comment for the exact format; a plan without an Operations section yields
/// an empty list, which makes every board workstream affected — the
/// conservative answer when the new plan carries no operations at all.
///
/// Scalar targeting lines (`provider:`, `model:`, `agent:`) may appear in a
/// workstream block alongside the operation bullets. An unknown `provider:`
/// id is refused naming the valid options; an empty `model:`/`agent:` value
/// is refused rather than dropped silently.
pub(crate) fn operations_from_plan(text: &str) -> Result<Vec<PlanWorkstream>, Failure> {
    let mut workstreams = Vec::new();
    let mut in_operations = false;
    let mut collecting_write_contract = false;
    let mut current: Option<PlanWorkstream> = None;

    for line in text.lines() {
        let trimmed = line.trim();

        if let Some(heading) = trimmed.strip_prefix('#') {
            let title = heading.trim_start_matches('#').trim();
            if title.eq_ignore_ascii_case("operations") {
                // The section (re)starts; whatever workstream was open ends.
                if let Some(workstream) = current.take() {
                    workstreams.push(workstream);
                }
                in_operations = true;
                collecting_write_contract = false;
                continue;
            }
            if !in_operations {
                continue;
            }
            // Any other heading inside the section starts a new workstream,
            // named by the heading text.
            if let Some(workstream) = current.take() {
                workstreams.push(workstream);
            }
            current = Some(PlanWorkstream {
                id: title.to_owned(),
                operations: Vec::new(),
                write_contract: Vec::new(),
                provider: None,
                model: None,
                agent: None,
            });
            collecting_write_contract = false;
            continue;
        }

        if !in_operations {
            continue;
        }
        let Some(workstream) = current.as_mut() else {
            continue;
        };
        if let Some(value) = trimmed.strip_prefix("provider:") {
            let value = value.trim();
            let provider = value.parse::<Provider>().map_err(|_| {
                Failure::blocked(
                    "execute.plan_provider_invalid",
                    format!(
                        "workstream `{}` names unknown provider `{value}`; \
                         valid providers are `claude-code` and `opencode`",
                        workstream.id
                    ),
                )
                .expected("`claude-code` or `opencode`")
                .actual(value)
                .fix(FixAction::safe(
                    "execute.plan_provider_fix",
                    "Set `provider:` to `claude-code` or `opencode`.",
                ))
            })?;
            workstream.provider = Some(provider);
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("model:") {
            workstream.model = non_empty_target("model", &workstream.id, value)?;
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("agent:") {
            workstream.agent = non_empty_target("agent", &workstream.id, value)?;
            continue;
        }
        if trimmed == "write_contract:" {
            collecting_write_contract = true;
            continue;
        }
        if let Some(bullet) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let item = bullet.trim().to_owned();
            if collecting_write_contract {
                workstream.write_contract.push(item);
            } else {
                workstream.operations.push(item);
            }
        }
    }
    if let Some(workstream) = current {
        workstreams.push(workstream);
    }

    Ok(workstreams)
}

/// Refuse an empty `model:`/`agent:` value so a bare selector is an explicit
/// authoring mistake, not a silently dropped one.
fn non_empty_target(field: &str, workstream: &str, value: &str) -> Result<Option<String>, Failure> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Failure::blocked(
            "execute.plan_target_empty",
            format!("workstream `{workstream}` has an empty `{field}:` value"),
        )
        .expected(format!("a non-empty {field} selector, or no `{field}:` line"))
        .actual("an empty value"));
    }
    Ok(Some(value.to_owned()))
}

/// One workstream's resolved targeting, ready to be written back into the
/// plan's `## Operations` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedTarget {
    /// The workstream's id — the `### <id>` heading it targets.
    pub(crate) id: String,
    /// The provider the workstream will run on.
    pub(crate) provider: Provider,
    /// The model selector, if one was resolved.
    pub(crate) model: Option<String>,
    /// The agent selector, if one was resolved.
    pub(crate) agent: Option<String>,
}

/// Rewrite targeting metadata into `text`'s `## Operations` section.
///
/// For every target, the matching `### <id>` heading is followed by the
/// resolved lines in stable order — `provider`, `model`, `agent` — and any
/// `provider:`/`model:`/`agent:` lines already in that block are removed, so
/// the resolved value is the only one that survives. Everything else in the
/// plan is preserved byte for byte, and the file's final newline state is
/// kept. A target with no matching heading is refused with
/// `execute.plan_workstream_missing`.
pub(crate) fn write_targets(text: &str, targets: &[ResolvedTarget]) -> Result<String, Failure> {
    let mut out: Vec<String> = Vec::new();
    let mut in_operations = false;
    // The id of the block whose targeting lines are being replaced, while the
    // block is open.
    let mut rewriting: Option<&str> = None;
    let mut written: Vec<&str> = Vec::with_capacity(targets.len());

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix('#') {
            let title = heading.trim_start_matches('#').trim();
            if title.eq_ignore_ascii_case("operations") {
                in_operations = true;
                rewriting = None;
                out.push(line.to_owned());
                continue;
            }
            if in_operations {
                if let Some(target) = targets.iter().find(|target| target.id == title) {
                    out.push(line.to_owned());
                    push_target_lines(&mut out, target);
                    written.push(target.id.as_str());
                    rewriting = Some(target.id.as_str());
                    continue;
                }
            }
            rewriting = None;
            out.push(line.to_owned());
            continue;
        }

        if rewriting.is_some()
            && (trimmed.starts_with("provider:")
                || trimmed.starts_with("model:")
                || trimmed.starts_with("agent:"))
        {
            // The resolved lines were already written after the heading; drop
            // the block's own copies.
            continue;
        }
        out.push(line.to_owned());
    }

    if let Some(missing) = targets.iter().find(|target| !written.contains(&target.id.as_str())) {
        return Err(Failure::blocked(
            "execute.plan_workstream_missing",
            format!(
                "the plan has no `### {}` workstream heading to write targeting into",
                missing.id
            ),
        )
        .expected("a `### <workstream-id>` heading for every graph workstream")
        .actual(format!("no heading matches workstream `{}`", missing.id))
        .fix(FixAction::safe(
            "execute.fix_plan_or_graph",
            format!(
                "Add a `### {}` heading under the plan's Operations section, or remove the workstream from the graph.",
                missing.id
            ),
        )));
    }

    let mut result = out.join("\n");
    if text.ends_with('\n') {
        result.push('\n');
    }
    Ok(result)
}

/// Append a target's resolved lines, in stable order: provider, model, agent.
fn push_target_lines(out: &mut Vec<String>, target: &ResolvedTarget) {
    out.push(format!("provider: {}", target.provider));
    if let Some(model) = &target.model {
        out.push(format!("model: {model}"));
    }
    if let Some(agent) = &target.agent {
        out.push(format!("agent: {agent}"));
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/execute/plan_ops.rs"]
mod tests;
