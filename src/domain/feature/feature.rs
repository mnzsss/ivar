//! The promotion record: `Feature`, per-repo `Promotion` and `WorktreeState`,
//! the `PromotionOutcome` a closed feature records, and the `FeatureBoard`
//! approval record.
//!
//! Pure data, no I/O — reading and writing these values is `store::feature`'s
//! job. See the module doc in `mod.rs` for the feature model as a whole.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Failure, FixAction};

use super::super::name::{BranchName, FeatureName, RepoName};
use super::delivery::Guard;
use super::integration::{IntegrationOverride, IntegrationReceipt};

/// The schema version of `feature.json`, stamped by `store::feature`.
const CURRENT_VERSION: u32 = 3;

/// One feature: a branch name and the set of repos promoted onto it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    version: u32,
    /// The feature's name — also the directory name under `.ivar/features/`.
    pub name: FeatureName,
    /// The branch name every promoted repo's worktree is checked out on.
    pub branch: BranchName,
    /// Repo name → how far that repo's promotion got.
    pub promotions: BTreeMap<RepoName, Promotion>,
    /// The branch new promotions should be based on, if the feature declared
    /// one explicitly. `None` means "use the repo's `default_branch`" — see
    /// [`super::effective_base`]. `#[serde(default)]` so a v1 `feature.json`,
    /// which predates this field, still deserialises.
    #[serde(default)]
    pub base: Option<BranchName>,
    /// The feature's parent, if it is a subfeature. Children are **derived**
    /// by scanning this field — no feature stores a child list. A parent-less
    /// feature is a root. `#[serde(default)]` so a v2 `feature.json`, which
    /// predates this field, still deserialises.
    #[serde(default)]
    pub parent: Option<FeatureName>,
    /// The feature's own integration-policy override, if it declared one at
    /// creation. Omitting a field leaves it inheritable (hall default then
    /// embedded default). `#[serde(default)]` so a v2 `feature.json` still
    /// deserialises.
    #[serde(default)]
    pub integration: IntegrationOverride,
}

impl Feature {
    /// Build a new, empty feature: no repo promoted yet.
    ///
    /// # Errors
    ///
    /// Nothing can fail here — the name and branch arrive already validated.
    #[must_use]
    pub fn new(name: FeatureName, branch: BranchName) -> Self {
        Self {
            version: CURRENT_VERSION,
            name,
            branch,
            promotions: BTreeMap::new(),
            base: None,
            parent: None,
            integration: IntegrationOverride::default(),
        }
    }

    /// Record `repo` as promoted into this feature, at
    /// [`WorktreeState::Pending`]. Overwrites any previous record.
    pub fn promote(&mut self, repo: RepoName) {
        self.promotions.insert(
            repo,
            Promotion {
                worktree: WorktreeState::Pending,
                base: None,
                integration_receipt: None,
            },
        );
    }

    /// Whether any promotion carries an integration receipt — the first
    /// receipt of any kind freezes the feature's relationship/base/policy and
    /// promotion membership.
    #[must_use]
    pub fn has_any_receipt(&self) -> bool {
        self.promotions
            .values()
            .any(|promotion| promotion.integration_receipt.is_some())
    }

    /// Whether `repo`'s promotion carries a receipt with recorded *passing*
    /// evidence. Based on the recorded evidence, not current freshness: a
    /// source/check/history drift that makes the receipt stale never unlocks
    /// an already-successful promotion.
    #[must_use]
    pub fn promotion_has_successful_receipt(&self, repo: &RepoName) -> bool {
        self.promotions
            .get(repo)
            .and_then(|promotion| promotion.integration_receipt.as_ref())
            .is_some_and(|receipt| receipt.verification.passed())
    }

    /// Whether every promoted repo carries a receipt with passing evidence.
    /// The fully-integrated bar; an empty promotion map is deliberately not
    /// "fully integrated".
    #[must_use]
    pub fn all_promotions_have_passing_receipts(&self) -> bool {
        !self.promotions.is_empty()
            && self.promotions.values().all(|promotion| {
                promotion
                    .integration_receipt
                    .as_ref()
                    .is_some_and(|receipt| receipt.verification.passed())
            })
    }

    /// Advance `repo`'s promotion to `state`, recording `Ready`/`Failed`
    /// once the worktree exists. A repo that was never promoted is ignored —
    /// `demote` is the only path out.
    pub fn set_worktree_state(&mut self, repo: &RepoName, state: WorktreeState) {
        if let Some(promotion) = self.promotions.get_mut(repo) {
            promotion.worktree = state;
        }
    }

    /// Remove `repo` from this feature. Returns whether it was promoted.
    pub fn demote(&mut self, repo: &RepoName) -> bool {
        self.promotions.remove(repo).is_some()
    }

    /// Whether `repo` has been promoted into this feature.
    #[must_use]
    pub fn is_promoted(&self, repo: &RepoName) -> bool {
        self.promotions.contains_key(repo)
    }

    /// The worktree state of `repo`, or `None` if it is not promoted.
    #[must_use]
    pub fn worktree_state(&self, repo: &RepoName) -> Option<WorktreeState> {
        self.promotions.get(repo).map(|p| p.worktree)
    }

    /// How many promotions have a worktree in `state`. The one place the
    /// count lives, so `feature list` and the session TUI agree.
    #[must_use]
    pub fn count_worktrees(&self, state: WorktreeState) -> usize {
        self.promotions
            .values()
            .filter(|promotion| promotion.worktree == state)
            .count()
    }

    /// The schema version — always [`CURRENT_VERSION`] for a value built
    /// through [`Self::new`] or read by `store::feature`.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }
}

/// One repo's promotion into a feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Promotion {
    /// How far the worktree materialisation got.
    pub worktree: WorktreeState,
    /// The base branch this promotion's worktree was cut from, if it
    /// differs from what [`super::effective_base`] would compute today.
    /// `None` means "computed from the feature/repo default, nothing
    /// promotion-specific recorded". `#[serde(default)]` so a v1
    /// `feature.json`, which predates this field, still deserialises.
    #[serde(default)]
    pub base: Option<BranchName>,
    /// The durable receipt of this repo's integration into the feature's
    /// immediate parent, once `ivar feature integrate` has applied it — on
    /// success *and* on a post-parent failure, so partial multi-repo
    /// integration is resumable. `None` until integration reaches this repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration_receipt: Option<IntegrationReceipt>,
}

/// The state of a feature's worktree for a promoted repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeState {
    /// Recorded in the promotion but not yet materialised on disk.
    Pending,
    /// Worktree created and setup script has run.
    Ready,
    /// Setup script failed; the next sync will retry it.
    Failed,
}

/// The outcome of a closed feature, recorded in `plan.md`'s frontmatter by
/// `ivar feature close` and read back to make closing idempotent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionOutcome {
    /// The feature's work shipped.
    Delivered,
    /// The feature's work landed in its immediate parent, verified.
    Integrated,
    /// The feature was closed without shipping.
    Abandoned,
}

impl PromotionOutcome {
    /// Parse the CLI spelling of an outcome — `delivered`, `integrated`, or
    /// `abandoned`. [`fmt::Display`] emits the same names.
    pub fn parse(value: &str) -> Result<Self, UnknownOutcome> {
        match value {
            "delivered" => Ok(PromotionOutcome::Delivered),
            "integrated" => Ok(PromotionOutcome::Integrated),
            "abandoned" => Ok(PromotionOutcome::Abandoned),
            other => Err(UnknownOutcome(other.to_owned())),
        }
    }
}

impl fmt::Display for PromotionOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            PromotionOutcome::Delivered => "delivered",
            PromotionOutcome::Integrated => "integrated",
            PromotionOutcome::Abandoned => "abandoned",
        };
        f.pad(name)
    }
}

/// An outcome name that matched neither [`PromotionOutcome`] variant. The CLI
/// passes the raw string through to the action, which parses it here — `cli`
/// cannot import `domain`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown outcome `{0}` — expected one of: delivered, integrated, abandoned")]
pub struct UnknownOutcome(pub String);

impl From<UnknownOutcome> for Failure {
    fn from(error: UnknownOutcome) -> Self {
        Failure::blocked("feature.unknown_outcome", error.to_string()).fix(FixAction::safe(
            "feature.valid_outcome",
            "Use one of: delivered, integrated, abandoned.",
        ))
    }
}

/// The execution board for a feature: whether it is approved to run, and
/// which guard checks stand between that and execution.
///
/// Created empty on `feature create`; `feature execute` (a later slice) is
/// what fills the guards in and flips `approved`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureBoard {
    /// Whether the feature has been approved for execution.
    pub approved: bool,
    /// The guard checks, each named, each with whether it passed.
    pub guards: Vec<Guard>,
}

impl FeatureBoard {
    /// A fresh, unapproved board with no guards recorded.
    #[must_use]
    pub fn new() -> Self {
        Self {
            approved: false,
            guards: Vec::new(),
        }
    }
}

impl Default for FeatureBoard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/feature/feature.rs"]
mod tests;
