use clap::{Args, Subcommand};

use crate::action::plan::approve as plan_approve;
use crate::action::plan::{create as plan_create, show as plan_show, status as plan_status};

/// The `ivar plan` surface: the SPDD artifacts, committed per feature, and
/// the approval gates that transition a feature through the SPDD lifecycle.
#[derive(Debug, Subcommand)]
pub enum PlanCommand {
    /// Scaffold a feature's SPDD artifacts (requirements, analysis, plan), or
    /// only the ones named. With a subset, writes what is missing and leaves
    /// what is already there untouched.
    Create(PlanCreateArgs),
    /// List which features have plans, and how complete.
    List,
    /// Print one feature's SPDD artifact.
    Show(PlanShowArgs),
    /// Approve one of a feature's SPDD gates: requirements, analysis, plan.
    /// Requires every gate upstream of it to be either approved or never
    /// written — an artifact that exists still has to be approved, even
    /// though an absent one is skipped — and records a fingerprint of the
    /// artifact's content.
    Approve(PlanApproveArgs),
    /// Declare a revision of an approved gate, marking it — and every gate
    /// downstream — as needing revision.
    Invalidate(PlanInvalidateArgs),
    /// Show approval gate status for a plan file. Omits a gate that has no
    /// artifact and was never approved; a gate that was approved and whose
    /// artifact then vanished is still shown, as needs-revision.
    Status(PlanStatusArgs),
}

/// Arguments for `ivar plan create`.
#[derive(Debug, Args)]
pub struct PlanCreateArgs {
    /// The feature to scaffold plans for.
    pub feature: Option<String>,
    /// Which artifacts to scaffold (`requirements`, `analysis`, `plan`);
    /// scaffolds all three when omitted.
    pub artifacts: Vec<crate::action::plan::Artifact>,
}

/// Arguments for `ivar plan show`.
#[derive(Debug, Args)]
#[command(allow_missing_positional = true)]
pub struct PlanShowArgs {
    /// The feature whose artifact to show.
    pub feature: Option<String>,
    /// Which artifact: `requirements`, `analysis`, or `plan`.
    pub artifact: crate::action::plan::show::Artifact,
}

/// Arguments for `ivar plan approve`.
#[derive(Debug, Args)]
#[command(allow_missing_positional = true)]
pub struct PlanApproveArgs {
    /// The feature whose gate to approve.
    pub feature: Option<String>,
    /// The gate: `requirements`, `analysis`, or `plan`.
    pub gate: String,
}

/// Arguments for `ivar plan invalidate`.
#[derive(Debug, Args)]
#[command(allow_missing_positional = true)]
pub struct PlanInvalidateArgs {
    /// The feature whose gate to invalidate.
    pub feature: Option<String>,
    /// The gate: `requirements`, `analysis`, or `plan`.
    pub gate: String,
}

/// Arguments for `ivar plan status`.
#[derive(Debug, Args)]
pub struct PlanStatusArgs {
    /// Path to the plan file (plan.md or similar).
    pub plan_path: String,
}

impl From<PlanCreateArgs> for plan_create::CreateInput {
    fn from(args: PlanCreateArgs) -> Self {
        let PlanCreateArgs { feature, artifacts } = args;
        Self {
            feature: feature.unwrap_or_default(),
            artifacts,
        }
    }
}

impl From<PlanShowArgs> for plan_show::ShowInput {
    fn from(args: PlanShowArgs) -> Self {
        let PlanShowArgs { feature, artifact } = args;
        Self {
            feature: feature.unwrap_or_default(),
            artifact,
        }
    }
}

impl From<PlanApproveArgs> for plan_approve::ApproveInput {
    fn from(args: PlanApproveArgs) -> Self {
        let PlanApproveArgs { feature, gate } = args;
        Self {
            feature: feature.unwrap_or_default(),
            gate,
        }
    }
}

impl From<PlanInvalidateArgs> for plan_approve::InvalidateInput {
    fn from(args: PlanInvalidateArgs) -> Self {
        let PlanInvalidateArgs { feature, gate } = args;
        Self {
            feature: feature.unwrap_or_default(),
            gate,
        }
    }
}

impl From<PlanStatusArgs> for plan_status::StatusInput {
    fn from(args: PlanStatusArgs) -> Self {
        let PlanStatusArgs { plan_path } = args;
        Self { plan_path }
    }
}
