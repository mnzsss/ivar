//! The effective base of one promoted repo, for every verb that reads it
//! back: `status`, `rebase`, `prune`, `deliver`. One place so they cannot
//! disagree with what `promote` recorded.

use std::collections::HashSet;

use camino::Utf8Path;

use crate::domain::feature::{Feature, Promotion, effective_base};
use crate::domain::name::BranchName;
use crate::git::{Error, Git};

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
