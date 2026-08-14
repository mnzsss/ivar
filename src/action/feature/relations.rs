//! The child-derived tree projection: how features relate, how healthy they
//! are, and what blocks what.
//!
//! Nothing here is persisted. The tree is derived by scanning every
//! `feature.json`'s single `parent` field; parent existence and acyclicity are
//! validated whenever the tree is read, and a missing parent or a cycle is a
//! hard, non-mutating refusal. Receipt freshness is derived live against git
//! and the current manifest checks — a receipt is stale when its source moved,
//! its check fingerprint drifted, or its result left the immediate parent's
//! history, and failed when its recorded evidence failed.
//!
//! The immediate-parent direction is the whole design: a child integrates into
//! its immediate parent's branch, never an ancestor, never a default branch.
//! `target_branch` on every receipt is that branch.

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;

use crate::domain::feature::{
    ClassificationFacts, Feature, FeatureIntegrationState, IntegrationReceipt, classify,
};
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction};
use crate::git::Git;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::lifecycle::read_close;
use super::verification;

/// One feature's position and health in the derived tree, in deterministic
/// pre-order. Used by recursive status and by blocker reporting; the flat
/// shape (with `depth`) is what lets a script read the tree without a
/// recursive JSON schema parser.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TreeEntry {
    /// The feature.
    pub feature: FeatureName,
    /// Its parent, if it is a child.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<FeatureName>,
    /// Depth in the tree — 0 for the queried root.
    pub depth: usize,
    /// The derived integration state.
    pub state: FeatureIntegrationState,
    /// The promoted repos, in name order.
    pub repos: Vec<RepoName>,
    /// The names of descendants that block this feature, each rendered as
    /// `name (state)`.
    pub blockers: Vec<String>,
}

/// How a receipt measures against live state. The evidence-vs-freshness split
/// matters: failed recorded evidence is `Failed`; everything a live check
/// disproves is `Stale`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReceiptFreshness {
    /// Source tip, checks, and result history all still match.
    Fresh,
    /// The recorded evidence itself failed.
    Failed,
    /// Live state no longer matches: source moved or missing, checks changed,
    /// or the result left the parent's history.
    Stale { reason: String },
}

/// Read one feature's record, or a hard `feature.not_found`.
pub(crate) fn read_feature(layout: &Layout, name: &FeatureName) -> Result<Feature, Failure> {
    Feature::read(layout, name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
        .expected("an existing feature")
        .actual(format!("`{name}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.create_first",
            format!("Create it first with `ivar feature create {name}`."),
        ))
    })
}

/// Read every feature in the hall, sorted by name, validating the derived
/// tree: every parent reference resolves, and no parent chain cycles. A
/// missing parent or a cycle is a hard, non-mutating refusal — the tree is
/// read as a whole or not at all.
pub(crate) fn read_all(layout: &Layout) -> Result<Vec<Feature>, Failure> {
    let mut features = Vec::new();
    let features_dir = layout.features_dir();
    if fs::is_dir(&features_dir)? {
        for entry in fs::read_dir(&features_dir)? {
            let Some(name) = entry.file_name() else {
                continue;
            };
            let Ok(feature_name) = FeatureName::new(name.to_owned()) else {
                continue;
            };
            if let Some(feature) = Feature::read(layout, &feature_name)? {
                features.push(feature);
            }
        }
    }
    features.sort_by(|a, b| a.name.cmp(&b.name));
    validate_tree(&features)?;
    Ok(features)
}

/// The immediate parent of `feature`, or `None` for a root. A parent name that
/// does not resolve is a hard refusal — by the time this is called the tree
/// should have been validated, but a feature can be deleted between a tree
/// read and this call.
pub(crate) fn parent(layout: &Layout, feature: &Feature) -> Result<Option<Feature>, Failure> {
    let Some(parent_name) = &feature.parent else {
        return Ok(None);
    };
    Feature::read(layout, parent_name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.parent_missing",
            format!(
                "feature `{}` names parent `{parent_name}`, which does not exist",
                feature.name
            ),
        )
        .expected("every parent reference to resolve to an existing feature")
        .actual(format!("`{parent_name}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.fix_parent_reference",
            format!(
                "Repair `{}`'s `parent` field, or create the missing `{parent_name}` feature.",
                feature.name
            ),
        ))
    })
    .map(Some)
}

/// Every direct/recursive descendant of `name`, in deterministic pre-order
/// (a feature, then its children sorted by name, recursively). The tree is
/// validated by the read.
pub(crate) fn descendants(layout: &Layout, name: &FeatureName) -> Result<Vec<Feature>, Failure> {
    let all = read_all(layout)?;
    Ok(descendants_from(&all, name)
        .into_iter()
        .map(|(_, feature)| feature.clone())
        .collect())
}

/// The subtree rooted at `root` (inclusive, depth 0), in pre-order, with each
/// entry's derived state, repos, and blockers.
pub(crate) fn subtree_status(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    root: &FeatureName,
) -> Result<Vec<TreeEntry>, Failure> {
    let all = read_all(layout)?;
    let map = feature_map(&all);
    let root_feature = map.get(root).copied().ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{root}` does not exist"),
        )
        .expected("an existing feature")
        .actual(format!("`{root}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.create_first",
            format!("Create it first with `ivar feature create {root}`."),
        ))
    })?;

    let mut entries = Vec::new();
    walk(git, layout, manifest, &map, root_feature, 0, &mut entries)?;
    Ok(entries)
}

/// Every descendant of `feature` whose derived state blocks integration —
/// active, failed, or stale. Abandoned descendants do not block, but a
/// descendant *beneath* an abandoned node still does; integrated fresh
/// verified descendants do not block.
pub(crate) fn blocking_descendants(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    feature: &Feature,
) -> Result<Vec<TreeEntry>, Failure> {
    let all = read_all(layout)?;
    let map = feature_map(&all);
    let mut blockers = Vec::new();
    for (depth, descendant) in descendants_from(&all, &feature.name) {
        let parent_feature = descendant
            .parent
            .as_ref()
            .and_then(|name| map.get(name))
            .copied();
        let state = state_of(git, layout, manifest, descendant, parent_feature)?;
        if blocks(&state) {
            blockers.push(TreeEntry {
                feature: descendant.name.clone(),
                parent: descendant.parent.clone(),
                depth,
                state,
                repos: descendant.promotions.keys().cloned().collect(),
                blockers: Vec::new(),
            });
        }
    }
    Ok(blockers)
}

/// How one receipt measures against live git and manifest state. See
/// [`ReceiptFreshness`]. A missing source revision is stale, never
/// "not integrated".
pub(crate) fn receipt_freshness(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    receipt: &IntegrationReceipt,
) -> Result<ReceiptFreshness, Failure> {
    // Recorded evidence is the first fact: failed evidence is failed, and no
    // live check can un-fail it.
    if !receipt.verification.passed() {
        return Ok(ReceiptFreshness::Failed);
    }

    let bare = layout.repo_bare(repo);

    // Source equality: the child branch's tip must still be the recorded
    // source. A missing or recreated branch is stale — the receipt describes
    // a branch that is no longer there.
    let source_tip = match git.revision_commit(&bare, child.branch.as_str()) {
        Ok(tip) => tip,
        Err(error) => {
            return Ok(ReceiptFreshness::Stale {
                reason: format!(
                    "child branch `{}` is missing or unreadable: {error}",
                    child.branch
                ),
            });
        }
    };
    if source_tip != receipt.source_sha {
        return Ok(ReceiptFreshness::Stale {
            reason: format!(
                "child branch `{}` moved from {} to {}",
                child.branch, receipt.source_sha, source_tip
            ),
        });
    }

    // Check fingerprint: the current manifest checks must match the ones the
    // evidence was produced under.
    let checks = manifest
        .repos()
        .iter()
        .find(|candidate| candidate.name() == repo)
        .map(|candidate| candidate.checks().to_vec())
        .unwrap_or_default();
    let current_fingerprint = verification::fingerprint(&checks)?;
    if current_fingerprint != receipt.verification.command_fingerprint {
        return Ok(ReceiptFreshness::Stale {
            reason: "the verification checks changed since this receipt was recorded".to_owned(),
        });
    }

    // Result membership: the result must still be in the immediate parent's
    // branch history. A missing revision here is git's own refusal — reported
    // as stale, since a parent branch that lost the result cannot be resumed.
    match git.is_ancestor(&bare, &receipt.result_sha, parent.branch.as_str()) {
        Ok(true) => Ok(ReceiptFreshness::Fresh),
        Ok(false) => Ok(ReceiptFreshness::Stale {
            reason: format!(
                "result {} is no longer in the immediate parent's history ({})",
                receipt.result_sha, parent.branch
            ),
        }),
        Err(error) => Ok(ReceiptFreshness::Stale {
            reason: format!(
                "the parent branch `{}` is missing or unreadable: {error}",
                parent.branch
            ),
        }),
    }
}

/// The failure for an integration blocked by descendants: names every
/// blocker, and points at leaves-first integration.
///
/// Consumed by the integrate and deliver actions (Tasks 11 and 12); until
/// those land it has no caller in the tree.
#[allow(dead_code)]
pub(crate) fn tree_block_failure(feature: &FeatureName, blockers: &[TreeEntry]) -> Failure {
    let names = blockers
        .iter()
        .map(|entry| format!("{} ({})", entry.feature, entry.state))
        .collect::<Vec<_>>();
    Failure::blocked(
        "feature.descendants_block",
        format!(
            "feature `{feature}` cannot integrate while {} descendant{} still block{}",
            blockers.len(),
            if blockers.len() == 1 { "" } else { "s" },
            if blockers.len() == 1 { "s" } else { "" },
        ),
    )
    .expected("every descendant to be integrated, verified, or abandoned")
    .actual(names.join(", "))
    .details(serde_json::json!({ "blockers": blockers }))
    .fix(FixAction::safe(
        "feature.integrate_leaves_first",
        "Integrate the blocking descendants first, leaves first.",
    ))
}

/// The failure for a stale or failed receipt, with the recorded-ref
/// restoration commands (unsafe — they rewrite branches) and the safe
/// new-child route. Never executed automatically.
#[allow(dead_code)]
pub(crate) fn stale_receipt_failure(
    layout: &Layout,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
    receipt: &IntegrationReceipt,
    reason: &str,
) -> Failure {
    let child_worktree = layout.repo_worktree(repo, &child.branch);
    let parent_worktree = layout.repo_worktree(repo, &parent.branch);
    Failure::blocked(
        "feature.receipt_stale",
        format!(
            "the integration receipt for `{repo}` in feature `{}` is stale: {reason}",
            child.name
        ),
    )
    .expected("the receipt to match live state (source tip, checks, result history)")
    .actual(reason)
    .fix(FixAction::unsafe_(
        "feature.restore_source",
        "Restore the child branch to the recorded source, then retry.",
    )
    .command(format!(
        "git -C {child_worktree} reset --hard {}",
        receipt.source_sha
    )))
    .fix(FixAction::unsafe_(
        "feature.restore_result",
        "Restore the parent branch to the recorded result, then retry.",
    )
    .command(format!(
        "git -C {parent_worktree} merge --ff-only {}",
        receipt.result_sha
    )))
    .fix(FixAction::safe(
        "feature.create_new_child",
        "Create a fresh child feature instead of repairing the moved branches.",
    )
    .command(format!(
        "ivar feature create {}-redo --parent {}",
        child.name, parent.name
    )))
}

/// Whether a derived state blocks its parent's integration.
fn blocks(state: &FeatureIntegrationState) -> bool {
    matches!(
        state,
        FeatureIntegrationState::Active
            | FeatureIntegrationState::Failed
            | FeatureIntegrationState::Stale
    )
}

/// The derived state of one feature: the close record wins; without one, the
/// receipt facts classify. A root never has receipts to judge (integration is
/// a child's act), so roots classify as active until closed.
fn state_of(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    feature: &Feature,
    parent_feature: Option<&Feature>,
) -> Result<FeatureIntegrationState, Failure> {
    let outcome = read_close(layout, &feature.name)?.and_then(|record| record.known_outcome());
    let facts = match parent_feature {
        Some(parent) => facts_of(git, layout, manifest, feature, parent)?,
        None => ClassificationFacts::active(),
    };
    Ok(classify(outcome, facts))
}

/// Collect the per-promotion receipt facts of a child against its immediate
/// parent.
fn facts_of(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    child: &Feature,
    parent: &Feature,
) -> Result<ClassificationFacts, Failure> {
    let mut fully_receipted = true;
    let mut any_failed_evidence = false;
    let mut any_stale = false;

    if child.promotions.is_empty() {
        fully_receipted = false;
    }
    for (repo, promotion) in &child.promotions {
        let Some(receipt) = &promotion.integration_receipt else {
            fully_receipted = false;
            continue;
        };
        match receipt_freshness(git, layout, manifest, child, parent, repo, receipt)? {
            ReceiptFreshness::Fresh => {}
            ReceiptFreshness::Failed => any_failed_evidence = true,
            ReceiptFreshness::Stale { .. } => any_stale = true,
        }
    }

    Ok(ClassificationFacts {
        fully_receipted,
        any_failed_evidence,
        any_stale,
    })
}

/// Depth-first, deterministic pre-order walk over `root`'s subtree, filling
/// `entries`. The tree was validated by [`read_all`], so parent references
/// inside the walk always resolve.
fn walk(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    map: &FeatureMap<'_>,
    feature: &Feature,
    depth: usize,
    entries: &mut Vec<TreeEntry>,
) -> Result<(), Failure> {
    let parent_feature = feature
        .parent
        .as_ref()
        .and_then(|name| map.get(name))
        .copied();
    let state = state_of(git, layout, manifest, feature, parent_feature)?;

    let blockers: Vec<String> = blocking_entries(git, layout, manifest, map, feature)?
        .into_iter()
        .map(|entry| format!("{} ({})", entry.feature, entry.state))
        .collect();

    entries.push(TreeEntry {
        feature: feature.name.clone(),
        parent: feature.parent.clone(),
        depth,
        state,
        repos: feature.promotions.keys().cloned().collect(),
        blockers,
    });

    let mut children: Vec<&Feature> = map
        .values()
        .filter(|candidate| candidate.parent.as_ref() == Some(&feature.name))
        .copied()
        .collect();
    children.sort_by(|a, b| a.name.cmp(&b.name));
    for child in children {
        walk(git, layout, manifest, map, child, depth + 1, entries)?;
    }
    Ok(())
}

/// The blocking descendants of `feature`, using the already-read `map` — the
/// shared core of [`blocking_descendants`] and the per-entry `blockers` in
/// [`subtree_status`].
fn blocking_entries(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    map: &FeatureMap<'_>,
    feature: &Feature,
) -> Result<Vec<TreeEntry>, Failure> {
    let mut blockers = Vec::new();
    for (depth, descendant) in descendants_from_values(map, &feature.name) {
        let parent_feature = descendant
            .parent
            .as_ref()
            .and_then(|name| map.get(name))
            .copied();
        let state = state_of(git, layout, manifest, descendant, parent_feature)?;
        if blocks(&state) {
            blockers.push(TreeEntry {
                feature: descendant.name.clone(),
                parent: descendant.parent.clone(),
                depth,
                state,
                repos: descendant.promotions.keys().cloned().collect(),
                blockers: Vec::new(),
            });
        }
    }
    Ok(blockers)
}

/// Validate the whole derived tree: every parent reference resolves, and no
/// parent chain cycles. Runs on every tree read; a corrupt tree is refused as
/// a whole.
fn validate_tree(features: &[Feature]) -> Result<(), Failure> {
    let map = feature_map(features);
    for feature in features {
        let mut seen: HashSet<&FeatureName> = HashSet::new();
        let mut current = Some(&feature.name);
        while let Some(name) = current {
            if !seen.insert(name) {
                return Err(Failure::blocked(
                    "feature.parent_cycle",
                    format!("feature `{}` is part of a parent cycle", feature.name),
                )
                .expected("every parent chain to end at a root (a feature with no parent)")
                .actual(format!("walking parents from `{}` revisited `{name}`", feature.name))
                .fix(FixAction::safe(
                    "feature.fix_parent_reference",
                    "Repair the hand-edited `parent` fields so no feature is its own ancestor.",
                )));
            }
            let Some(parent_feature) = map.get(name) else {
                return Err(Failure::blocked(
                    "feature.parent_missing",
                    format!(
                        "feature `{}` names parent `{name}`, which does not exist",
                        feature.name
                    ),
                )
                .expected("every parent reference to resolve to an existing feature")
                .actual(format!("`{name}` has no feature.json"))
                .fix(FixAction::safe(
                    "feature.fix_parent_reference",
                    format!(
                        "Repair `{}`'s `parent` field, or create the missing `{name}` feature.",
                        feature.name
                    ),
                )));
            };
            current = parent_feature.parent.as_ref();
        }
    }
    Ok(())
}

/// `name` → feature, for the walk helpers.
type FeatureMap<'a> = BTreeMap<&'a FeatureName, &'a Feature>;

fn feature_map(features: &[Feature]) -> FeatureMap<'_> {
    features
        .iter()
        .map(|feature| (&feature.name, feature))
        .collect()
}

/// Every direct/recursive descendant of `name` as `(depth, feature)` pairs in
/// deterministic pre-order (children sorted by name at every level).
fn descendants_from<'a>(all: &'a [Feature], name: &FeatureName) -> Vec<(usize, &'a Feature)> {
    descendants_from_values(&feature_map(all), name)
}

/// The same traversal over an already-built map, so a caller that read the
/// tree once does not read it again.
fn descendants_from_values<'a>(
    map: &FeatureMap<'a>,
    name: &FeatureName,
) -> Vec<(usize, &'a Feature)> {
    let mut result = Vec::new();
    // DFS with an explicit stack: children sorted by name, pushed reversed so
    // the pop order is the sorted pre-order.
    let mut stack: Vec<(usize, &'a Feature)> = map
        .values()
        .filter(|feature| feature.parent.as_ref() == Some(name))
        .copied()
        .map(|feature| (1, feature))
        .collect();
    stack.sort_by(|(_, a), (_, b)| b.name.cmp(&a.name));
    while let Some((depth, feature)) = stack.pop() {
        result.push((depth, feature));
        let mut children: Vec<(usize, &'a Feature)> = map
            .values()
            .filter(|candidate| candidate.parent.as_ref() == Some(&feature.name))
            .copied()
            .map(|candidate| (depth + 1, candidate))
            .collect();
        children.sort_by(|(_, a), (_, b)| b.name.cmp(&a.name));
        stack.extend(children);
    }
    result
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/relations.rs"]
mod tests;
