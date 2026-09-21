use clap::{Args, Subcommand, ValueEnum};

use crate::action::review::comment as review_comment;

/// The `ivar review` surface: local review of a feature's changes.
#[derive(Debug, Subcommand)]
pub enum ReviewCommand {
    /// Manage line-range review comments.
    #[command(subcommand)]
    Comment(CommentCommand),
}

/// `ivar review comment add|list|resolve`.
#[derive(Debug, Subcommand)]
pub enum CommentCommand {
    /// Add a comment on a line range of a file in one of the feature's repos.
    Add(CommentAddArgs),
    /// List a feature's review comments.
    List(CommentListArgs),
    /// Mark a review comment resolved.
    Resolve(CommentResolveArgs),
}

/// Arguments for `ivar review comment add`.
#[derive(Debug, Args)]
pub struct CommentAddArgs {
    /// The feature under review.
    pub feature: String,
    /// The repo the file belongs to.
    #[arg(long)]
    pub repo: String,
    /// The file path, relative to the repo root.
    #[arg(long, allow_hyphen_values = true)]
    pub file: String,
    /// `<n>` or `<n>-<m>`, 1-based and inclusive.
    #[arg(long)]
    pub lines: String,
    /// The comment text.
    #[arg(long, allow_hyphen_values = true)]
    pub body: String,
}

/// Arguments for `ivar review comment list`.
#[derive(Debug, Args)]
pub struct CommentListArgs {
    /// The feature under review.
    pub feature: String,
    /// Only comments on this repo.
    #[arg(long)]
    pub repo: Option<String>,
    /// Only comments with this status.
    #[arg(long, value_enum)]
    pub status: Option<CommentStatusArg>,
}

/// `--status` values for `ivar review comment list`.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CommentStatusArg {
    Open,
    Resolved,
}

/// Arguments for `ivar review comment resolve`.
#[derive(Debug, Args)]
pub struct CommentResolveArgs {
    /// The feature under review.
    pub feature: String,
    /// The comment id, e.g. `c1`.
    pub id: String,
}

impl From<CommentAddArgs> for review_comment::AddInput {
    fn from(args: CommentAddArgs) -> Self {
        let CommentAddArgs {
            feature,
            repo,
            file,
            lines,
            body,
        } = args;
        Self {
            feature,
            repo,
            file,
            lines,
            body,
        }
    }
}

impl From<CommentListArgs> for review_comment::ListInput {
    fn from(args: CommentListArgs) -> Self {
        let CommentListArgs {
            feature,
            repo,
            status,
        } = args;
        Self {
            feature,
            repo,
            status: status.map(|s| match s {
                CommentStatusArg::Open => review_comment::CommentStatus::Open,
                CommentStatusArg::Resolved => review_comment::CommentStatus::Resolved,
            }),
        }
    }
}

impl From<CommentResolveArgs> for review_comment::ResolveInput {
    fn from(args: CommentResolveArgs) -> Self {
        let CommentResolveArgs { feature, id } = args;
        Self { feature, id }
    }
}
