//! `ivar agent resume`: run again the agents whose latest run is blocked.

use std::io;

use serde::Serialize;

use super::run::{self, HostDeps};
use crate::action::{Ctx, discover_hall};
use crate::domain::agent::receipt::{AgentRunOutcome, AgentRunReceipt, Trigger};
use crate::domain::agent::spec::AgentSpec;
use crate::domain::name::FeatureName;
use crate::error::{Failure, Outcome, Report, Warning, WriteHuman};
use crate::store;
use crate::store::layout::Layout;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeInput {
    /// One agent; every agent with a blocked latest run when absent.
    pub name: Option<String>,
}

/// One blocked run, run again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Resumed {
    pub agent: String,
    /// The blocked run this one replaces.
    pub blocked_run: String,
    pub run_id: String,
    pub outcome: AgentRunOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResumeOutcome {
    pub resumed: Vec<Resumed>,
}

impl WriteHuman for ResumeOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.resumed.is_empty() {
            return writeln!(w, "No blocked agent runs to resume.");
        }
        for run in &self.resumed {
            writeln!(
                w,
                "{} resumed {}: {} ({})",
                run.agent,
                run.blocked_run,
                run.outcome.as_str(),
                run.run_id
            )?;
        }
        Ok(())
    }
}

/// Runs, one after the other, each agent whose latest receipt is `blocked`
/// (see [`AgentRunReceipt::is_resumable`]), as a manual run through the same
/// path as `ivar agent run --wait`: the preflight, the agent's lock, the
/// receipt and the notifications all apply. A concluded Agent Task has left
/// `agents/` and is not resumed.
///
/// # Errors
///
/// A [`Failure`] when the hall cannot be found, or `name` is not a valid agent
/// or has no blocked latest run. An agent whose file is invalid, or whose
/// receipt cannot be written, is a warning and the others still run.
pub fn resume(ctx: &Ctx, input: ResumeInput) -> Outcome<ResumeOutcome> {
    resume_with(ctx, input, &HostDeps::system, &run::resolves)
}

pub(crate) fn resume_with(
    ctx: &Ctx,
    input: ResumeInput,
    host: &dyn Fn() -> Result<HostDeps, Failure>,
    resolves: &dyn Fn(&str) -> bool,
) -> Outcome<ResumeOutcome> {
    let ResumeInput { name } = input;
    let layout = discover_hall(ctx)?;
    let mut warnings = Vec::new();
    let blocked = match &name {
        Some(name) => vec![blocked_agent(&layout, name)?],
        None => all_blocked(&layout, &mut warnings)?,
    };
    let mut resumed = Vec::new();
    for (spec, receipt) in blocked {
        match run::run_spec(ctx, &layout, &spec, Trigger::Manual, host, resolves) {
            Ok(report) => {
                warnings.extend(report.warnings);
                resumed.push(Resumed {
                    agent: spec.name,
                    blocked_run: receipt.run_id,
                    run_id: report.value.run_id,
                    outcome: report.value.outcome,
                });
            }
            Err(failure) => warnings.push(Warning::new(
                "agent.resume_failed",
                &spec.name,
                failure.what,
            )),
        }
    }
    let mut report = Report::new(ResumeOutcome { resumed });
    for warning in warnings {
        report.warn(warning);
    }
    Ok(report)
}

fn blocked_agent(layout: &Layout, name: &str) -> Result<(AgentSpec, AgentRunReceipt), Failure> {
    FeatureName::new(format!("agent-{name}")).map_err(|e| {
        Failure::failed(
            "agent.invalid",
            format!("`{name}` is not a valid agent name: {e}"),
        )
    })?;
    let spec = store::agent::load(layout, name)?;
    match store::agent::latest_receipt(layout, name)? {
        Some(receipt) if receipt.is_resumable() => Ok((spec, receipt)),
        Some(receipt) => Err(Failure::blocked(
            "agent.not_blocked",
            format!(
                "agent `{name}` has nothing to resume: its latest run, {}, is {}",
                receipt.run_id,
                receipt.outcome.as_str()
            ),
        )),
        None => Err(Failure::blocked(
            "agent.no_runs",
            format!("agent `{name}` has not run yet"),
        )),
    }
}

fn all_blocked(
    layout: &Layout,
    warnings: &mut Vec<Warning>,
) -> Result<Vec<(AgentSpec, AgentRunReceipt)>, Failure> {
    let mut blocked = Vec::new();
    for file in store::agent::list_files(layout)? {
        let Some(receipt) = store::agent::latest_receipt(layout, &file.name)? else {
            continue;
        };
        if !receipt.is_resumable() {
            continue;
        }
        match store::agent::parse(&file) {
            Ok(spec) => blocked.push((spec, receipt)),
            Err(failure) => warnings.push(Warning::new("agent.invalid", &file.name, failure.what)),
        }
    }
    Ok(blocked)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/agent/resume.rs"]
mod tests;
