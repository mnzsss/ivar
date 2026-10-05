use clap::{Args, Subcommand};

use crate::action::execute::{accept_revision, finish, start, status as execute_status};
use crate::error::Failure;

/// The `ivar feature execute` Run Receipt lifecycle.
#[derive(Debug, Subcommand)]
pub enum ExecuteCommand {
    /// Start a new run, resume a blocked run, or restart a non-terminal run.
    Start(ExecuteStartArgs),
    /// Record a coordinator's structured completion report (see `--print-schema`).
    Finish(ExecuteFinishArgs),
    /// Show the current receipt, a receipt by id, or complete history.
    Status(ExecuteStatusArgs),
    /// Accept an approved plan revision for a diverged run.
    AcceptRevision(ExecuteAcceptRevisionArgs),
    /// Record an approved wave on the active run without editing the plan.
    Checkpoint(ExecuteCheckpointArgs),
    /// Abandon an active or blocked run, transitioning it to interrupted.
    Interrupt(ExecuteInterruptArgs),
}

/// Arguments for `ivar feature execute start`.
#[derive(Debug, Args)]
pub struct ExecuteStartArgs {
    pub feature: Option<String>,
    /// Plan file; defaults to `.ivar/features/<feature>/plan.md`.
    #[arg(long)]
    pub plan: Option<String>,
    #[arg(long, conflicts_with = "restart")]
    pub resume: bool,
    #[arg(long, conflicts_with = "resume")]
    pub restart: bool,
    /// Execution mode: `default` keeps every human gate; `goal` runs to the delivery gate
    /// without stopping. Omitted on `--resume`, the run keeps its recorded mode; omitted
    /// otherwise, a new run is `default`.
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(["default", "goal"]))]
    pub mode: Option<String>,
}

/// Arguments for `ivar feature execute finish`.
#[derive(Debug, Args)]
pub struct ExecuteFinishArgs {
    pub feature: Option<String>,
    /// Plan file; defaults to `.ivar/features/<feature>/plan.md`.
    #[arg(long)]
    pub plan: Option<String>,
    /// Path to the coordinator report JSON. Run with `--print-schema` for its shape.
    #[arg(long, required_unless_present = "print_schema")]
    pub report_json: Option<String>,
    /// How the run ended: succeeded, failed or blocked.
    #[arg(long, required_unless_present = "print_schema")]
    pub outcome: Option<String>,
    /// Print the coordinator report JSON schema and the accepted `--outcome` values, then exit.
    #[arg(long, conflicts_with_all = ["report_json", "outcome"])]
    pub print_schema: bool,
}

/// Arguments for `ivar feature execute status`.
#[derive(Debug, Args)]
pub struct ExecuteStatusArgs {
    pub feature: Option<String>,
    /// Plan file; defaults to `.ivar/features/<feature>/plan.md`.
    #[arg(long)]
    pub plan: Option<String>,
    #[arg(long, conflicts_with = "run")]
    pub history: bool,
    #[arg(long, conflicts_with = "history")]
    pub run: Option<String>,
}

/// Arguments for `ivar feature execute accept-revision`.
#[derive(Debug, Args)]
pub struct ExecuteAcceptRevisionArgs {
    pub feature: Option<String>,
    /// Plan file; defaults to `.ivar/features/<feature>/plan.md`.
    #[arg(long)]
    pub plan: Option<String>,
}

/// Arguments for `ivar feature execute checkpoint`.
#[derive(Debug, Args)]
pub struct ExecuteCheckpointArgs {
    pub feature: Option<String>,
    /// The 1-based wave number from `plan.md`.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub wave: u32,
    /// Completed tasks, satisfied exit criteria, and deferred validation failures.
    #[arg(long)]
    pub summary: String,
}

/// Arguments for `ivar feature execute interrupt`.
#[derive(Debug, Args)]
pub struct ExecuteInterruptArgs {
    pub feature: Option<String>,
}

impl From<ExecuteStartArgs> for start::StartInput {
    fn from(args: ExecuteStartArgs) -> Self {
        let ExecuteStartArgs {
            feature,
            plan,
            resume,
            restart,
            mode,
        } = args;
        Self {
            feature: feature.unwrap_or_default(),
            plan,
            resume,
            restart,
            mode,
        }
    }
}

/// `--report-json` and `--outcome` are optional to clap only so that
/// `--print-schema` can stand alone; every other invocation carries both.
/// Converting refuses rather than substituting empty strings, so a clap
/// surface that stops enforcing that says so instead of failing downstream.
impl TryFrom<ExecuteFinishArgs> for finish::FinishInput {
    type Error = Failure;

    fn try_from(args: ExecuteFinishArgs) -> Result<Self, Failure> {
        let ExecuteFinishArgs {
            feature,
            plan,
            report_json,
            outcome,
            print_schema: _,
        } = args;
        let (Some(report_json), Some(outcome)) = (report_json, outcome) else {
            return Err(Failure::blocked(
                "execute.finish_arguments_required",
                "`ivar feature execute finish` needs both `--report-json` and `--outcome`",
            ));
        };
        Ok(Self {
            feature: feature.unwrap_or_default(),
            plan,
            report_json,
            outcome,
        })
    }
}

impl From<ExecuteStatusArgs> for execute_status::StatusInput {
    fn from(args: ExecuteStatusArgs) -> Self {
        let ExecuteStatusArgs {
            feature,
            plan,
            history,
            run,
        } = args;
        Self {
            feature: feature.unwrap_or_default(),
            plan,
            history,
            run,
        }
    }
}

impl From<ExecuteAcceptRevisionArgs> for accept_revision::AcceptRevisionInput {
    fn from(args: ExecuteAcceptRevisionArgs) -> Self {
        let ExecuteAcceptRevisionArgs { feature, plan } = args;
        Self {
            feature: feature.unwrap_or_default(),
            plan,
        }
    }
}
