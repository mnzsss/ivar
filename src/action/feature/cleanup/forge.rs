use super::preview::{absent_manifest_facts, revision};
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn collect_repo_facts(
    git: &impl Git,
    layout: &crate::store::layout::Layout,
    manifest: &crate::store::manifest::Manifest,
    feature: &Feature,
    repo: &RepoName,
    promotion: &crate::domain::feature::Promotion,
    worktree_lookup: &Result<Option<WorktreeEntry>, Failure>,
    consult_forge: bool,
    merged: &std::cell::RefCell<Vec<(RepoName, PullRequest)>>,
) -> CleanupRepoFacts {
    let Some(manifest_repo) = manifest
        .repos()
        .iter()
        .find(|candidate| candidate.name() == repo)
    else {
        return absent_manifest_facts(repo);
    };
    let effective_base = base::resolve(feature, promotion, manifest_repo.default_branch());
    let bare = layout.repo_bare(repo);
    let clone_exists = matches!(git.target_state(&bare), Ok(TargetState::Repository));
    let mut inspection_error = None;
    let (feature_head, base_head, local_branch_exists, unmerged_commits, forge_delivery) =
        if clone_exists {
            let feature_head = revision(git, &bare, feature.branch.as_str(), &mut inspection_error);
            let base_head = revision(git, &bare, effective_base.as_str(), &mut inspection_error);
            let local_branch_exists = feature_head.is_some();
            let unmerged_commits = match base::unmerged_commits_with_integration(
                git,
                layout,
                manifest,
                feature,
                repo,
                &effective_base,
            ) {
                Ok(commits) => Some(commits),
                Err(error) => {
                    inspection_error.get_or_insert_with(|| error.to_string());
                    None
                }
            };
            let forge_delivery = if consult_forge {
                match (unmerged_commits, feature_head.as_deref()) {
                    (Some(1..), Some(head)) => {
                        let (verdict, merged_pr) = ask_forge(&bare, feature.branch.as_str(), head);
                        if let Some(pr) = merged_pr {
                            merged.borrow_mut().push((repo.clone(), pr));
                        }
                        Some(verdict)
                    }
                    _ => None,
                }
            } else {
                None
            };
            (
                feature_head,
                base_head,
                local_branch_exists,
                unmerged_commits,
                forge_delivery,
            )
        } else {
            (None, None, false, None, None)
        };
    let worktree = match worktree_lookup {
        Ok(entry) => entry.as_ref().filter(|entry| !entry.prunable),
        Err(failure) => {
            inspection_error.get_or_insert_with(|| failure.what.clone());
            None
        }
    };
    let worktree_exists = worktree.is_some();
    let dirty_worktree = if let Some(worktree) = worktree {
        match git.worktree_dirty(&worktree.path) {
            Ok(dirty) => Some(dirty),
            Err(error) => {
                inspection_error.get_or_insert_with(|| error.to_string());
                None
            }
        }
    } else {
        None
    };

    CleanupRepoFacts {
        repo: repo.clone(),
        effective_base: Some(effective_base),
        feature_head,
        base_head,
        local_branch_exists,
        worktree_exists,
        clone_exists,
        dirty_worktree,
        unmerged_commits,
        in_manifest: true,
        inspection_error,
        forge_delivery,
    }
}

/// Ask the forge about `branch`, and read its answer as a delivery verdict,
/// with the pull request merged at `feature_head` when there is one.
pub(super) fn ask_forge(
    bare: &Utf8Path,
    branch: &str,
    feature_head: &str,
) -> (ForgeDelivery, Option<PullRequest>) {
    match pull_requests::list_pull_requests(bare, branch, "all") {
        Ok(prs) => (
            read_forge_answer(&prs, feature_head),
            merged_pull_request(&prs, feature_head).cloned(),
        ),
        Err(failure) => (
            ForgeDelivery::Unavailable {
                reason: failure.what,
            },
            None,
        ),
    }
}

pub(super) fn merged_pull_request<'a>(
    prs: &'a [PullRequest],
    feature_head: &str,
) -> Option<&'a PullRequest> {
    prs.iter().find(|pr| pr.merged_head(feature_head))
}

/// The verdict the forge's pull requests support for `feature_head`. A merged
/// pull request for this exact head wins over every earlier record for the
/// branch.
pub(super) fn read_forge_answer(prs: &[PullRequest], feature_head: &str) -> ForgeDelivery {
    if prs.iter().any(|pr| pr.merged_head(feature_head)) {
        return ForgeDelivery::Merged;
    }
    let Some(pr) = prs.iter().find(|pr| pr.is_merged()).or(prs.first()) else {
        return ForgeDelivery::NoPullRequest;
    };
    if pr.is_merged() {
        ForgeDelivery::MergedOtherHead { number: pr.number }
    } else {
        ForgeDelivery::NotMerged {
            number: pr.number,
            state: pr.state.clone(),
        }
    }
}
