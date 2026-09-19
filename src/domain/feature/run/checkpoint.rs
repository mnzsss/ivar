use std::fmt;

use serde::{Deserialize, Serialize};

use super::coordinator::CoordinatorReport;
use super::evidence::RunDiff;
use super::status::RunStatus;
use crate::domain::name::SessionId;
use crate::domain::provider::Provider;

/// One coordinator that attached to this run.
///
/// The pair is an *ivar* session and the provider it opened. Resume appends a
/// new entry rather than replacing the old one, so a run that began under
/// Claude Code and continued under OpenCode reads as two entries in order —
/// which is the honest claim. Nothing here identifies a provider-native
/// conversation, because ivar does not resume one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoordinatorEntry {
    /// The ivar feature session that coordinated.
    pub session: SessionId,
    /// The provider that session opened.
    pub provider: Provider,
    /// When it attached.
    pub attached_at: String,
}

/// What a checkpoint records about the lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckpointKind {
    /// The run was created.
    Started,
    /// A coordinator re-attached to a non-terminal run.
    Resumed,
    /// The coordinator stopped on a question.
    Blocked,
    /// The approved plan changed under the run.
    Diverged,
    /// A human adopted the new plan revision.
    RevisionAccepted,
    /// The run ended with a reported outcome.
    Terminated,
    /// The run was abandoned — restarted by a human, or imported from a
    /// non-terminal board.
    Interrupted,
    /// The receipt was reconstructed from a legacy execution board.
    LegacyImport,
    /// A coordinator recorded an approved wave.
    Wave,
}

impl fmt::Display for CheckpointKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Started => "started",
            Self::Resumed => "resumed",
            Self::Blocked => "blocked",
            Self::Diverged => "diverged",
            Self::RevisionAccepted => "revision-accepted",
            Self::Terminated => "terminated",
            Self::Interrupted => "interrupted",
            Self::LegacyImport => "legacy-import",
            Self::Wave => "wave",
        };
        f.pad(name)
    }
}

/// One ordered lifecycle decision, with whatever evidence that decision
/// carried.
///
/// Checkpoints are why a blocked run can be finished twice: the first finish
/// appends a blocked checkpoint with its report and diff, a resume appends
/// another, and only the last one supplies the receipt's terminal outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunCheckpoint {
    /// When it happened.
    pub at: String,
    /// What kind of decision it was.
    pub kind: CheckpointKind,
    /// The status the receipt moved to.
    pub status: RunStatus,
    /// The coordinator session that made it. `None` for a legacy import,
    /// which no session performed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionId>,
    /// The provider that session opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    /// The report submitted at this checkpoint, when one was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<CoordinatorReport>,
    /// The filesystem evidence captured at this checkpoint, when it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<RunDiff>,
    /// The plan fingerprint the receipt was pinned to before this checkpoint,
    /// when the checkpoint changed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_fingerprint_from: Option<String>,
    /// The plan fingerprint observed or adopted, when the checkpoint changed
    /// or compared it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_fingerprint_to: Option<String>,
    /// The wave this checkpoint records, when it is a wave checkpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wave: Option<WaveProgress>,
}

/// What a coordinator recorded when a wave was approved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaveProgress {
    /// The 1-based wave number in `plan.md`.
    pub number: u32,
    /// Completed tasks, satisfied exit criteria, and deferred validation failures.
    pub summary: String,
}
