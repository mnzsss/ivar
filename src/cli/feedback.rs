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
    /// Submit a feedback entry as a GitHub issue.
    Submit(FeedbackSubmitArgs),
}

#[derive(Debug, Args)]
pub struct FeedbackSubmitArgs {
    /// Feedback entry ID (e.g. 001-my-bug).
    pub id: String,

    /// GitHub target repository (defaults to mnzsss/ivar).
    #[arg(long)]
    pub repo: Option<String>,

    /// Print the redacted issue and its fingerprint; publish nothing. Needs no terminal.
    #[arg(long, conflicts_with = "fingerprint")]
    pub preview: bool,

    /// Publish without a terminal, only if the entry still matches this preview fingerprint.
    #[arg(long)]
    pub fingerprint: Option<String>,
}

impl From<FeedbackSubmitArgs> for crate::action::feedback::submit::SubmitInput {
    fn from(args: FeedbackSubmitArgs) -> Self {
        use crate::action::feedback::submit::SubmitMode;
        let mode = match (args.preview, args.fingerprint) {
            (true, _) => SubmitMode::Preview,
            (false, Some(fingerprint)) => SubmitMode::Apply { fingerprint },
            (false, None) => SubmitMode::Interactive,
        };
        Self {
            id: args.id,
            repo: args.repo,
            mode,
        }
    }
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
