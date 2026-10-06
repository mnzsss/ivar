use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct FeedbackArgs {
    #[command(subcommand)]
    pub command: FeedbackCommand,
}

#[derive(Debug, Subcommand)]
pub enum FeedbackCommand {
    /// Add a new feedback entry (bug or proposal).
    Add(FeedbackAddArgs),
    /// List recorded feedback entries.
    List(FeedbackListArgs),
    /// Show details and body of a feedback entry.
    Show(FeedbackShowArgs),
}

#[derive(Debug, Args)]
pub struct FeedbackAddArgs {
    /// Title of the feedback entry.
    #[arg(long)]
    pub title: String,

    /// Kind of feedback (bug or proposal).
    #[arg(long, default_value = "bug", value_parser = ["bug", "proposal"])]
    pub kind: String,

    /// File containing feedback description, or - for stdin.
    #[arg(long)]
    pub file: Option<String>,
}

#[derive(Debug, Args)]
pub struct FeedbackListArgs {
    /// Filter by status: open or published.
    #[arg(long, value_parser = ["open", "published"])]
    pub status: Option<String>,
}

#[derive(Debug, Args)]
pub struct FeedbackShowArgs {
    /// Feedback entry ID (e.g. 001-my-bug).
    pub id: String,

    /// Redact local paths, usernames, and hostnames in displayed output.
    #[arg(long)]
    pub redacted: bool,
}
