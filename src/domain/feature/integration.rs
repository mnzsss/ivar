//! The pure nested-integration vocabulary: how a child feature integrates
//! into its immediate parent, and what counts as verified.
//!
//! Everything in this module is a value, never an I/O act. It owns:
//!
//! - **Policy**: [`IntegrationVia`] (`pr|local` — the public vocabulary;
//!   `github` is deliberately not accepted anywhere), [`IntegrationStrategy`]
//!   (`squash|merge|rebase`), the per-feature [`IntegrationOverride`], and
//!   [`IntegrationPolicy::resolve`] — the per-field precedence CLI override >
//!   feature override > hall default > embedded default.
//! - **Evidence**: [`VerificationResult`] (one ordered check's outcome),
//!   [`PrCheckResult`] (one required GitHub check's bucket), and
//!   [`VerificationEvidence`] — everything a receipt records about why its
//!   result SHA is trusted, including the fingerprint of the check list that
//!   produced it.
//! - **Receipts**: [`IntegrationReceipt`] — source SHA, immediate-parent
//!   target branch, result SHA, via/strategy, optional PR URL, and the
//!   evidence.
//! - **Classification**: [`FeatureIntegrationState`] and [`classify`] — the
//!   derived state of a feature from the close record (if any) plus live
//!   receipt facts. Nothing here is ever serialized onto `Feature`: the tree
//!   and its health are derived by `action::feature::relations` scanning
//!   records, never stored.
//!
//! Freshness itself is not a fact this module knows — a receipt is stale when
//! its source moved, its check fingerprint drifted, or its result left the
//! parent's history, all of which require live git state. [`classify`] accepts
//! the *answers* ([`ClassificationFacts`]) and derives the state; collecting
//! the facts is the action layer's job.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::domain::name::BranchName;
use crate::error::{Failure, FixAction};

use super::promotion::PromotionOutcome;

/// How a child's changes travel into its immediate parent's branch.
///
/// The public vocabulary is exactly `pr` and `local`. `github` is not accepted
/// as an enum variant or a CLI value — the PR implementation happens to use
/// the `gh` executable, but that is an implementation detail of `via=pr`.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationVia {
    /// A pull request through the forge, merged and observed.
    Pr,
    /// A local merge into the parent's branch.
    #[default]
    Local,
}

impl IntegrationVia {
    /// Parse the CLI spelling — `pr` or `local`. Everything else is refused;
    /// in particular `github` is not a via.
    pub fn parse(value: &str) -> Result<Self, UnknownIntegrationVia> {
        match value {
            "pr" => Ok(Self::Pr),
            "local" => Ok(Self::Local),
            other => Err(UnknownIntegrationVia(other.to_owned())),
        }
    }
}

impl fmt::Display for IntegrationVia {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Pr => "pr",
            Self::Local => "local",
        };
        f.pad(name)
    }
}

/// How the child's commits land on the immediate parent's branch.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationStrategy {
    /// One commit, the parent's, carrying the child's whole change.
    Merge,
    /// A single squashed commit.
    #[default]
    Squash,
    /// The child replayed onto the parent, then the parent fast-forwarded.
    Rebase,
}

impl IntegrationStrategy {
    /// Parse the CLI spelling — `squash`, `merge`, or `rebase`.
    pub fn parse(value: &str) -> Result<Self, UnknownIntegrationStrategy> {
        match value {
            "squash" => Ok(Self::Squash),
            "merge" => Ok(Self::Merge),
            "rebase" => Ok(Self::Rebase),
            other => Err(UnknownIntegrationStrategy(other.to_owned())),
        }
    }
}

impl fmt::Display for IntegrationStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
        };
        f.pad(name)
    }
}

/// The per-feature policy override, persisted on `feature.json` at creation.
///
/// Each field is optional: omitting it leaves that field inheritable, so the
/// effective policy resolves per field (CLI > this > hall > embedded).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationOverride {
    /// The feature's via override, if it declared one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<IntegrationVia>,
    /// The feature's strategy override, if it declared one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<IntegrationStrategy>,
}

/// A fully-resolved integration policy: one via, one strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntegrationPolicy {
    /// How the child's changes travel into the parent.
    pub via: IntegrationVia,
    /// How the child's commits land on the parent.
    pub strategy: IntegrationStrategy,
}

impl IntegrationPolicy {
    /// Resolve the effective policy for one integration, per field:
    /// CLI override > feature override > hall default > embedded default
    /// ([`Self::default`], `local`/`squash`).
    #[must_use]
    pub fn resolve(
        cli: IntegrationOverride,
        feature: IntegrationOverride,
        hall: IntegrationPolicy,
    ) -> Self {
        Self {
            via: cli.via.or(feature.via).unwrap_or(hall.via),
            strategy: cli.strategy.or(feature.strategy).unwrap_or(hall.strategy),
        }
    }
}

impl Default for IntegrationPolicy {
    /// The embedded default: local integration, squashed.
    fn default() -> Self {
        Self {
            via: IntegrationVia::Local,
            strategy: IntegrationStrategy::Squash,
        }
    }
}

/// The outcome of one ordered verification command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationResult {
    /// The command that ran, as declared in the manifest's ordered `checks`.
    pub command: String,
    /// Whether it exited zero.
    pub success: bool,
    /// Its exit code, when the process exited with one (a signal death has
    /// none).
    pub exit_code: Option<i32>,
    /// The most useful sentence it produced — its stderr, or stdout when
    /// stderr is empty, or a description of the exit itself.
    pub diagnostic: String,
}

impl VerificationResult {
    /// A passing result.
    #[must_use]
    pub fn passed(
        command: impl Into<String>,
        exit_code: Option<i32>,
        diagnostic: impl Into<String>,
    ) -> Self {
        Self {
            command: command.into(),
            success: true,
            exit_code,
            diagnostic: diagnostic.into(),
        }
    }

    /// A failing result.
    #[must_use]
    pub fn failed(
        command: impl Into<String>,
        exit_code: Option<i32>,
        diagnostic: impl Into<String>,
    ) -> Self {
        Self {
            command: command.into(),
            success: false,
            exit_code,
            diagnostic: diagnostic.into(),
        }
    }
}

/// One required pull-request check, by bucket.
///
/// `bucket` stays a string because it is whatever the forge reported (`pass`,
/// `fail`, `pending`, …); the only classification this module needs is
/// pass-versus-not, which callers read from the bucket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrCheckResult {
    /// The check's name.
    pub name: String,
    /// The bucket the forge reported: `pass`, `fail`, `pending`, …
    pub bucket: String,
}

impl PrCheckResult {
    /// A passing check.
    #[must_use]
    pub fn passed(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            bucket: "pass".to_owned(),
        }
    }
}

/// Everything a receipt records about why its result SHA is trusted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationEvidence {
    /// A fingerprint of the manifest's ordered checks at verification time.
    /// Receipt freshness compares this against the *current* fingerprint, so
    /// check edits invalidate old receipts.
    pub command_fingerprint: String,
    /// The ordered results of the child's own checks, run before integration.
    pub child: Vec<VerificationResult>,
    /// The ordered results of the immediate parent's checks, run after the
    /// per-repo result was applied (or, for local integration, on the
    /// candidate first).
    pub parent: Vec<VerificationResult>,
    /// The required pull-request checks, when `via=pr`.
    pub pr_checks: Vec<PrCheckResult>,
    /// When the verification ran, as an RFC 3339 timestamp.
    pub verified_at: String,
}

impl VerificationEvidence {
    /// Whether every recorded child and parent check passed. PR-check buckets
    /// are recorded but do not contribute to this answer — the child/parent
    /// results are the ones that gate the receipt; PR checks are orientation.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.child.iter().all(|result| result.success)
            && self.parent.iter().all(|result| result.success)
    }
}

/// The durable record of one repo's integration into its immediate parent.
///
/// Persisted on the child's promotion the moment the result lands — success
/// *and* failure — so a partial multi-repo integration is resumable, never
/// described as atomic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationReceipt {
    /// The child branch's tip at the moment integration ran. Freshness is
    /// judged against this: if the child branch moved, the receipt is stale.
    pub source_sha: String,
    /// The immediate parent's branch — the only thing a child ever targets.
    pub target_branch: BranchName,
    /// The commit the parent's branch now carries the child's change at, or
    /// the commit that *was* applied when a post-parent check failed.
    pub result_sha: String,
    /// How the change travelled.
    pub via: IntegrationVia,
    /// How the commits landed.
    pub strategy: IntegrationStrategy,
    /// The pull request that carried the change, when `via=pr`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    /// The child/parent/PR-check evidence.
    pub verification: VerificationEvidence,
}

/// The derived state of a feature's integration, from the close record plus
/// live receipt facts. Never serialized onto `Feature` — it is recomputed by
/// [`classify`] wherever the tree is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureIntegrationState {
    /// Work is in progress: no close record, and not every promotion carries
    /// a fresh passing receipt.
    Active,
    /// Every promotion carries a fresh passing receipt (with or without the
    /// `integrated` close record that formalizes it).
    Integrated,
    /// A receipt records failed verification evidence.
    Failed,
    /// A receipt exists but is no longer fresh against live state.
    Stale,
    /// Closed with outcome `abandoned` — history, not a blocker.
    Abandoned,
    /// A root closed with outcome `delivered`.
    Delivered,
}

impl fmt::Display for FeatureIntegrationState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Active => "active",
            Self::Integrated => "integrated",
            Self::Failed => "failed",
            Self::Stale => "stale",
            Self::Abandoned => "abandoned",
            Self::Delivered => "delivered",
        };
        f.pad(name)
    }
}

/// The receipt-derived facts the pure classifier needs. Collected by
/// `action::feature::relations` against live git state; this module only
/// classifies what it is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassificationFacts {
    /// Every promoted repo carries a receipt (of any outcome).
    pub fully_receipted: bool,
    /// At least one receipt records failed verification evidence.
    pub any_failed_evidence: bool,
    /// At least one receipt is stale against live state (source moved, check
    /// fingerprint drifted, or its result left the parent's history).
    pub any_stale: bool,
}

impl ClassificationFacts {
    /// No receipts at all — work in progress.
    #[must_use]
    pub fn active() -> Self {
        Self {
            fully_receipted: false,
            any_failed_evidence: false,
            any_stale: false,
        }
    }

    /// Every promotion receipted, all fresh and passing.
    #[must_use]
    pub fn integrated() -> Self {
        Self {
            fully_receipted: true,
            any_failed_evidence: false,
            any_stale: false,
        }
    }

    /// At least one receipt records failed evidence.
    #[must_use]
    pub fn failed() -> Self {
        Self {
            fully_receipted: true,
            any_failed_evidence: true,
            any_stale: false,
        }
    }

    /// At least one receipt is stale, none recorded failed.
    #[must_use]
    pub fn stale() -> Self {
        Self {
            fully_receipted: true,
            any_failed_evidence: false,
            any_stale: true,
        }
    }
}

/// Derive a feature's integration state from its close record (if any) plus
/// the live receipt facts.
///
/// A close record wins: `delivered` and `abandoned` are lifecycle facts a
/// scan cannot re-derive. Without a record, the receipts classify: not fully
/// receipted is `Active`, failed evidence outranks staleness, staleness
/// outranks a clean integrated result.
#[must_use]
pub fn classify(
    outcome: Option<PromotionOutcome>,
    facts: ClassificationFacts,
) -> FeatureIntegrationState {
    match outcome {
        Some(PromotionOutcome::Delivered) => FeatureIntegrationState::Delivered,
        Some(PromotionOutcome::Abandoned) => FeatureIntegrationState::Abandoned,
        Some(PromotionOutcome::Integrated) => FeatureIntegrationState::Integrated,
        None => {
            if !facts.fully_receipted {
                FeatureIntegrationState::Active
            } else if facts.any_failed_evidence {
                FeatureIntegrationState::Failed
            } else if facts.any_stale {
                FeatureIntegrationState::Stale
            } else {
                FeatureIntegrationState::Integrated
            }
        }
    }
}

/// A via name that matched neither [`IntegrationVia`] variant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown integration via `{0}` — expected one of: pr, local")]
pub struct UnknownIntegrationVia(pub String);

impl From<UnknownIntegrationVia> for Failure {
    fn from(error: UnknownIntegrationVia) -> Self {
        Failure::blocked("feature.unknown_integration_via", error.to_string()).fix(FixAction::safe(
            "feature.valid_integration_via",
            "Use one of: pr, local.",
        ))
    }
}

/// A strategy name that matched neither [`IntegrationStrategy`] variant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown integration strategy `{0}` — expected one of: squash, merge, rebase")]
pub struct UnknownIntegrationStrategy(pub String);

impl From<UnknownIntegrationStrategy> for Failure {
    fn from(error: UnknownIntegrationStrategy) -> Self {
        Failure::blocked("feature.unknown_integration_strategy", error.to_string()).fix(
            FixAction::safe(
                "feature.valid_integration_strategy",
                "Use one of: squash, merge, rebase.",
            ),
        )
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/feature/integration.rs"]
mod tests;
