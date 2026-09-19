use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Failure, FixAction};

/// Where a run is in its lifecycle.
///
/// Three non-terminal states hold the feature's single-run lock, and three
/// terminal states release it. That split is the whole state machine: it is
/// what "may another coordinator start?" reduces to, and why recovery
/// (`blocked`, `diverged`) is deliberately not spelled as a kind of failure.
///
/// ```text
/// start ──────────────────→ active
/// active ──finish blocked─→ blocked   ──start --resume──→ active
/// active ──plan changed───→ diverged  ──accept-revision─→ blocked
/// any non-terminal ──start --restart─→ interrupted  (terminal)
/// active ──finish succeeded────────→ succeeded    (terminal)
/// active ──finish failed───────────→ failed       (terminal)
/// legacy non-terminal board ───────→ interrupted  (terminal)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// A coordinator is attached and work is in flight.
    Active,
    /// The coordinator stopped and asked for a human decision. Resumable;
    /// still holds the lock, because the work is not finished.
    Blocked,
    /// The approved plan changed under a run in flight. Resumable only after
    /// an explicit `accept-revision`; still holds the lock.
    Diverged,
    /// The coordinator reported success and the evidence was recorded.
    Succeeded,
    /// The coordinator reported failure and the evidence was recorded.
    Failed,
    /// The run stopped without a reported outcome — restarted by a human, or
    /// imported from a non-terminal legacy board.
    Interrupted,
}

impl RunStatus {
    /// Whether the run is over. A terminal receipt keeps every byte of its
    /// evidence and releases the feature's single-run lock.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Interrupted)
    }

    /// Whether this status holds the feature's single-run lock — the exact
    /// negation of [`Self::is_terminal`], named for the question callers
    /// actually ask so no call site has to re-derive it.
    #[must_use]
    pub const fn holds_lock(self) -> bool {
        !self.is_terminal()
    }
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Active => "active",
            Self::Blocked => "blocked",
            Self::Diverged => "diverged",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        };
        f.pad(name)
    }
}

/// Where a receipt came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunProvenance {
    /// Created by `feature execute start` under the provider-native
    /// lifecycle.
    Native,
    /// Reconstructed from an execution board written before this lifecycle
    /// existed. Carries [`RunReceipt::legacy`] evidence and is always
    /// terminal.
    LegacyImport,
}

impl fmt::Display for RunProvenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Native => "native",
            Self::LegacyImport => "legacy-import",
        };
        f.pad(name)
    }
}

/// The outcome a coordinator asks `finish` to record.
///
/// Distinct from [`RunStatus`] on purpose: this is the coordinator's *claim*,
/// and finish may refuse it — a plan that moved under the run lands on
/// [`RunStatus::Diverged`] no matter which outcome was submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    /// Every operation landed and verification passed.
    Succeeded,
    /// The run cannot land its operations; a human must decide what next.
    Failed,
    /// Work stopped on a question. Recoverable — the receipt stays resumable.
    Blocked,
}

impl RunOutcome {
    /// The status this outcome produces when finish accepts it.
    #[must_use]
    pub const fn status(self) -> RunStatus {
        match self {
            Self::Succeeded => RunStatus::Succeeded,
            Self::Failed => RunStatus::Failed,
            Self::Blocked => RunStatus::Blocked,
        }
    }

    /// Parse the CLI spelling.
    pub fn parse(value: &str) -> Result<Self, UnknownRunOutcome> {
        match value {
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "blocked" => Ok(Self::Blocked),
            other => Err(UnknownRunOutcome(other.to_owned())),
        }
    }
}

impl fmt::Display for RunOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
        };
        f.pad(name)
    }
}

/// An `--outcome` value that matched no run outcome.
///
/// Named for the run rather than for the flag because `domain::feature`
/// already exports a promotion `UnknownOutcome`, and one flat namespace
/// cannot hold two.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown outcome `{0}` — expected one of: succeeded, failed, blocked")]
pub struct UnknownRunOutcome(pub String);

impl From<UnknownRunOutcome> for Failure {
    fn from(error: UnknownRunOutcome) -> Self {
        Failure::blocked("execute.unknown_outcome", error.to_string()).fix(FixAction::safe(
            "execute.valid_outcome",
            "Use one of: succeeded, failed, blocked.",
        ))
    }
}
