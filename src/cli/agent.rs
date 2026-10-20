use clap::{Args, Subcommand};

use crate::action::agent::feedback::FeedbackInput;
use crate::action::agent::logs::LogsInput;

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// Install, update and remove the systemd timers for agents/*.md.
    Sync,
    /// List agents with schedule, next run, last outcome and timer state.
    Status,
    /// Run an agent now, through the same path its timer uses.
    Run(AgentRunArgs),
    /// Print the output of the latest run, or of --run <id>.
    Logs(AgentLogsArgs),
    /// Record your verdict on a run.
    Feedback(AgentFeedbackArgs),
}

#[derive(Debug, Args)]
pub struct AgentRunArgs {
    /// Agent name (the file stem under agents/).
    pub name: String,
    #[arg(long, hide = true, default_value = "manual", value_parser = ["manual", "schedule"])]
    pub trigger: String,
}

#[derive(Debug, Args)]
pub struct AgentLogsArgs {
    /// Agent name (the file stem under agents/).
    pub name: String,
    /// Run id to print instead of the latest run.
    #[arg(long)]
    pub run: Option<String>,
}

#[derive(Debug, Args)]
pub struct AgentFeedbackArgs {
    /// Run id to rate.
    pub run_id: String,
    /// Your verdict on the run.
    #[arg(long, value_parser = ["up", "down"])]
    pub rating: String,
    /// Why, in a sentence.
    #[arg(long)]
    pub comment: Option<String>,
}

impl From<AgentLogsArgs> for LogsInput {
    fn from(args: AgentLogsArgs) -> Self {
        let AgentLogsArgs { name, run } = args;
        Self { name, run }
    }
}

impl From<AgentFeedbackArgs> for FeedbackInput {
    fn from(args: AgentFeedbackArgs) -> Self {
        let AgentFeedbackArgs {
            run_id,
            rating,
            comment,
        } = args;
        Self {
            run_id,
            rating,
            comment,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/cli/agent.rs"]
mod tests;
