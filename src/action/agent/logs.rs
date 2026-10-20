//! `ivar agent logs`: the captured output of an agent run.

use std::io;

use serde::Serialize;

use crate::action::{Ctx, discover_hall};
use crate::domain::agent::receipt::AgentRunOutcome;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::store;

#[derive(Debug)]
pub struct LogsInput {
    pub name: String,
    pub run: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogsOutcome {
    pub run_id: String,
    pub agent: String,
    pub outcome: AgentRunOutcome,
    pub output: String,
}

impl WriteHuman for LogsOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        w.write_all(self.output.as_bytes())
    }
}

/// # Errors
///
/// Returns a [`Failure`] when the agent is unknown, has never run, or the
/// requested run does not exist.
pub fn logs(ctx: &Ctx, input: LogsInput) -> Outcome<LogsOutcome> {
    let layout = discover_hall(ctx)?;
    let LogsInput { name, run } = input;
    store::agent::load(&layout, &name)?;
    let receipt = match &run {
        Some(run) => store::agent::read_receipt(&layout, &name, run)?.ok_or_else(|| {
            Failure::blocked("agent.run_not_found", format!("no agent run `{run}`"))
        })?,
        None => store::agent::latest_receipt(&layout, &name)?.ok_or_else(|| {
            Failure::blocked("agent.no_runs", format!("agent `{}` has not run yet", name))
        })?,
    };
    Ok(Report::new(LogsOutcome {
        run_id: receipt.run_id,
        agent: receipt.agent,
        outcome: receipt.outcome,
        output: receipt.output,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/agent/logs.rs"]
mod tests;
