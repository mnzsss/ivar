use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Failure, FixAction};

/// One task the coordinator's subagents carried out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskResult {
    /// What the task was.
    pub title: String,
    /// How it ended.
    pub status: TaskStatus,
    /// What it produced, in one or two sentences.
    pub result: String,
}

/// How one reported task ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Finished, with its work landed.
    Completed,
    /// Attempted and did not land.
    Failed,
    /// Deliberately not attempted.
    Skipped,
    /// Stopped on a question a human must answer.
    Blocked,
}

/// One verification the coordinator ran.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationCheck {
    /// What was run — a command line, or the name of the check.
    pub command: String,
    /// How it ended.
    pub status: CheckStatus,
    /// What it said, condensed. Never raw output.
    pub summary: String,
}

/// How one verification ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// Ran and passed.
    Passed,
    /// Ran and failed.
    Failed,
    /// Not run.
    Skipped,
}

/// One native subagent, described in provider-neutral terms.
///
/// A *role* and a *status*, never a native child id: the identifier is
/// provider-specific, unstable, and worthless to anyone reading the receipt
/// later, which is exactly the coupling this feature removes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentRole {
    /// What the subagent was asked to be — "reviewer", "test-writer".
    pub role: String,
    /// How its work ended.
    pub status: TaskStatus,
}

/// The coordinator's structured account of a run, supplied to `finish`.
///
/// Closed by `deny_unknown_fields`, which is load-bearing rather than tidy:
/// it is what stops a provider envelope, a transcript excerpt, or a native
/// session id from being smuggled in as an extra key and quietly becoming
/// ivar domain state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CoordinatorReport {
    /// What happened, in prose. Required and non-blank.
    pub summary: String,
    /// What was done. At least one entry.
    pub tasks: Vec<TaskResult>,
    /// What was checked. At least one entry — a run that verified nothing has
    /// not finished, it has stopped.
    pub verification: Vec<VerificationCheck>,
    /// The subagents that ran, if the coordinator chose to describe them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentRole>,
    /// Where the run departed from the approved plan.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deviations: Vec<String>,
    /// What stopped the run, when the outcome is blocked or failed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
    /// Isolatable work deliberately left for a child feature.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub follow_ups: Vec<String>,
}

impl CoordinatorReport {
    /// Refuse a report that cannot serve as evidence.
    ///
    /// The three rules are the ones a coordinator gets wrong when it is
    /// hurrying: an empty summary, no tasks, and no verification. Each is a
    /// separate refusal because "your report is invalid" is not an actionable
    /// sentence.
    pub fn validate(&self) -> Result<(), Failure> {
        if self.summary.trim().is_empty() {
            return Err(Failure::blocked(
                "execute.report_summary_blank",
                "the coordinator report has no summary",
            )
            .fix(FixAction::safe(
                "execute.report_summary",
                "Set `summary` to a sentence describing what the run did.",
            )));
        }
        if self.tasks.is_empty() {
            return Err(Failure::blocked(
                "execute.report_no_tasks",
                "the coordinator report lists no tasks",
            )
            .fix(FixAction::safe(
                "execute.report_tasks",
                "Add at least one entry to `tasks` with a title, status, and result.",
            )));
        }
        if self.verification.is_empty() {
            return Err(Failure::blocked(
                "execute.report_no_verification",
                "the coordinator report lists no verification",
            )
            .fix(FixAction::safe(
                "execute.report_verification",
                "Add at least one entry to `verification` — the command run, its status, \
                 and what it said.",
            )));
        }
        Ok(())
    }
}
