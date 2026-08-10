//! The delivery surface: the `Guard` checks on a feature's approval board and
//! the side-effect-free `DeliveryPreview` / `DeliveryRepo` / `DeliveryAction`
//! summary `feature deliver --preview` produces.
//!
//! Pure data, no I/O — building a preview reads the world, but these values
//! are what the preview prints and what apply gates on.

use serde::{Deserialize, Serialize};

use super::super::name::{BranchName, FeatureName, RepoName};
use super::approval::GateState;

/// One named guard check on a feature's execution board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guard {
    /// The guard's name — what it checks, e.g. `tests_pass`.
    pub name: String,
    /// Whether the check passed.
    pub passed: bool,
}

/// The side-effect-free summary of a feature's pending delivery actions.
///
/// Produced by `ivar feature deliver --preview` and re-produced by apply mode,
/// where [`Self::fingerprint`] is the gate: apply recomputes the fingerprint
/// from the current state and refuses when it differs from the one the preview
/// printed — the human approved *that* state, and anything else has drifted.
///
/// Pure data: building it reads the world, but this value itself is what the
/// preview prints and what apply gates on. `fingerprint` is computed over the
/// rest of this value (see `action/feature/deliver.rs`), so it is never
/// part of its own digest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryPreview {
    /// The feature being delivered.
    pub feature: FeatureName,
    /// The state of the feature's [`Gate::Plan`] — the gate delivery is
    /// conditioned on. Read from the approvals artifact, never stored on the
    /// feature: there is no lifecycle field to fall out of step with it.
    ///
    /// `--preview` reports it and refuses nothing; apply refuses anything but
    /// [`GateState::Approved`]. It is part of the fingerprinted summary, so
    /// crossing the gate after a preview reads as drift, exactly like a new
    /// commit would.
    pub plan_gate: GateState,
    /// One entry per promoted repo, in push order.
    pub repos: Vec<DeliveryRepo>,
    /// Content hash of the preview summary, for apply gating.
    pub fingerprint: String,
}

/// What delivering one promoted repo will do (or did), as far as the preview
/// can know without touching the remote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryRepo {
    /// The repo's name, as declared in `ivar.json`.
    pub repo: RepoName,
    /// The feature branch this repo's worktree is on.
    pub local_branch: BranchName,
    /// The remote the branch is pushed to — the repo's `url` from the
    /// manifest.
    pub remote: String,
    /// The refspec the push uses, `local_branch:refs/heads/local_branch`.
    pub push_refspec: String,
    /// What delivering this repo means — create a new pull request, update an
    /// existing one, or push only (the branch already exists on the remote but
    /// has no PR).
    pub action: DeliveryAction,
    /// The branch this feature's work started from — the repo's default
    /// branch.
    pub base_branch: BranchName,
    /// Repos that must be delivered before this one. `ivar`'s feature model
    /// declares no cross-repo dependencies, so this is empty for every repo;
    /// the ordering machinery that consumes it exists for when it is not.
    pub dependencies: Vec<RepoName>,
    /// Everything that stands between the current state and a clean push:
    /// a dirty worktree, commits that have never been pushed. Informational
    /// in the preview; the fingerprint gate is what apply actually enforces.
    pub blockers: Vec<String>,
    /// The created or updated PR URL, recorded after apply. Present only when
    /// the action was [`DeliveryAction::NewPr`] or
    /// [`DeliveryAction::UpdatePr`] and the PR step succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
}

/// What a delivery action is. The valhalla model distinguishes creating and
/// updating pull requests from a bare push; `ivar` is serverless — no PR
/// surface exists, so only [`Self::PushOnly`] is ever produced. The other two
/// variants exist so the type is the full model and a future PR-capable
/// surface cannot invent a fourth meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryAction {
    /// Open a new pull request for this branch.
    NewPr,
    /// Update an existing pull request for this branch.
    UpdatePr,
    /// Just push the branch to the remote.
    PushOnly,
}
