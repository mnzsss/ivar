//! The effective base of one promoted repo, for every verb that reads it
//! back: `status`, `rebase`, `prune`, `deliver`. One place so they cannot
//! disagree with what `promote` recorded.

use std::collections::HashSet;

use camino::Utf8Path;

use crate::domain::feature::{Feature, Promotion, effective_base};
use crate::domain::name::{BranchName, RepoName};
use crate::git::{Error, Git};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

/// `promotion`'s recorded base, or — for a promotion recorded before this
/// field existed — [`effective_base`] of the feature's declared base against
/// `default_branch`, the same fallback `promote` itself resolves.
pub(crate) fn resolve(
    feature: &Feature,
    promotion: &Promotion,
    default_branch: &BranchName,
) -> BranchName {
    promotion
        .base
        .clone()
        .unwrap_or_else(|| effective_base(feature.base.as_ref(), default_branch))
}

/// How many commits `branch` carries beyond `base`, or zero when those
/// changes already landed on `base` under new identities — squash-merged as
/// one commit, or rebased/cherry-picked commit by commit.
///
/// Anything git cannot answer while proving the landing leaves the plain
/// count standing: a feature is never called delivered on a guess.
pub(crate) fn unmerged_commits(
    git: &impl Git,
    bare: &Utf8Path,
    base: &str,
    branch: &str,
) -> Result<u64, Error> {
    let ahead = git.commits_ahead(bare, base, branch)?;
    if ahead == 0 || landed_under_new_identity(git, bare, base, branch).unwrap_or(false) {
        return Ok(0);
    }
    Ok(ahead)
}

/// How many commits `feature` still carries beyond `base` for `repo`, once
/// its integration into its parent is taken into account.
///
/// A child integrated into its parent never has its own commits on the
/// parent's branch (a squash writes a new commit, and a later rebase of the
/// parent rewrites that one too), so [`unmerged_commits`] alone calls it
/// undelivered for good. The integration receipt is the proof instead: when
/// the child branch is still at the receipt's source, its change landed on
/// the parent's branch (`result_sha` is still in that history), or — once the
/// parent was rewritten — the parent's own content is delivered to its base.
///
/// Anything that cannot be proven leaves the plain count standing: a feature
/// is never called delivered on a guess.
///
/// # Errors
///
/// Returns [`Error`] if git cannot count the commits ahead.
pub(crate) fn unmerged_commits_with_integration(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    feature: &Feature,
    repo: &RepoName,
    base: &BranchName,
) -> Result<u64, Error> {
    unmerged_commits_inner(git, layout, manifest, feature, repo, base, 0)
}

/// The deepest parent chain followed to prove a delivery. A real hall is a
/// handful of levels deep; the cap only stops a hand-edited cycle.
const MAX_PARENT_HOPS: usize = 32;

fn unmerged_commits_inner(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    feature: &Feature,
    repo: &RepoName,
    base: &BranchName,
    hops: usize,
) -> Result<u64, Error> {
    let bare = layout.repo_bare(repo);
    let ahead = unmerged_commits(git, &bare, base.as_str(), feature.branch.as_str())?;
    if ahead > 0 && integration_landed(git, layout, manifest, feature, repo, base, hops) {
        return Ok(0);
    }
    Ok(ahead)
}

/// Whether `feature`'s receipt for `repo` proves its change shipped with its
/// parent. See [`unmerged_commits_with_integration`].
fn integration_landed(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    feature: &Feature,
    repo: &RepoName,
    base: &BranchName,
    hops: usize,
) -> bool {
    let Some(receipt) = feature
        .promotions
        .get(repo)
        .and_then(|promotion| promotion.integration_receipt.as_ref())
    else {
        return false;
    };
    let Some(parent_name) = &feature.parent else {
        return false;
    };
    // The receipt describes this branch only while it still sits at the
    // integrated source, and only for the parent the child is based on.
    if !receipt.verification.passed() || receipt.target_branch != *base {
        return false;
    }
    let bare = layout.repo_bare(repo);
    match git.revision_commit(&bare, feature.branch.as_str()) {
        Ok(tip) if tip == receipt.source_sha => {}
        _ => return false,
    }
    // The parent still carries the result: the child is landed on its base.
    if git
        .is_ancestor(&bare, &receipt.result_sha, base.as_str())
        .unwrap_or(false)
    {
        return true;
    }
    // The parent was rewritten (a rebase), so the result commit left its
    // history. The child's content then shipped with the parent's.
    if hops >= MAX_PARENT_HOPS {
        return false;
    }
    let Ok(Some(parent)) = Feature::read(layout, parent_name) else {
        return false;
    };
    let Some(parent_promotion) = parent.promotions.get(repo) else {
        return false;
    };
    let Some(manifest_repo) = manifest
        .repos()
        .iter()
        .find(|candidate| candidate.name() == repo)
    else {
        return false;
    };
    let parent_base = resolve(&parent, parent_promotion, manifest_repo.default_branch());
    matches!(
        unmerged_commits_inner(git, layout, manifest, &parent, repo, &parent_base, hops + 1),
        Ok(0)
    )
}

fn landed_under_new_identity(
    git: &impl Git,
    bare: &Utf8Path,
    base: &str,
    branch: &str,
) -> Result<bool, Error> {
    let divergence = git.divergence(bare, branch, base)?;
    let base_patch_ids = divergence
        .remote_only
        .iter()
        .map(|commit| git.commit_patch_id(bare, &commit.sha))
        .collect::<Result<HashSet<String>, Error>>()?;

    let merge_base = git.merge_base(bare, base, branch)?;
    if base_patch_ids.contains(&git.diff_patch_id(bare, &merge_base, branch)?) {
        return Ok(true);
    }

    for commit in &divergence.local_only {
        if !base_patch_ids.contains(&git.commit_patch_id(bare, &commit.sha)?) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/base.rs"]
mod tests;
