//! CLI argument definitions for `ivar automation` subcommands.

use clap::{Args, Subcommand, ValueEnum};

use crate::action::automation::decision::{Answer, DecideInput, Form};
use crate::action::automation::scan::ScanInput;

#[derive(Debug, Subcommand)]
pub enum AutomationCommand {
    /// Report the requests users keep repeating, as candidates to automate: groups of similar prompts typed in at least 3 sessions on at least 2 days of the last 90. Read-only apart from draining the capture spool; the samples are memory text, evidence only. Run it from a session that can write `.ivar/` or with `--no-drain` where it cannot.
    Scan(ScanArgs),
    /// Record the answer to a proposal to automate a candidate: `--accepted` (the automation exists as `--artifact`), `--declined` (do not propose it again while it keeps recurring) or `--deferred` (not before `--until`, 30 days by default). The group is the one a scan reported as `group_ref`; its current texts are stored, so the answer survives the group gaining new ones. `--same-as` adds the group to an existing declined or deferred decision instead
    Decide(DecideArgs),
    /// Mark an accepted decision as still wanted: rewrites its `updated_at`
    Renew(DecisionIdArgs),
    /// List the automation decisions. Read-only
    Decisions,
    /// Delete a decision: its group is a candidate again once it recurs enough
    Undo(DecisionIdArgs),
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    /// Keep only the groups with a prompt in a session of this feature or one of its descendants.
    #[arg(long)]
    pub feature: Option<String>,
    /// Do not drain the capture spool first. Nothing is written, so it works when `.ivar/` is not writable; prompts still in the spool are missed.
    #[arg(long)]
    pub no_drain: bool,
}

impl From<ScanArgs> for ScanInput {
    fn from(args: ScanArgs) -> Self {
        let ScanArgs { feature, no_drain } = args;
        Self {
            feature,
            drain: !no_drain,
        }
    }
}

#[derive(Debug, Args)]
#[command(group(clap::ArgGroup::new("answer").required(true).args(["accepted", "declined", "deferred", "same_as"])))]
pub struct DecideArgs {
    /// The `group_ref` of a candidate: the id of one of its request fingerprints.
    pub group_ref: i64,
    /// The automation exists. Needs `--intent`, `--form` and `--artifact`.
    #[arg(long, requires_all = ["intent", "form", "artifact"])]
    pub accepted: bool,
    /// Do not propose this request again while it keeps recurring. Needs `--intent`.
    #[arg(long, requires = "intent")]
    pub declined: bool,
    /// Do not propose this request before `--until`. Needs `--intent`.
    #[arg(long, requires = "intent")]
    pub deferred: bool,
    /// Add this group's texts to an existing declined or deferred decision, which keeps its kind and `until`.
    #[arg(long, value_name = "DECISION_ID", conflicts_with_all = ["intent", "form", "artifact", "until"])]
    pub same_as: Option<i64>,
    /// A short name for what the request is.
    #[arg(long)]
    pub intent: Option<String>,
    /// What the request became.
    #[arg(long, value_enum, conflicts_with_all = ["declined", "deferred"])]
    pub form: Option<FormArg>,
    /// The automation's path, relative to the hall.
    #[arg(long, conflicts_with_all = ["declined", "deferred"])]
    pub artifact: Option<String>,
    /// The last day of the deferral, `YYYY-MM-DD`. Defaults to 30 days from now.
    #[arg(long, conflicts_with_all = ["accepted", "declined"])]
    pub until: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FormArg {
    Script,
    Skill,
    Agent,
    Procedure,
}

impl From<FormArg> for Form {
    fn from(form: FormArg) -> Self {
        match form {
            FormArg::Script => Self::Script,
            FormArg::Skill => Self::Skill,
            FormArg::Agent => Self::Agent,
            FormArg::Procedure => Self::Procedure,
        }
    }
}

impl From<DecideArgs> for DecideInput {
    fn from(args: DecideArgs) -> Self {
        let DecideArgs {
            group_ref,
            accepted,
            declined,
            deferred: _,
            same_as,
            intent,
            form,
            artifact,
            until,
        } = args;
        let intent = intent.unwrap_or_default();
        let answer = match (same_as, accepted, declined, form, artifact) {
            (Some(decision_id), ..) => Answer::SameAs { decision_id },
            (None, true, _, Some(form), Some(artifact)) => Answer::Accepted {
                intent,
                form: form.into(),
                artifact,
            },
            (None, false, true, ..) => Answer::Declined { intent },
            _ => Answer::Deferred { intent, until },
        };
        Self { group_ref, answer }
    }
}

#[derive(Debug, Args)]
pub struct DecisionIdArgs {
    /// The decision's id, as `ivar automation decisions` lists it.
    pub decision_id: i64,
}
