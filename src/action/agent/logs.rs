//! `ivar agent logs`: the captured output of an agent run.

use std::io;

use serde::Serialize;

use crate::action::{Ctx, discover_hall};
use crate::domain::agent::receipt::AgentRunOutcome;
use crate::domain::name::FeatureName;
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
/// Returns a [`Failure`] when the name is invalid, the agent is unknown, has
/// never run, or the requested run does not exist. The agent's file is not
/// required: a concluded Agent Task has moved to `agents/done/`.
pub fn logs(ctx: &Ctx, input: LogsInput) -> Outcome<LogsOutcome> {
    let layout = discover_hall(ctx)?;
    let LogsInput { name, run } = input;
    // The name becomes a path under `.ivar/agents/runs/`: validate it first.
    FeatureName::new(format!("agent-{name}")).map_err(|e| {
        Failure::failed(
            "agent.invalid",
            format!("`{name}` is not a valid agent name: {e}"),
        )
    })?;
    let receipt = match &run {
        Some(run) => store::agent::read_receipt(&layout, &name, run)?.ok_or_else(|| {
            Failure::blocked("agent.run_not_found", format!("no agent run `{run}`"))
        })?,
        None => match store::agent::latest_receipt(&layout, &name)? {
            Some(receipt) => receipt,
            None if store::agent::name_exists(&layout, &name)? => {
                return Err(Failure::blocked(
                    "agent.no_runs",
                    format!("agent `{name}` has not run yet"),
                ));
            }
            None => {
                return Err(Failure::failed(
                    "agent.not_found",
                    format!(
                        "no agent `{name}` in {}, {} or {}",
                        layout.agents_dir(),
                        layout.agents_done_dir(),
                        layout.agent_state_dir().join("runs")
                    ),
                ));
            }
        },
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
