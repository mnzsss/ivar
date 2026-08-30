//! The delivery surface: the `Guard` checks on a feature's approvals,
//! the side-effect-free `DeliveryPreview` / `DeliveryRepo` / `DeliveryAction`
//! summary `feature deliver --preview` produces, and the verdict on whether a
//! repo's base still supports delivering onto it.
//!
//! Pure data and pure classification, no I/O — building a preview reads the
//! world, and so does gathering the facts [`DeliveryRepo::check_base`]
//! classifies, but neither this module nor that method ever performs a read
//! itself.

use serde::{Deserialize, Serialize};

use super::super::name::{BranchName, FeatureName, RepoName};
use super::approval::GateState;
use super::integration::FeatureIntegrationState;
use crate::error::{Failure, FixAction};

/// One named guard check on a feature's approval state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guard {
    /// The guard's name — what it checks, e.g. `tests_pass`.
    pub name: String,
    /// Whether the check passed.
    pub passed: bool,
}

/// What mode delivery runs in: pushing feature branches or landing onto default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    /// Today's behaviour: push feature branches to the remote.
    #[default]
    Push,
    /// Fast-forward merge feature branches into default branches locally, then push.
    Land,
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
    /// What mode this delivery preview was generated for.
    #[serde(default)]
    pub mode: DeliveryMode,
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
    /// The descendants that block this root's delivery — active, failed,
    /// stale, or unintegrated, at any depth. Empty for a healthy root; a
    /// child's delivery is refused outright, never reported here. Part of the
    /// fingerprinted summary, so a descendant crossing a gate after the
    /// preview reads as drift.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tree_blockers: Vec<DeliveryTreeBlocker>,
    /// Content hash of the preview summary, for apply gating.
    pub fingerprint: String,
}

/// One descendant that blocks a root's delivery.
///
/// Abandoned descendants do not block and are not listed; a descendant
/// *beneath* an abandoned node still is, and still blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryTreeBlocker {
    /// The blocking descendant.
    pub feature: FeatureName,
    /// Its depth in the tree.
    pub depth: usize,
    /// Its derived integration state.
    pub state: FeatureIntegrationState,
    /// Why it blocks — one sentence.
    pub reason: String,
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
    /// The branch this feature's work started from — this repo's effective
    /// base, per [`super::effective_base`] and what `promote` recorded.
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
    /// The default branch of the repository, populated in land mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<BranchName>,
    /// Whether the default branch can fast-forward to the feature branch tip,
    /// populated in land mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ff_possible: Option<bool>,
}

/// What a delivery action is across a feature's promoted repositories.
///
/// `ivar` opens pull requests through `gh` when a repo's delivery action calls
/// for one, pushes directly when only a push is requested, and lands locally
/// onto default branches when requested. These variants form the complete
/// model; adding a new variant is a deliberate model change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryAction {
    /// Open a new pull request for this branch.
    NewPr,
    /// Update an existing pull request for this branch.
    UpdatePr,
    /// Just push the branch to the remote.
    PushOnly,
    /// Merge this repo's feature branch into its default branch, fast-forward
    /// only, then push the default.
    LandOnDefault,
}

/// Whether a repo's declared base still supports delivering onto it.
///
/// Computed from facts the caller has already gathered from git — this type
/// never reaches for git itself. [`classify_base`] is the pure function that
/// builds one; [`DeliveryRepo::check_base`] is the boundary `action` calls
/// through, so a caller outside this module never needs to name the variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaseVerdict {
    /// The base is present on the remote, and this repo's branch is still
    /// built on it. Delivery proceeds.
    Ok,
    /// The base is gone from the remote, and locally it looks merged into
    /// the repo's default branch — it was delivered, and its own PR's merge
    /// deleted it.
    BaseMergedAndDeleted,
    /// The base is gone from the remote, and nothing confirms it was ever
    /// merged — it looks like it was never delivered.
    BaseNeverDelivered,
    /// The remote did not answer whether the base exists. The question is
    /// unanswered, never "absent" — those are different facts and call for
    /// different fixes.
    BaseUnconfirmed,
    /// The base exists, but this repo's branch is no longer built on its
    /// current tip — the base moved on without a rebase.
    BaseMoved,
}

/// Classify a base from facts the caller already gathered.
///
/// `remote_tip` is `remote_branch_tip(base)`'s result, with the network error
/// collapsed to `Err(())` — the caller knows *why* the remote did not
/// answer, and this classification only needs *that* it did not.
///
/// `secondary` is one `is_ancestor` call, whichever `remote_tip` calls for:
/// against the repo's default branch when the base is absent (was it merged
/// before its ref was deleted?), or against this repo's own branch when the
/// base is present (does the branch still build on it?). Its error case
/// (a local ref genuinely missing) reads the same as "no" — refusing is
/// always the safe default when the question cannot be answered.
fn classify_base(
    remote_tip: Result<Option<String>, ()>,
    secondary: Result<bool, ()>,
) -> BaseVerdict {
    match remote_tip {
        Err(()) => BaseVerdict::BaseUnconfirmed,
        Ok(None) => match secondary {
            Ok(true) => BaseVerdict::BaseMergedAndDeleted,
            _ => BaseVerdict::BaseNeverDelivered,
        },
        Ok(Some(_)) => match secondary {
            Ok(true) => BaseVerdict::Ok,
            _ => BaseVerdict::BaseMoved,
        },
    }
}

impl BaseVerdict {
    /// The refusal this verdict delivers, or `None` when delivery may
    /// proceed. `repo` and `base` name what refused; `default_branch` only
    /// matters for the merged-and-deleted fix hint.
    fn into_failure(
        self,
        repo: &RepoName,
        base: &BranchName,
        default_branch: &BranchName,
    ) -> Option<Failure> {
        let (code, what, fix_code, fix_what) = match self {
            BaseVerdict::Ok => return None,
            BaseVerdict::BaseMergedAndDeleted => (
                "feature.base_merged_and_deleted",
                format!(
                    "`{repo}`'s base `{base}` is gone from the remote, and looks merged into `{default_branch}`"
                ),
                "feature.rebase_onto_default",
                format!(
                    "Run `ivar feature rebase <feature> --onto {default_branch}` — `{base}` shipped, so this feature's base collapses onto `{default_branch}`."
                ),
            ),
            BaseVerdict::BaseNeverDelivered => (
                "feature.base_never_delivered",
                format!(
                    "`{repo}`'s base `{base}` is gone from the remote, and nothing confirms it was ever delivered"
                ),
                "feature.deliver_parent_first",
                format!("Deliver the feature that owns `{base}` first, then deliver this one."),
            ),
            BaseVerdict::BaseUnconfirmed => (
                "feature.base_unconfirmed",
                format!(
                    "`{repo}`'s base `{base}` could not be confirmed on the remote — the remote did not answer"
                ),
                "feature.retry_deliver",
                "The remote did not answer; retry the delivery.".to_owned(),
            ),
            BaseVerdict::BaseMoved => (
                "feature.base_moved",
                format!("`{repo}`'s branch is no longer built on `{base}`'s current tip"),
                "feature.rebase_feature",
                "Run `ivar feature rebase <feature>` to bring the branch back onto its base."
                    .to_owned(),
            ),
        };
        Some(Failure::blocked(code, what).fix(FixAction::unsafe_(fix_code, fix_what)))
    }
}

impl DeliveryRepo {
    /// Refuse delivering this repo when its base no longer supports it, or
    /// `None` when it does.
    ///
    /// `remote_tip` and `secondary` are facts the caller already gathered —
    /// see [`classify_base`] for what each means and which one to compute.
    /// `default_branch` is the repo's own default branch, from `ivar.json`,
    /// used only to word the merged-and-deleted fix hint.
    ///
    /// This refusal is per repo and blocks apply for that repo alone — it is
    /// not a [`Self::blockers`] entry, which is informational only.
    #[must_use]
    pub fn check_base(
        &self,
        remote_tip: Result<Option<String>, ()>,
        secondary: Result<bool, ()>,
        default_branch: &BranchName,
    ) -> Option<Failure> {
        classify_base(remote_tip, secondary).into_failure(
            &self.repo,
            &self.base_branch,
            default_branch,
        )
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/feature/delivery.rs"]
mod tests;
