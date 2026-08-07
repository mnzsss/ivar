//! Types for features and how repos are promoted into them.
//!
//! A **Feature** is one branch across the repos it has **Promoted**. The
//! feature's own `branch` name is shared by every promoted repo's worktree
//! path (`.ivar/repos/<repo>/<branch>/`); what differs per repo is whether it
//! is promoted at all, and how far the promotion got.
//!
//! # What lives here
//!
//! `Feature` — the promotion record: which repos, and each one's
//! [`WorktreeState`]. `FeatureBoard` — the execution board (approval +
//! guards), which slice 4 creates and later slices fill in. All pure, no I/O —
//! reading and writing these values is `store::feature`'s job.
//!
//! # What a valid promotion is
//!
//! - A repo is either promoted or not; there is no partial record.
//! - A promoted repo starts at [`WorktreeState::Pending`] (recorded, not yet
//!   materialised) and moves to `Ready` once its worktree exists and its
//!   setup script ran — or `Failed` when the setup script failed and the next
//!   sync must retry.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::name::{BranchName, FeatureName, RepoName};

/// The schema version of `feature.json`, stamped by `store::feature`.
const CURRENT_VERSION: u32 = 1;

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
        }
    }

    /// Record `repo` as promoted into this feature, at
    /// [`WorktreeState::Pending`]. Overwrites any previous record.
    pub fn promote(&mut self, repo: RepoName) {
        self.promotions.insert(
            repo,
            Promotion {
                worktree: WorktreeState::Pending,
            },
        );
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

/// One named guard check on a feature's execution board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guard {
    /// The guard's name — what it checks, e.g. `tests_pass`.
    pub name: String,
    /// Whether the check passed.
    pub passed: bool,
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;

    fn feature() -> Feature {
        Feature::new(
            FeatureName::new("checkout").unwrap(),
            BranchName::new("feat/checkout").unwrap(),
        )
    }

    #[test]
    fn a_new_feature_has_no_promotions_and_is_unapproved() {
        let feature = feature();
        assert!(feature.promotions.is_empty());
        assert!(!FeatureBoard::new().approved);
    }

    #[test]
    fn promote_adds_a_pending_record_and_is_promoted_answers() {
        let mut feature = feature();
        let repo = RepoName::new("api").unwrap();

        feature.promote(repo.clone());

        assert!(feature.is_promoted(&repo));
        assert_eq!(feature.worktree_state(&repo), Some(WorktreeState::Pending));
    }

    #[test]
    fn set_worktree_state_advances_only_a_promoted_repo() {
        let mut feature = feature();
        let repo = RepoName::new("api").unwrap();
        feature.promote(repo.clone());
        let stranger = RepoName::new("web").unwrap();

        feature.set_worktree_state(&repo, WorktreeState::Ready);
        feature.set_worktree_state(&stranger, WorktreeState::Ready);

        assert_eq!(feature.worktree_state(&repo), Some(WorktreeState::Ready));
        assert_eq!(feature.worktree_state(&stranger), None);
    }

    #[test]
    fn demote_removes_the_record_and_reports_whether_it_was_there() {
        let mut feature = feature();
        let repo = RepoName::new("api").unwrap();
        feature.promote(repo.clone());

        assert!(feature.demote(&repo));
        assert!(!feature.is_promoted(&repo));
        assert!(!feature.demote(&repo));
    }

    #[test]
    fn feature_round_trips_through_serde_without_unknown_fields() {
        let mut feature = feature();
        feature.promote(RepoName::new("api").unwrap());
        let rendered = serde_json::to_string(&feature).unwrap();

        let parsed: Feature = serde_json::from_str(&rendered).unwrap();

        assert_eq!(parsed, feature);
        assert_eq!(parsed.version(), 1);
    }

    #[test]
    fn an_unknown_field_in_feature_json_is_refused() {
        let raw = r#"{"version":1,"name":"checkout","branch":"feat/checkout","promotions":{},"bogus":true}"#;
        assert!(serde_json::from_str::<Feature>(raw).is_err());
    }
}
