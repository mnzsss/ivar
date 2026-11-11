use clap::{Args, Subcommand};

use crate::action::agent::feedback::FeedbackInput;
use crate::action::agent::logs::LogsInput;

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// Install, update and remove the systemd timers for agents/*.md.
    Sync,
    /// List agents with schedule (or a task's `at`), next run, last outcome and timer state.
    Status,
    /// Run an agent now, through the same path its timer uses. A run that
    /// succeeds commits what changed under the agent's `write:` roots that lie
    /// inside the hall as `docs(<agent>): Record the <YYYY-MM-DD> run`, then every
    /// other pending hall change as `chore(hall): Sync pending hall changes`, and
    /// pushes the branch. Roots outside the hall are ignored, and a run that fails
    /// commits nothing. `auto_commit: false` in `ivar.json` turns all of that off
    /// and `auto_push: false` only the push. A hall sync that cannot finish is
    /// recorded in the receipt's `sync_warnings` and never changes the outcome.
    Run(AgentRunArgs),
    /// Print the output of the latest run, or of --run <id>.
    Logs(AgentLogsArgs),
    /// Record your verdict on a run.
    Feedback(AgentFeedbackArgs),
    /// Set up where agents send their run notifications.
    #[command(subcommand)]
    Notify(NotifyCommand),
    /// Apply a Landlock write sandbox, then become the given command.
    #[command(hide = true)]
    Sandbox(AgentSandboxArgs),
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

#[derive(Debug, Args)]
pub struct AgentSandboxArgs {
    /// A path the command may write; repeatable.
    #[arg(long = "write", value_name = "PATH")]
    pub write: Vec<String>,
    /// The command to run once the sandbox is applied.
    #[arg(last = true, required = true, allow_hyphen_values = true)]
    pub command: Vec<String>,
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

#[derive(Debug, Subcommand)]
pub enum NotifyCommand {
    /// Notifications through a Telegram bot.
    #[command(subcommand)]
    Telegram(TelegramCommand),
}

#[derive(Debug, Subcommand)]
pub enum TelegramCommand {
    /// Save the bot token, find the chat, send a test message and set `notify` on agents.
    Setup,
}

#[cfg(test)]
#[path = "../../tests/unit/cli/agent.rs"]
mod tests;
